//! Custom in-app notification banner for HQ Sync.
//!
//! ## Why this exists
//!
//! The native banner path (`mac-notification-sys`, used by `dm_notify.rs` and
//! `share_notify.rs`) is brittle:
//!
//!   * the clickable path busy-spins a Cocoa run loop (`wait_for_click(true)`
//!     ≈ 1 core, capped by `BlockingNotifyGuard` — see `hq-sync-cpu-spin`),
//!   * `tauri-plugin-notification`'s desktop impl hardcodes permission state
//!     (see `notifications.rs`), and
//!   * macOS Focus/DND silently swallows banners with no app-visible signal.
//!
//! HQ Sync is a menu-bar app — always resident — so the strongest reasons to
//! keep system notifications (delivery when closed, Notification Center
//! history) mostly don't apply. This module renders a fully-controlled,
//! transparent, always-on-top, non-activating webview banner with the same
//! NSVisualEffectView vibrancy as the detail windows.
//!
//! ## One surface, many sources
//!
//! Every notification source builds a neutral [`BannerPayload`] and calls
//! [`show_banner`]. On action, [`banner_action`] re-emits a single
//! `notification:banner-action` event `{kind, action, data}` that `App.svelte`
//! routes by `kind` — DMs open the DM detail / copy a prompt, shares open the
//! share detail, updates install or reveal the popover. The `data` field is the
//! opaque source event echoed back, so no re-fetch is needed.
//!
//! Sources:
//!   * DMs    — [`show_dm_banner`]    (gated by `customBanner` in `dm_notify`)
//!   * Shares — [`show_share_banner`] (gated by `customBanner` in `share_notify`)
//!   * Update — [`show_update_banner`] (raised from `updater` on `update:available`)
//!
//! ## Productionisation notes (out of spike scope)
//!
//!   * Convert the NSWindow to a true `NSPanel` (`.nonactivatingPanel` +
//!     `canBecomeKey = false`) via `tauri-nspanel`. Accessory activation policy
//!     + `focused(false)` covers the common case today.
//!   * Multi-banner **stacking** (vertical offset per live banner). Today a
//!     second notification replaces the first in the single banner window.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindowBuilder};

use crate::util::logfile::log;

const LOG_TAG: &str = "banner";

/// Window label — kept in sync with the `main.ts` router branch and
/// `capabilities/dm-banner.json`.
pub const WINDOW_LABEL: &str = "dm-banner";

/// Tauri event the banner webview listens for to receive its payload.
const EVENT_BANNER: &str = "banner:event";

/// Unified action event. `App.svelte` has one listener that routes by `kind`.
/// Replaces the per-source `notification:dm-action` / `notification:share-action`
/// for the CUSTOM banner path (the native paths still emit their own events).
const EVENT_BANNER_ACTION: &str = "notification:banner-action";

/// Banner geometry (logical px). `BANNER_H` is sized tight to the card content
/// (avatar + title + two-line body + action row) so the vibrancy backdrop and
/// the rounded card coincide with no dead padding. We do NOT resize the window
/// from the webview: resizing an NSWindow leaves the NSVisualEffectView's
/// rounded-corner mask at the old geometry, exposing square corners behind the
/// card. Fixed size keeps the corners clean.
const BANNER_W: f64 = 366.0;
const BANNER_H: f64 = 104.0;
const MARGIN_RIGHT: f64 = 14.0;
const MARGIN_TOP: f64 = 40.0;

/// Neutral notification payload rendered by `BannerNotification.svelte`. Every
/// source maps its event onto this shape; `data` carries the original event
/// (a `DmEvent`, `ShareEvent`, or update info) echoed back on action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannerPayload {
    /// `"dm" | "share" | "update"` — routes the action in `App.svelte`.
    pub kind: String,
    /// Secondary label shown after "HQ Sync ·" (sender / source).
    pub title: String,
    /// Body line (clamped to two lines in the UI).
    pub body: String,
    /// Avatar text — initials for people, a glyph for system sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_text: Option<String>,
    /// Primary action chip label, e.g. "Copy prompt" / "Update now". None → no chip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_label: Option<String>,
    /// Action id dispatched when the chip is clicked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    /// Action id dispatched on a body click (the discoverable default).
    pub click_action_id: String,
    /// Opaque source event echoed back on action (DmEvent / ShareEvent / update info).
    pub data: serde_json::Value,
}

/// Managed state: the payload pending for the banner's ready-handshake.
pub struct PendingBanner(pub Mutex<Option<BannerPayload>>);

/// Action re-dispatched to `App.svelte`. One shape for every source.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BannerActionEvent {
    kind: String,
    action: String,
    data: serde_json::Value,
}

// ── Shared gate ──────────────────────────────────────────────────────────────

/// True when DMs/shares/updates should route through the custom banner instead
/// of `mac-notification-sys`. **Default ON** as of v0.3.0 — custom notifications
/// are the default surface for everyone; set `"customBanner": false` in
/// `~/.hq/menubar.json` to fall back to native. Read directly so the toggle is
/// additive and picked up live on the next poll (no restart). Shared by
/// `dm_notify`, `share_notify`, and `updater`.
pub(crate) fn custom_banner_enabled() -> bool {
    let Ok(dir) = crate::util::paths::hq_config_dir() else {
        return true;
    };
    let Ok(contents) = std::fs::read_to_string(dir.join("menubar.json")) else {
        return true;
    };
    serde_json::from_str::<serde_json::Value>(&contents)
        .ok()
        .and_then(|j| j.get("customBanner").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Up-to-two-letter initials from a display name, for the avatar.
fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [] => "?".to_string(),
        [one] => one.chars().take(2).collect::<String>().to_uppercase(),
        [first, .., last] => {
            let a = first.chars().next().unwrap_or('?');
            let b = last.chars().next().unwrap_or('?');
            format!("{a}{b}").to_uppercase()
        }
    }
}

fn top_right_position(app: &AppHandle) -> tauri::LogicalPosition<f64> {
    let monitor = app
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| app.available_monitors().ok().and_then(|m| m.into_iter().next()));
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let logical_w = m.size().width as f64 / scale;
        let x = (logical_w - BANNER_W - MARGIN_RIGHT).max(0.0);
        return tauri::LogicalPosition::new(x, MARGIN_TOP);
    }
    tauri::LogicalPosition::new(1440.0 - BANNER_W - MARGIN_RIGHT, MARGIN_TOP)
}

// ── Core: show any banner ───────────────────────────────────────────────────────

/// Show (or refresh) the banner for a neutral [`BannerPayload`].
///
/// Single-window: a second notification reuses the same window (focus +
/// re-emit). Stacking is a productionisation note above.
pub async fn show_banner(app: AppHandle, payload: BannerPayload) -> Result<(), String> {
    log(
        LOG_TAG,
        &format!("show: kind={} title={} body_len={}", payload.kind, payload.title, payload.body.len()),
    );

    if let Some(state) = app.try_state::<PendingBanner>() {
        *state.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(payload.clone());
    }

    let pos = top_right_position(&app);

    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.set_position(pos);
        window.show().map_err(|e| e.to_string())?;
        app.emit_to(WINDOW_LABEL, EVENT_BANNER, &payload)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let _window = WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("HQ Sync Notification")
    .inner_size(BANNER_W, BANNER_H)
    .position(pos.x, pos.y)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    // Native shadow ON — the contentView is clipped to a rounded rect (below),
    // so the OS shadow follows the rounded shape. (The card's CSS box-shadow is
    // clipped away by masksToBounds, so the native one provides the drop.)
    .shadow(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible_on_all_workspaces(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    // (1) Clear the WKWebView's `underPageBackgroundColor`. macOS 12+ WebKit
    // paints it (a system gray) behind a transparent page, filling the square
    // window rect — THIS was the "square box behind the rounded card", not the
    // vibrancy/blur/shadow. `transparent: true` does not clear it.
    #[cfg(target_os = "macos")]
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.with_webview(|webview| {
            use objc2::{class, msg_send, runtime::AnyObject};
            // SAFETY: runs on the main thread (with_webview guarantees it);
            // `inner()` is the live WKWebView; selectors are public AppKit/WebKit.
            unsafe {
                let wk = webview.inner() as *mut AnyObject;
                let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
                let _: () = msg_send![wk, setUnderPageBackgroundColor: clear];
                let _: () = msg_send![wk, setValue: clear, forKey: ns_str("backgroundColor")];
            }
        });
    }

    // (2) Apply native NSVisualEffectView vibrancy for the frosted-glass look.
    // Now that the webview gray is cleared, the rounded effect view (radius 18,
    // matching the card) shows cleanly behind the translucent CSS card. AppKit
    // is main-thread-only → dispatch via run_on_main_thread.
    #[cfg(target_os = "macos")]
    {
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
            let Some(window) = app_for_main.get_webview_window(WINDOW_LABEL) else {
                return;
            };
            if let Err(e) = apply_vibrancy(
                &window,
                // Popover material = the app's "Liquid Glass" look (same as the
                // main window's apply_liquid_glass). Brighter / more translucent
                // than HudWindow for a pure-glass feel.
                NSVisualEffectMaterial::Popover,
                Some(NSVisualEffectState::Active),
                Some(18.0), // match the card's border-radius
            ) {
                log(LOG_TAG, &format!("apply_vibrancy FAILED: {e}"));
            }

            // Clip the window CONTENT to a rounded rect at the OS level. The
            // NSVisualEffectView is a square view and window-vibrancy's own
            // `radius` does not reliably clip it on this macOS, so its square
            // corners showed through. cornerRadius + masksToBounds on the
            // contentView's layer clips ALL subviews (the effect view AND the
            // webview) to the rounded shape — the definitive fix.
            use objc2::{msg_send, runtime::AnyObject};
            if let Ok(ns_win) = window.ns_window() {
                let ns_win = ns_win as *mut AnyObject;
                // SAFETY: main thread (run_on_main_thread); AppKit selectors.
                unsafe {
                    let content: *mut AnyObject = msg_send![ns_win, contentView];
                    if !content.is_null() {
                        let _: () = msg_send![content, setWantsLayer: true];
                        let layer: *mut AnyObject = msg_send![content, layer];
                        if !layer.is_null() {
                            let _: () = msg_send![layer, setCornerRadius: 18.0_f64];
                            let _: () = msg_send![layer, setMasksToBounds: true];
                        }
                    }
                }
            }
        });
    }

    Ok(())
}

/// Build an autoreleased `NSString` from a Rust &str for KVC selectors.
#[cfg(target_os = "macos")]
fn ns_str(s: &str) -> *mut objc2::runtime::AnyObject {
    use objc2::{class, msg_send, runtime::AnyObject};
    unsafe {
        let cls = class!(NSString);
        let bytes = s.as_ptr() as *const std::ffi::c_void;
        let ns: *mut AnyObject = msg_send![
            cls,
            stringWithBytes: bytes,
            length: s.len(),
            encoding: 4usize /* NSUTF8StringEncoding */
        ];
        ns
    }
}

// ── Source-specific constructors ─────────────────────────────────────────────────

/// DM → banner. Body-click opens the DM detail; the chip copies the agent
/// prompt when the DM carries one.
pub async fn show_dm_banner(
    app: AppHandle,
    event: crate::commands::dm_notify::DmEvent,
) -> Result<(), String> {
    let has_prompt = event.prompt.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
    let payload = BannerPayload {
        kind: "dm".to_string(),
        title: event.from_display_name.clone(),
        body: event.body.clone(),
        icon_text: Some(initials(&event.from_display_name)),
        action_label: has_prompt.then(|| "Copy prompt".to_string()),
        action_id: has_prompt.then(|| "copy".to_string()),
        click_action_id: "open".to_string(),
        data: serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
    };
    show_banner(app, payload).await
}

/// Share ("shared with me") → banner. Body-click opens the share detail window.
pub async fn show_share_banner(
    app: AppHandle,
    event: crate::commands::share_notify::ShareEvent,
) -> Result<(), String> {
    let title = crate::commands::share_notify::notification_title(&event.issuer_display_name);
    let body = crate::commands::share_notify::notification_body(event.note.as_deref(), &event.paths);
    let payload = BannerPayload {
        kind: "share".to_string(),
        title,
        body,
        icon_text: Some(initials(&event.issuer_display_name)),
        action_label: Some("Open".to_string()),
        action_id: Some("open".to_string()),
        click_action_id: "open".to_string(),
        data: serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
    };
    show_banner(app, payload).await
}

/// New HQ Sync version → banner. The chip installs; a body-click reveals the
/// popover (which carries the full update UI) without forcing a restart.
pub async fn show_update_banner(
    app: AppHandle,
    version: String,
    notes: Option<String>,
) -> Result<(), String> {
    let body = match notes.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => format!("Version {version} is ready — {n}"),
        None => format!("Version {version} is ready to install."),
    };
    let payload = BannerPayload {
        kind: "update".to_string(),
        title: "New version".to_string(),
        body,
        icon_text: Some("⬆".to_string()),
        action_label: Some("Update now".to_string()),
        action_id: Some("update".to_string()),
        click_action_id: "open".to_string(),
        data: serde_json::json!({ "version": version }),
    };
    show_banner(app, payload).await
}

// ── Commands ─────────────────────────────────────────────────────────────────────

/// Called by `BannerNotification.svelte` once its `listen` handler is mounted.
#[tauri::command]
pub async fn banner_window_ready(app: AppHandle) -> Result<(), String> {
    let payload = app
        .try_state::<PendingBanner>()
        .and_then(|s| s.0.lock().unwrap_or_else(|p| p.into_inner()).clone());
    if let Some(payload) = payload {
        app.emit_to(WINDOW_LABEL, EVENT_BANNER, &payload)
            .map_err(|e| e.to_string())?;
    }
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.show();
    }
    Ok(())
}

/// The banner was actioned. Re-emit the unified `notification:banner-action`
/// event for `App.svelte` to route by `kind`, then dismiss the banner.
#[tauri::command]
pub async fn banner_action(
    app: AppHandle,
    action: String,
    payload: BannerPayload,
) -> Result<(), String> {
    log(LOG_TAG, &format!("action kind={} action={}", payload.kind, action));
    app.emit(
        EVENT_BANNER_ACTION,
        BannerActionEvent { kind: payload.kind, action, data: payload.data },
    )
    .map_err(|e| e.to_string())?;
    dismiss_banner_inner(&app);
    Ok(())
}

/// Dismiss the banner (auto-timeout or explicit close).
#[tauri::command]
pub async fn dismiss_banner(app: AppHandle) -> Result<(), String> {
    dismiss_banner_inner(&app);
    Ok(())
}

fn dismiss_banner_inner(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.close();
    }
}

/// Show the main popover anchored under the tray icon. Used by the update
/// banner's body-click so the user lands on the full update UI — positioned at
/// the menu-bar tray (like a normal popover open), NOT centered on screen.
#[tauri::command]
pub async fn show_main_window(app: AppHandle) -> Result<(), String> {
    crate::tray::show_window_at_tray(&app);
    Ok(())
}

// ── Preview triggers (devtools / env-var) ────────────────────────────────────────

/// SPIKE trigger — fabricate a representative DM and show its banner.
#[tauri::command]
pub async fn preview_dm_banner(app: AppHandle) -> Result<(), String> {
    let event = crate::commands::dm_notify::DmEvent {
        event_id: "evt_preview".to_string(),
        from_person_uid: "prs_preview".to_string(),
        from_email: "ada@getindigo.ai".to_string(),
        from_display_name: "Ada Lovelace".to_string(),
        body: "Custom banner spike is live — click me to open the detail window, or hit Copy prompt.".to_string(),
        details: Some("This banner is a transparent Tauri webview with NSVisualEffectView vibrancy, pinned top-right. It auto-dismisses; hover to keep it.".to_string()),
        prompt: Some("Review the custom notification banner spike in repos/public/hq-sync and report on the feel vs native.".to_string()),
        created_at: "2026-05-29T00:00:00Z".to_string(),
    };
    show_dm_banner(app, event).await
}

/// Fabricate a representative "shared with me" event and show its banner.
#[tauri::command]
pub async fn preview_share_banner(app: AppHandle) -> Result<(), String> {
    let event = crate::commands::share_notify::ShareEvent {
        event_id: "shr_preview".to_string(),
        issuer_email: "grace@getindigo.ai".to_string(),
        issuer_display_name: "Grace Hopper".to_string(),
        paths: vec![
            "indigo/reports/q1-forecast.md".to_string(),
            "indigo/reports/q1-model.xlsx".to_string(),
        ],
        note: Some("Sharing the Q1 forecast — take a look before our sync.".to_string()),
        permission: "read".to_string(),
        created_at: "2026-05-29T00:00:00Z".to_string(),
    };
    show_share_banner(app, event).await
}

/// Fabricate a new-version event and show its banner.
#[tauri::command]
pub async fn preview_update_banner(app: AppHandle) -> Result<(), String> {
    show_update_banner(app, "0.4.0".to_string(), Some("instant DMs + custom banners".to_string())).await
}
