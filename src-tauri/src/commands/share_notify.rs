//! Share-notification poller for HQ Sync (US-004).
//!
//! Polls `/v1/files/shared-with-me` on app launch (5s delay, matching the
//! updater pattern) and after every `sync:all-complete` event. Emits a
//! `share:new-events` Tauri event to the Svelte renderer when new events
//! are found, which US-005 will consume to fire macOS notifications and open
//! the ShareDetail window.
//!
//! ## Feature gating
//!
//! A single check gates a poll: the **`shareNotifications`** preference in
//! `~/.hq/menubar.json` (re-read on every poll cycle so the Settings toggle
//! takes effect immediately after the next sync without an app restart;
//! defaults ON when absent or unreadable).
//!
//! The former `@getindigo.ai` dogfood gate was removed 2026-05-26 — see
//! `should_poll` for the full rationale (it silently suppressed notifications
//! for recipients signed in under a non-getindigo Cognito identity).
//!
//! ## Cursor
//!
//! `~/.hq/share-notify-cursor.json` is a JSON object keyed by `machineId`
//! (from `ensure_machine_id`) so multiple Macs for the same user each track
//! their own polling position independently. The value is an ISO8601 string
//! passed as `since=` in the query. Absent → no `since=` (server returns all
//! events up to the default limit).
//!
//! ## Singleton lock
//!
//! A `std::sync::Mutex<bool>` (`POLL_IN_FLIGHT`) prevents concurrent polls.
//! The guard is dropped before the async HTTP call so it is never held across
//! an `.await` point (MutexGuard is !Send).
//!
//! ## Log codes
//!
//! All log lines to `~/.hq/logs/hq-sync.log` carry a `share-notify` tag and
//! one of these code prefixes so support can grep for them:
//!   `SHARE_NOTIFY_POLL_SKIP`          — gate/preference disabled or in-flight
//!   `SHARE_NOTIFY_POLL_START`         — poll request about to fire
//!   `SHARE_NOTIFY_POLL_OK`            — poll succeeded (may have 0 events)
//!   `SHARE_NOTIFY_POLL_AUTH_FAIL`     — 401/403 or token resolution failure
//!   `SHARE_NOTIFY_POLL_NETWORK_FAIL`  — reqwest transport error
//!   `SHARE_NOTIFY_POLL_ERROR`         — 4xx/5xx other than auth, or parse fail

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::logfile::log;
use crate::util::paths;

// ── Blocking-notification concurrency cap ─────────────────────────────────────
//
// `mac-notification-sys` 0.6.12 busy-spins a Cocoa run loop inside
// `Notification::send()` when `wait_for_click(true)` is set: its
// `NotificationCenterDelegate keepRunning` loop calls
// `[[NSRunLoop currentRunLoop] runUntilDate:…]` on a run loop with no attached
// input source, so `runUntilDate:` returns immediately and the loop spins a
// full core. `keepRunning` only flips false when the user clicks/dismisses, so
// every un-actioned interactive notification leaks one spinning thread forever
// (measured: 8 leaked threads ≈ 673% CPU). See
// `workspace/reports/hq-sync-cpu-spin-debug.md`.
//
// Mitigation (CPU fix, Option 1): cap *blocking* sends to at most one at a
// time, shared across BOTH the share and DM notification surfaces. The first
// caller to acquire the slot sends with `wait_for_click(true)` (interactive);
// any concurrent caller falls back to a fire-and-forget `.send()` (no spin).
// This bounds the busy-spin at ~1 core instead of growing without limit, and
// preserves interactivity for the in-flight notification. The proper long-term
// fix (drop blocking sends entirely / move to a non-spinning action surface) is
// tracked separately.
static BLOCKING_NOTIFY_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// RAII guard for the single "blocking notification send" slot, shared process-
/// wide across `share_notify` and `dm_notify`. Acquire with
/// [`BlockingNotifyGuard::try_acquire`]; the slot is released on `Drop`.
pub(crate) struct BlockingNotifyGuard;

impl BlockingNotifyGuard {
    /// Try to claim the single blocking-send slot. Returns `Some(guard)` if the
    /// slot was free (now claimed); `None` if another blocking send is already
    /// in flight — the caller should then fire-and-forget instead.
    pub(crate) fn try_acquire() -> Option<Self> {
        BLOCKING_NOTIFY_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then_some(BlockingNotifyGuard)
    }
}

impl Drop for BlockingNotifyGuard {
    fn drop(&mut self) {
        BLOCKING_NOTIFY_IN_FLIGHT.store(false, Ordering::Release);
    }
}

// ── Notification action wiring ────────────────────────────────────────────────
//
// We DELIBERATELY bypass tauri-plugin-notification for the share-event
// notification surface and use mac-notification-sys directly. Rationale:
//
//   * tauri-plugin-notification (2.3.3) on macOS routes through notify-rust,
//     which DOES NOT support action buttons on desktop. `register_action_types`
//     is mobile-only and there's no `on_action` callback for desktop.
//   * mac-notification-sys (already a transitive dep via notify-rust) exposes
//     `MainButton::DropdownActions(title, &[...])` + `wait_for_click(true)`
//     which on modern macOS (Sonoma/Sequoia) reveal action buttons on hover.
//   * Same pattern is used in commands/meetings.rs (proven 2026-05-25 dogfood).
//
// On user action (Copy / Open / body click), the spawned thread emits a
// `notification:share-action` Tauri event to the frontend, which:
//   * "copy"  → writes the templated prompt to clipboard via navigator.clipboard
//   * "open"  → invokes the `open_share_detail` command
//
// Side effect: pinning to mac-notification-sys means this notification surface
// is macOS-only. The poller code is also macOS-only in spirit (Tauri menubar
// app), so this is consistent with the broader app target.

/// Tauri event channel name for `NotificationShareActionEvent`.
const EVENT_NOTIFICATION_SHARE_ACTION: &str = "notification:share-action";

/// Action dispatched by the frontend listener when the user interacts with a
/// share-notification banner. `event_id` lets the frontend look up the full
/// share event from its in-memory pending list (primed by `share:new-events`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationShareActionEvent {
    /// One of: `"copy"`, `"open"`. Any other action is filtered out before emit.
    action: String,
    /// Event ID of the share this notification represents — lets the
    /// frontend route to the right SHARE_EVENT row when multiple
    /// notifications are stacked.
    event_id: String,
    /// Full event payload embedded for offline-from-server convenience —
    /// the frontend can render Copy prompt without round-tripping
    /// `/v1/files/shared-with-me` again.
    event: ShareEvent,
}

// ── Event name emitted to the Svelte renderer ────────────────────────────────

/// Tauri event emitted when new share events are found. US-005 listens for
/// this to fire macOS notifications and open the ShareDetail window.
pub const EVENT_SHARE_NEW_EVENTS: &str = "share:new-events";

// ── Singleton in-flight guard ─────────────────────────────────────────────────

const LOG_TAG: &str = "share-notify";

static POLL_IN_FLIGHT: OnceLock<Mutex<bool>> = OnceLock::new();

fn poll_lock() -> &'static Mutex<bool> {
    POLL_IN_FLIGHT.get_or_init(|| Mutex::new(false))
}

/// Atomically mark a poll as in-flight. Returns `true` if we successfully
/// took the lock (caller must `clear_in_flight()` when done), `false` if
/// another poll is already running.
fn try_set_in_flight() -> bool {
    let mut guard = poll_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if *guard {
        return false;
    }
    *guard = true;
    true
    // guard dropped here — not held across any await point
}

fn clear_in_flight() {
    let mut guard = poll_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    *guard = false;
}

// ── Wire-format types ─────────────────────────────────────────────────────────

/// A single share event as returned by `GET /v1/files/shared-with-me`.
/// Fields mirror the US-003 response schema; `note` is omitted when absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareEvent {
    pub event_id: String,
    pub issuer_email: String,
    pub issuer_display_name: String,
    pub paths: Vec<String>,
    pub note: Option<String>,
    pub permission: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedWithMeResponse {
    events: Vec<ShareEvent>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

/// Tauri event emitted to the share-detail window once its listener is ready
/// (ready-handshake; mirrors "new-files:list" in new_files.rs).
const EVENT_SHARE_EVENTS_LIST: &str = "share:events-list";

/// Managed state: pending share events for the detail window ready-handshake.
/// Follows the `PendingNewFiles` pattern in new_files.rs exactly.
pub struct PendingShareEvents(pub Mutex<Vec<ShareEvent>>);

// ── Cursor persistence ────────────────────────────────────────────────────────

/// Upper bound on the per-machine `notified` ring. The repeated boundary events
/// (the cause of the re-notify bug) are always the newest, so they never reach
/// the eviction end of the FIFO — 200 is comfortably more than any single
/// `?since=` page (`limit=50`).
const NOTIFIED_CAP: usize = 200;

/// Per-machine cursor state.
///
/// `cursor` is the ISO8601 `createdAt` of the newest event seen so far (the
/// `?since=` value). `notified` is a bounded FIFO of recently-notified
/// `eventId`s: the `shared-with-me` endpoint treats `?since=` as **inclusive**,
/// so the boundary event(s) are returned on every subsequent poll. Without an
/// id-level guard that re-delivers the same banner on every poll/launch (the
/// 2026-05-29 "same 8 events, cursor stuck" symptom). Deduping by id makes
/// re-notification impossible regardless of the server's `since` semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CursorEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    #[serde(default)]
    notified: Vec<String>,
}

/// Back-compat shim: pre-0.4.4 stored a bare ISO string per machine. Accept both
/// the new object form and the legacy string form on read so an upgrade doesn't
/// re-notify every historical share once.
#[derive(Deserialize)]
#[serde(untagged)]
enum CursorEntryCompat {
    Entry(CursorEntry),
    Legacy(String),
}

impl From<CursorEntryCompat> for CursorEntry {
    fn from(c: CursorEntryCompat) -> Self {
        match c {
            CursorEntryCompat::Entry(e) => e,
            CursorEntryCompat::Legacy(s) => CursorEntry {
                cursor: Some(s),
                notified: Vec::new(),
            },
        }
    }
}

type CursorStore = HashMap<String, CursorEntry>;

fn cursor_path() -> Result<std::path::PathBuf, String> {
    paths::hq_config_dir().map(|d| d.join("share-notify-cursor.json"))
}

/// Read the whole store, normalising any legacy bare-string entries to the
/// current object shape.
fn read_cursor_store() -> CursorStore {
    let Ok(path) = cursor_path() else {
        return CursorStore::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return CursorStore::default();
    };
    match serde_json::from_str::<HashMap<String, CursorEntryCompat>>(&contents) {
        Ok(store) => store.into_iter().map(|(k, v)| (k, v.into())).collect(),
        Err(_) => CursorStore::default(),
    }
}

fn read_cursor_entry(machine_id: &str) -> CursorEntry {
    read_cursor_store().remove(machine_id).unwrap_or_default()
}

fn write_cursor_entry(machine_id: &str, entry: &CursorEntry) {
    let Ok(path) = cursor_path() else { return };
    // Re-read (with normalisation) so we never clobber other machines' entries.
    let mut store = read_cursor_store();
    store.insert(machine_id.to_string(), entry.clone());
    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(&path, json);
    }
}

/// Split a poll's events into the subset to notify (dropping any whose `eventId`
/// is already in `notified`, preserving order) and the updated `notified` ring
/// (bounded to [`NOTIFIED_CAP`], newest at the end). Pure so it is unit-testable
/// without the filesystem or network.
fn partition_unnotified(
    events: &[ShareEvent],
    notified: &[String],
) -> (Vec<ShareEvent>, Vec<String>) {
    let seen: std::collections::HashSet<&str> =
        notified.iter().map(String::as_str).collect();
    let fresh: Vec<ShareEvent> = events
        .iter()
        .filter(|e| !seen.contains(e.event_id.as_str()))
        .cloned()
        .collect();

    let mut updated = notified.to_vec();
    updated.extend(fresh.iter().map(|e| e.event_id.clone()));
    if updated.len() > NOTIFIED_CAP {
        updated.drain(0..updated.len() - NOTIFIED_CAP);
    }
    (fresh, updated)
}

// ── Gate check ───────────────────────────────────────────────────────────────

/// Returns true when the user preference allows polling. Re-reads menubar.json
/// on every call so toggling the setting takes effect on the next poll without
/// an app restart.
///
/// NOTE: the former `@getindigo.ai` dogfood gate (`feature_gate::is_indigo_user`)
/// was removed 2026-05-26 once the share-notify feature was proven in production.
/// Gating the poller to indigo emails silently suppressed macOS notifications for
/// any recipient whose HQ Sync was signed in under a non-`@getindigo.ai` Cognito
/// identity: the grant resolves to the recipient's canonical personUid and the
/// `SHARE_EVENT` row + email fallback are written correctly, but the poller never
/// ran, so the events sat unacked forever and no notification fired. The
/// `shareNotifications` pref is now the only gate. (Other features — meetings,
/// staging-drift, release-channel — keep their own independent indigo gates.)
async fn should_poll() -> bool {
    let pref = match crate::commands::settings::get_settings().await {
        Ok(prefs) => prefs.share_notifications,
        Err(_) => None, // settings unreadable → fall through to default
    };
    share_notifications_enabled(pref)
}

/// Pure gating decision for the share-notify poller. Notifications are ON unless
/// the user explicitly turned the `shareNotifications` pref off. A missing pref
/// (`None`) or unreadable settings default to ON.
pub fn share_notifications_enabled(share_notifications: Option<bool>) -> bool {
    share_notifications.unwrap_or(true)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Interval between independent share-notify polls once the launch poll has
/// run. Delivery MUST NOT depend on `sync:all-complete` firing — see the
/// 2026-05-28 incident (`workspace/reports/hq-sync-notifications-debug.md`):
/// the sync daemon was down ~34h, so the poller never ran and 7 incoming
/// shares sat unacked, then drained in a single cursor jump (≤1 banner for 7
/// events). The post-sync poll in `main.rs` is now a latency optimization on
/// top of this timer, not the sole delivery mechanism.
const SHARE_POLL_INTERVAL_SECS: u64 = 60;

/// Spawn the share-notify poller. Called from `main.rs` setup. Runs a launch
/// poll after a 5-second delay (matches the updater pattern — gives the app
/// time to fully initialize and the Cognito token to be loaded from disk),
/// then polls on an independent `SHARE_POLL_INTERVAL_SECS` timer so delivery
/// is decoupled from sync completion. `poll_once` is singleton-guarded
/// (`POLL_IN_FLIGHT`) so this composes safely with the post-sync poll.
pub fn setup_share_notify_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        // Launch poll: shares + DM inbox (one timer, two fetches — the DM
        // channel rides this same independent timer so it can never inherit
        // the sync-coupling flaw that broke share notifications).
        poll_once(app.clone()).await;
        crate::commands::dm_notify::poll_dm_once(app.clone()).await;

        let mut ticker =
            tokio::time::interval(tokio::time::Duration::from_secs(SHARE_POLL_INTERVAL_SECS));
        // The first tick fires immediately; consume it so the launch poll
        // above isn't double-counted, then poll once per interval thereafter.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            poll_once(app.clone()).await;
            crate::commands::dm_notify::poll_dm_once(app.clone()).await;
        }
    });
}

/// Fire one poll cycle. Skips if another poll is already in flight (singleton
/// guard). Safe to call from the `sync:all-complete` listener or the Tauri
/// command handler — the guard prevents overlap.
pub async fn poll_once(app: AppHandle) {
    if !try_set_in_flight() {
        log(LOG_TAG, "SHARE_NOTIFY_POLL_SKIP poll already in-flight");
        return;
    }
    do_poll(&app).await;
    clear_in_flight();
}

/// Tauri command: manual poll trigger. Exposed so the frontend (and tests)
/// can force a poll without waiting for a sync event.
#[tauri::command]
pub async fn poll_shared_with_me(app: AppHandle) -> Result<(), String> {
    poll_once(app).await;
    Ok(())
}

// ── Core poll logic ───────────────────────────────────────────────────────────

async fn do_poll(app: &AppHandle) {
    if !should_poll().await {
        log(LOG_TAG, "SHARE_NOTIFY_POLL_SKIP feature gate or setting disabled");
        return;
    }

    // Cursor key: machineId so each Mac tracks independently.
    let machine_id = match crate::commands::config::ensure_machine_id() {
        Ok(id) => id,
        Err(e) => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_POLL_ERROR cannot resolve machineId: {e}"),
            );
            return;
        }
    };

    // Resolved + refreshed access token (transparent Cognito refresh).
    let access_token = match cognito::get_valid_access_token().await {
        Ok(t) => t,
        Err(e) => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_POLL_AUTH_FAIL token error: {e}"),
            );
            return;
        }
    };

    let base_url = match resolve_vault_api_url() {
        Ok(u) => u.trim_end_matches('/').to_string(),
        Err(e) => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_POLL_ERROR cannot resolve vault URL: {e}"),
            );
            return;
        }
    };

    let entry = read_cursor_entry(&machine_id);
    let since = entry.cursor.clone();
    let url = match since.as_deref() {
        Some(s) => format!(
            "{}/v1/files/shared-with-me?since={}&limit=50",
            base_url, s
        ),
        None => format!("{}/v1/files/shared-with-me?limit=50", base_url),
    };

    log(
        LOG_TAG,
        &format!("SHARE_NOTIFY_POLL_START since={:?}", since),
    );

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await;

    match resp {
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_POLL_NETWORK_FAIL {e}"));
        }
        Ok(r) => {
            let status = r.status();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                log(
                    LOG_TAG,
                    &format!("SHARE_NOTIFY_POLL_AUTH_FAIL status={status}"),
                );
                return;
            }
            if !status.is_success() {
                log(
                    LOG_TAG,
                    &format!("SHARE_NOTIFY_POLL_ERROR status={status}"),
                );
                return;
            }
            match r.json::<SharedWithMeResponse>().await {
                Err(e) => {
                    log(
                        LOG_TAG,
                        &format!("SHARE_NOTIFY_POLL_ERROR parse failed: {e}"),
                    );
                }
                Ok(body) => {
                    if body.events.is_empty() {
                        log(LOG_TAG, "SHARE_NOTIFY_POLL_OK no new events");
                        return;
                    }

                    // Dedupe by eventId BEFORE notifying: the endpoint's `?since=`
                    // is inclusive, so the boundary event(s) come back every poll.
                    // `fresh` is the never-notified subset; `notified` is the
                    // updated ring we persist below.
                    let (fresh, notified) =
                        partition_unnotified(&body.events, &entry.notified);

                    // Advance the timestamp cursor to the newest event seen this
                    // poll (across ALL returned events, not just fresh) so the
                    // `?since=` window keeps narrowing over time.
                    let newest = body
                        .events
                        .iter()
                        .map(|e| e.created_at.as_str())
                        .max()
                        .unwrap_or_default();
                    write_cursor_entry(
                        &machine_id,
                        &CursorEntry {
                            cursor: (!newest.is_empty())
                                .then(|| newest.to_string())
                                .or(entry.cursor.clone()),
                            notified,
                        },
                    );

                    if fresh.is_empty() {
                        log(
                            LOG_TAG,
                            &format!(
                                "SHARE_NOTIFY_POLL_OK no new events ({} already notified)",
                                body.events.len()
                            ),
                        );
                        return;
                    }

                    log(
                        LOG_TAG,
                        &format!(
                            "SHARE_NOTIFY_POLL_OK {} event(s) ({} new), cursor→{}",
                            body.events.len(),
                            fresh.len(),
                            newest
                        ),
                    );

                    // Lazily register the bundle identifier with mac-notification-sys
                    // on the first send per process. Without this, the library calls
                    // `get_bundle_identifier_or_default("use_default")` internally,
                    // which triggers a macOS "Choose Application" picker because
                    // Launch Services can't resolve the literal "use_default" to an
                    // installed app. Mirrors the fix in commands/meetings.rs.
                    //
                    // `set_application` itself is guarded by an internal Once, so
                    // calling it on every send would be safe — wrapping in our own
                    // OnceLock keeps the log line at one-per-process.
                    static NOTIFICATION_APP_INIT: OnceLock<()> = OnceLock::new();
                    NOTIFICATION_APP_INIT.get_or_init(|| {
                        const BUNDLE_ID: &str = "ai.indigo.hq-sync-menubar";
                        match mac_notification_sys::set_application(BUNDLE_ID) {
                            Ok(()) => log(
                                LOG_TAG,
                                &format!("SHARE_NOTIFY_BUNDLE_SET bundle={BUNDLE_ID}"),
                            ),
                            Err(e) => log(
                                LOG_TAG,
                                &format!("SHARE_NOTIFY_BUNDLE_SET_FAILED bundle={BUNDLE_ID} err={e}"),
                            ),
                        }
                    });

                    // Fire one macOS notification per share event (US-005).
                    //
                    // We use mac-notification-sys directly (NOT tauri-plugin-
                    // notification) so we can attach a `DropdownActions` button
                    // labelled "Actions" with two options ("Copy prompt", "Open
                    // details") that reveal on hover. The spawned thread blocks
                    // on `wait_for_click(true).send()` until the user interacts
                    // (or macOS auto-dismisses) and emits a Tauri event for the
                    // frontend listener to handle.
                    //
                    // Custom-banner path: when `customBanner` is enabled, route
                    // each share through the in-app banner (event-driven, no
                    // busy-spinning Cocoa run loop) and skip the native firing
                    // loop below entirely. The tray badge + `share:new-events`
                    // emit after the branch run for both paths.
                    if crate::commands::banner::custom_banner_enabled() {
                        log(
                            LOG_TAG,
                            &format!("SHARE_NOTIFY_CUSTOM_BANNER {} event(s)", fresh.len()),
                        );
                        for evt in &fresh {
                            if let Err(e) = crate::commands::banner::show_share_banner(
                                app.clone(),
                                evt.clone(),
                            )
                            .await
                            {
                                log(LOG_TAG, &format!("SHARE_NOTIFY_BANNER_FAIL err={e}"));
                            }
                        }
                    } else {
                    for evt in &fresh {
                        let body_text = notification_body(evt.note.as_deref(), &evt.paths);
                        let title = notification_title(&evt.issuer_display_name);
                        let app_for_thread = app.clone();
                        let event_clone = evt.clone();

                        std::thread::spawn(move || {
                            let mut notification = mac_notification_sys::Notification::default();
                            // The dropdown title appears as the visible button
                            // label; the slice elements are the dropdown items.
                            // Order = display order.
                            notification
                                .title(&title)
                                .message(&body_text)
                                // Body-click = primary action (open Claude Code
                                // with the templated prompt prefilled, see the
                                // Body-click = primary action: open Claude
                                // Code with the templated prompt prefilled
                                // (see "claude" branch in App.svelte).
                                // Dropdown surfaces two explicit alternatives:
                                //   * Copy prompt   — clipboard only (no app
                                //     open) for users who already have a
                                //     session running or want to paste
                                //     elsewhere.
                                //   * Open details  — ShareDetail window with
                                //     full path list + Open in HQ Console.
                                // Copy is intentionally redundant w/ body-
                                // click for the LLM-session case; explicit
                                // discoverability beats minimalism here
                                // (user direction 2026-05-26).
                                .main_button(mac_notification_sys::MainButton::DropdownActions(
                                    "Actions",
                                    &["Copy prompt", "Open details"],
                                ));

                            // CPU cap (Option 1): only the holder of the single
                            // blocking-send slot may use `wait_for_click(true)`
                            // (which busy-spins until the user acts — see
                            // BlockingNotifyGuard docs). Any concurrent send
                            // falls back to fire-and-forget so we never
                            // accumulate spinning threads. The guard is dropped
                            // the instant the blocking send returns.
                            let response = match BlockingNotifyGuard::try_acquire() {
                                Some(guard) => {
                                    let r = notification.wait_for_click(true).send();
                                    drop(guard);
                                    r
                                }
                                None => notification.send(),
                            };

                            match response {
                                Ok(resp) => {
                                    // Body-click → "claude": opens Claude
                                    // Code (`claude://code/new?q=…&folder=…`)
                                    // with the templated prompt pre-filled
                                    // and cwd at the user's HQ folder. The
                                    // recipient lands in an LLM session
                                    // ready to act on the shared files
                                    // without a paste step.
                                    //
                                    // Dropdown "Open details" → open the
                                    // ShareDetail window for a UI surface
                                    // (path list + Copy prompt fallback +
                                    // Open in HQ Console link).
                                    //
                                    // The frontend listener in App.svelte
                                    // owns the URL build + Tauri-command
                                    // dispatch.
                                    let action: Option<&'static str> = match resp {
                                        mac_notification_sys::NotificationResponse::ActionButton(name)
                                            if name.eq_ignore_ascii_case("copy prompt") =>
                                        {
                                            Some("copy")
                                        }
                                        mac_notification_sys::NotificationResponse::ActionButton(name)
                                            if name.eq_ignore_ascii_case("open details") =>
                                        {
                                            Some("open")
                                        }
                                        mac_notification_sys::NotificationResponse::Click => Some("claude"),
                                        // CloseButton / Reply / None — no actionable signal.
                                        _ => None,
                                    };

                                    if let Some(action) = action {
                                        let payload = NotificationShareActionEvent {
                                            action: action.to_string(),
                                            event_id: event_clone.event_id.clone(),
                                            event: event_clone,
                                        };
                                        if let Err(e) = app_for_thread
                                            .emit(EVENT_NOTIFICATION_SHARE_ACTION, &payload)
                                        {
                                            log(
                                                LOG_TAG,
                                                &format!(
                                                    "SHARE_NOTIFY_EMIT_ACTION_FAILED action={action} err={e}"
                                                ),
                                            );
                                        }
                                    }
                                }
                                Err(e) => log(
                                    LOG_TAG,
                                    &format!("SHARE_NOTIFY_SEND_FAILED err={e}"),
                                ),
                            }
                        });
                    }
                    } // end else — native firing path

                    // Badge the tray icon with the count of newly-notified events.
                    crate::tray::set_share_badge(app, fresh.len());

                    // Emit to frontend — US-005 listens here (currently no-op
                    // after the eager-open removal, kept for future popover UI).
                    let _ = app.emit(EVENT_SHARE_NEW_EVENTS, &fresh);
                }
            }
        }
    }
}

// ── Notification content helpers ──────────────────────────────────────────────

/// Build the macOS notification title for a share event.
/// Format: "<issuerDisplayName> shared files with you"
pub(crate) fn notification_title(issuer_display_name: &str) -> String {
    format!("{} shared files with you", issuer_display_name)
}

/// Build the macOS notification body for a share event.
///
/// - If a non-empty note is present: return the note, truncated to 100
///   *Unicode scalar values* (characters, not bytes) with a "…" suffix when
///   truncated. Using character count avoids a panic on multi-byte sequences.
/// - Otherwise: return the comma-joined basenames of the shared paths.
pub(crate) fn notification_body(note: Option<&str>, paths: &[String]) -> String {
    const CHAR_LIMIT: usize = 100;
    match note {
        Some(n) if !n.is_empty() => {
            let char_count = n.chars().count();
            if char_count > CHAR_LIMIT {
                // Find the byte offset of the CHAR_LIMIT-th character boundary.
                let cut = n
                    .char_indices()
                    .nth(CHAR_LIMIT)
                    .map(|(i, _)| i)
                    .unwrap_or(n.len());
                format!("{}…", &n[..cut])
            } else {
                n.to_string()
            }
        }
        _ => {
            let basenames: Vec<&str> = paths
                .iter()
                .map(|p| p.rsplit('/').next().unwrap_or(p.as_str()))
                .collect();
            basenames.join(", ")
        }
    }
}

// ── ShareDetail window ─────────────────────────────────────────────────────────

const SHARE_DETAIL_LABEL: &str = "share-detail";

/// Tauri command: open (or focus) the ShareDetail window with the given events.
///
/// Mirrors `open_new_files_detail` in new_files.rs:
/// 1. Stash events in `PendingShareEvents` managed state.
/// 2. If window exists, show + focus it and re-emit the list directly.
/// 3. Otherwise create it hidden; `share_detail_window_ready` will show it.
#[tauri::command]
pub async fn open_share_detail(
    app: AppHandle,
    events: Vec<ShareEvent>,
) -> Result<(), String> {
    // Stash so the ready-handshake can retrieve them.
    if let Some(state) = app.try_state::<PendingShareEvents>() {
        *state.0.lock().unwrap_or_else(|p| p.into_inner()) = events.clone();
    }

    if let Some(window) = app.get_webview_window(SHARE_DETAIL_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        app.emit_to(SHARE_DETAIL_LABEL, EVENT_SHARE_EVENTS_LIST, &events)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        SHARE_DETAIL_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Shared with Me")
    .inner_size(640.0, 560.0)
    .resizable(true)
    .decorations(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Tauri command: called by ShareDetail.svelte once its event listener is
/// registered. Emits pending events, shows the window, fires best-effort ack,
/// and clears the tray badge.
#[tauri::command]
pub async fn share_detail_window_ready(app: AppHandle) -> Result<(), String> {
    let events: Vec<ShareEvent> = app
        .try_state::<PendingShareEvents>()
        .map(|s| s.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
        .unwrap_or_default();

    app.emit_to(SHARE_DETAIL_LABEL, EVENT_SHARE_EVENTS_LIST, &events)
        .map_err(|e| e.to_string())?;

    if let Some(window) = app.get_webview_window(SHARE_DETAIL_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }

    // Best-effort ack — fire-and-forget so the window doesn't block on it.
    if !events.is_empty() {
        let event_ids: Vec<String> = events.iter().map(|e| e.event_id.clone()).collect();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            post_ack(&app_clone, event_ids).await;
            crate::tray::clear_share_badge(&app_clone);
        });
    }

    Ok(())
}

/// POST `/v1/files/shared-with-me/ack` with the given event IDs.
/// Best-effort: errors are logged but never surfaced to the caller.
async fn post_ack(_app: &AppHandle, event_ids: Vec<String>) {
    let access_token = match cognito::get_valid_access_token().await {
        Ok(t) => t,
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_AUTH_FAIL {e}"));
            return;
        }
    };
    let base_url = match resolve_vault_api_url() {
        Ok(u) => u.trim_end_matches('/').to_string(),
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_ERROR cannot resolve vault URL: {e}"));
            return;
        }
    };
    let url = format!("{}/v1/files/shared-with-me/ack", base_url);
    let body = serde_json::json!({ "eventIds": event_ids });

    match build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_ACK_OK {} event(s)", event_ids.len()),
            );
        }
        Ok(r) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_ERROR status={}", r.status()));
        }
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_NETWORK_FAIL {e}"));
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_singleton_lock_starts_false() {
        // The OnceLock starts unset; initialised to false on first access.
        let guard = poll_lock().lock().unwrap();
        assert!(!*guard);
    }

    #[test]
    fn test_try_set_and_clear_in_flight() {
        // Force the lock to false first (may already be set from another test
        // calling try_set_in_flight).
        {
            let mut g = poll_lock().lock().unwrap();
            *g = false;
        }
        assert!(try_set_in_flight(), "first attempt should succeed");
        assert!(!try_set_in_flight(), "second attempt while in-flight should fail");
        clear_in_flight();
        assert!(try_set_in_flight(), "after clear, should succeed again");
        clear_in_flight();
    }

    #[test]
    fn test_cursor_store_serde_roundtrip() {
        let mut store = CursorStore::default();
        store.insert(
            "machine-abc".to_string(),
            CursorEntry {
                cursor: Some("2026-05-25T12:00:00.000Z".to_string()),
                notified: vec!["e1".to_string(), "e2".to_string()],
            },
        );
        let json = serde_json::to_string(&store).unwrap();
        let parsed: HashMap<String, CursorEntryCompat> =
            serde_json::from_str(&json).unwrap();
        let entry: CursorEntry = parsed.into_iter().next().unwrap().1.into();
        assert_eq!(entry.cursor.as_deref(), Some("2026-05-25T12:00:00.000Z"));
        assert_eq!(entry.notified, vec!["e1", "e2"]);
    }

    #[test]
    fn test_cursor_store_reads_legacy_bare_string() {
        // Pre-0.4.4 format: bare ISO string per machine. Must upgrade cleanly to
        // the object form without losing the cursor or re-notifying history.
        let legacy = r#"{"machine-abc":"2026-05-25T12:00:00.000Z"}"#;
        let parsed: HashMap<String, CursorEntryCompat> =
            serde_json::from_str(legacy).unwrap();
        let entry: CursorEntry = parsed.into_iter().next().unwrap().1.into();
        assert_eq!(entry.cursor.as_deref(), Some("2026-05-25T12:00:00.000Z"));
        assert!(entry.notified.is_empty());
    }

    fn share_event(id: &str, created_at: &str) -> ShareEvent {
        let json = format!(
            r#"{{"eventId":"{id}","issuerEmail":"a@b.com","issuerDisplayName":"A",
                "paths":["/x.md"],"permission":"read","createdAt":"{created_at}"}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn test_partition_unnotified_drops_already_seen() {
        // The stuck-cursor symptom: the same events come back every poll. Once an
        // id is in `notified`, it must never be returned for notification again.
        let events = vec![
            share_event("e1", "2026-05-29T03:19:02.349Z"),
            share_event("e2", "2026-05-29T03:19:02.349Z"),
        ];
        let notified = vec!["e1".to_string(), "e2".to_string()];
        let (fresh, updated) = partition_unnotified(&events, &notified);
        assert!(fresh.is_empty(), "already-notified events must not re-fire");
        assert_eq!(updated, vec!["e1", "e2"]);
    }

    #[test]
    fn test_partition_unnotified_returns_only_new() {
        let events = vec![
            share_event("e1", "2026-05-29T00:00:00.000Z"), // already seen
            share_event("e3", "2026-05-30T00:00:00.000Z"), // new
        ];
        let notified = vec!["e1".to_string()];
        let (fresh, updated) = partition_unnotified(&events, &notified);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].event_id, "e3");
        assert_eq!(updated, vec!["e1", "e3"]);
    }

    #[test]
    fn test_partition_unnotified_caps_ring_keeping_newest() {
        let notified: Vec<String> = (0..NOTIFIED_CAP).map(|i| format!("old{i}")).collect();
        let events = vec![share_event("brand-new", "2026-06-01T00:00:00.000Z")];
        let (fresh, updated) = partition_unnotified(&events, &notified);
        assert_eq!(fresh.len(), 1);
        assert_eq!(updated.len(), NOTIFIED_CAP);
        assert_eq!(updated.last().unwrap(), "brand-new");
        assert_eq!(updated.first().unwrap(), "old1"); // old0 evicted
    }

    #[test]
    fn test_share_event_deserializes_without_note() {
        let json = r#"{
            "eventId": "e1",
            "issuerEmail": "a@b.com",
            "issuerDisplayName": "Alice",
            "paths": ["/Foo/bar.md"],
            "permission": "read",
            "createdAt": "2026-05-25T00:00:00.000Z"
        }"#;
        let evt: ShareEvent = serde_json::from_str(json).unwrap();
        assert_eq!(evt.event_id, "e1");
        assert_eq!(evt.paths.len(), 1);
        assert!(evt.note.is_none());
    }

    #[test]
    fn test_share_event_deserializes_with_note() {
        let json = r#"{
            "eventId": "e2",
            "issuerEmail": "s@getindigo.ai",
            "issuerDisplayName": "Stefan",
            "paths": ["/Shared/doc.md", "/Shared/img.png"],
            "note": "Please review before Friday",
            "permission": "read",
            "createdAt": "2026-05-25T10:00:00.000Z"
        }"#;
        let evt: ShareEvent = serde_json::from_str(json).unwrap();
        assert_eq!(evt.note.as_deref(), Some("Please review before Friday"));
        assert_eq!(evt.paths.len(), 2);
    }

    #[test]
    fn test_write_and_read_cursor() {
        // Uses a temp directory to avoid polluting ~/.hq
        let tmp = tempfile::tempdir().unwrap();
        let cursor_file = tmp.path().join("share-notify-cursor.json");

        // Manually exercise the cursor store logic (not through the real path
        // functions, which are hardcoded to ~/.hq).
        let machine_id = "test-machine-001";
        let ts = "2026-05-25T12:34:56.789Z";

        let mut store = CursorStore::default();
        store.insert(
            machine_id.to_string(),
            CursorEntry {
                cursor: Some(ts.to_string()),
                notified: vec!["e1".to_string()],
            },
        );
        let json = serde_json::to_string_pretty(&store).unwrap();
        std::fs::write(&cursor_file, &json).unwrap();

        let loaded: HashMap<String, CursorEntryCompat> =
            serde_json::from_str(&std::fs::read_to_string(&cursor_file).unwrap()).unwrap();
        let entry: CursorEntry = loaded.into_iter().next().unwrap().1.into();
        assert_eq!(entry.cursor.as_deref(), Some(ts));
        assert_eq!(entry.notified, vec!["e1"]);
    }

    #[test]
    fn test_cursor_path_under_dot_hq() {
        let path = cursor_path().unwrap();
        assert!(
            path.ends_with(".hq/share-notify-cursor.json"),
            "cursor path must live under ~/.hq, got {path:?}"
        );
    }

    // ── notification_body tests ───────────────────────────────────────────────

    #[test]
    fn test_notification_body_short_note_returned_as_is() {
        let body = notification_body(Some("Please review the Q1 data"), &[]);
        assert_eq!(body, "Please review the Q1 data");
    }

    #[test]
    fn test_notification_body_long_note_truncated_at_100_chars_with_ellipsis() {
        // Build a 150-character ASCII note.
        let long_note: String = "a".repeat(150);
        let body = notification_body(Some(&long_note), &[]);
        // Truncated to 100 chars + "…" (the Unicode ellipsis character, 3 UTF-8 bytes)
        let expected = format!("{}…", "a".repeat(100));
        assert_eq!(body, expected);
        assert_eq!(body.chars().count(), 101); // 100 content chars + 1 ellipsis
    }

    #[test]
    fn test_notification_body_note_exactly_100_chars_not_truncated() {
        let note: String = "b".repeat(100);
        let body = notification_body(Some(&note), &[]);
        assert_eq!(body, note);
        assert!(!body.contains('…'));
    }

    #[test]
    fn test_notification_body_truncates_safely_at_char_boundary_for_multibyte() {
        // Each "😀" is 4 bytes but 1 character. A 150-char emoji string
        // would be 600 bytes — byte-index slicing at 100 would panic; char-aware
        // slicing should not.
        let emoji_note: String = "😀".repeat(150);
        let body = notification_body(Some(&emoji_note), &[]);
        assert!(body.ends_with('…'));
        assert_eq!(body.chars().count(), 101); // 100 emojis + ellipsis
    }

    #[test]
    fn test_notification_body_falls_back_to_comma_joined_basenames_when_no_note() {
        let paths = vec![
            "/vault/reports/q1.csv".to_string(),
            "/vault/data/summary.md".to_string(),
        ];
        let body = notification_body(None, &paths);
        assert_eq!(body, "q1.csv, summary.md");
    }

    #[test]
    fn test_notification_body_falls_back_to_basenames_when_note_is_empty_string() {
        let paths = vec!["reports/annual.pdf".to_string()];
        let body = notification_body(Some(""), &paths);
        assert_eq!(body, "annual.pdf");
    }

    #[test]
    fn test_notification_body_basename_with_no_slash() {
        let paths = vec!["standalone.md".to_string()];
        let body = notification_body(None, &paths);
        assert_eq!(body, "standalone.md");
    }

    #[test]
    fn test_notification_title_format() {
        let title = notification_title("Stefan Johnson");
        assert_eq!(title, "Stefan Johnson shared files with you");
    }

    // ── should_poll gating (post indigo-gate removal, 2026-05-26) ─────────────

    #[test]
    fn test_share_notifications_enabled_defaults_on_when_pref_absent() {
        // Missing pref (None) → ON. A fresh install with no explicit toggle must
        // poll so recipients get notifications without opening Settings.
        assert!(share_notifications_enabled(None));
    }

    #[test]
    fn test_share_notifications_enabled_when_pref_true() {
        assert!(share_notifications_enabled(Some(true)));
    }

    #[test]
    fn test_share_notifications_disabled_only_when_pref_explicitly_false() {
        // The ONLY way the poller is gated off is an explicit opt-out. This is
        // the regression guard for dropping the `@getindigo.ai` gate: a
        // non-getindigo recipient (None / Some(true) pref) must still poll.
        assert!(!share_notifications_enabled(Some(false)));
    }

    // ── BlockingNotifyGuard cap-to-1 (CPU spin regression, 2026-05-28) ─────────
    //
    // Regression guard for the 673% CPU leak: `mac-notification-sys`
    // busy-spins one thread per outstanding `wait_for_click(true)` send. The
    // guard caps concurrent blocking sends to exactly one process-wide so the
    // spin can never accumulate. These tests assert the second concurrent
    // acquire is refused (→ caller fire-and-forgets) and that the slot frees on
    // drop. NOTE: this is the only test that touches BLOCKING_NOTIFY_IN_FLIGHT,
    // so the shared static can't be perturbed by sibling tests.

    //
    // Both invariants live in ONE test (not two) on purpose: cargo runs tests
    // in parallel threads within a process, and the guard's backing static is
    // process-wide — two separate tests racing on it would flake. A single
    // sequential test owns the slot for its whole body.
    #[test]
    fn test_blocking_guard_caps_concurrency_at_one() {
        let first = BlockingNotifyGuard::try_acquire();
        assert!(first.is_some(), "first acquire must claim the free slot");

        // Second acquire while the first is held → None. This is what forces
        // concurrent notification sends onto the fire-and-forget path instead
        // of leaking another spinning thread.
        assert!(
            BlockingNotifyGuard::try_acquire().is_none(),
            "second concurrent acquire must be refused while a send is in flight"
        );

        // Dropping the guard frees the slot (releases the in-flight flag).
        drop(first);
        assert!(
            !BLOCKING_NOTIFY_IN_FLIGHT.load(Ordering::Acquire),
            "the in-flight flag must be cleared once the guard drops"
        );

        // After the in-flight send returns (guard dropped), the slot is reusable.
        let third = BlockingNotifyGuard::try_acquire();
        assert!(third.is_some(), "slot must be reusable after the guard drops");
        drop(third);
    }
}
