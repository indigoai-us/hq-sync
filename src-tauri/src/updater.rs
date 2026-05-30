//! Tauri auto-updater nag — soft, user-initiated, channel-aware.
//!
//! Three channels (`util::release_channel::ReleaseChannel`):
//!   - Stable — every user. Polls the static
//!     `releases/latest/download/latest.json` alias that GitHub already
//!     filters to non-prereleases.
//!   - Beta   — `@getindigo.ai` users by default. Newer of (stable, beta).
//!   - Alpha  — `@getindigo.ai` opt-in via Settings. Newest of anything.
//!
//! Gating is layered:
//!   1. The Settings UI only renders the channel picker for
//!      `@getindigo.ai` users (`available_channels` command).
//!   2. The Rust-side resolver (`effective_channel`) coerces a non-indigo
//!      preference to Stable regardless of what's in `menubar.json` — a
//!      hand-edited config can't escape stable.
//!   3. The endpoint resolver falls back to the static stable alias on
//!      any GitHub API failure, so prerelease users behind a blocked
//!      proxy still get stable updates rather than nothing.
//!
//! This file replaces the older static-endpoint flow that called
//! `app.updater()` directly. The static endpoint in `tauri.conf.json` is
//! kept as the Stable channel URL — the `version_gate.rs` hard-yank path
//! continues to use it via `app.updater()` because hard-yank always pulls
//! the newest stable, regardless of channel preference.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

use crate::commands::config::MenubarPrefs;
use crate::util::feature_gate;
use crate::util::logfile::log;
use crate::util::paths;
use crate::util::release_channel::{
    effective_channel, resolve_channel_endpoint, ReleaseChannel,
};

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub body: Option<String>,
    pub date: Option<String>,
}

/// Stores pending update info so the frontend can query it.
pub struct PendingUpdate(pub Mutex<Option<UpdateInfo>>);

/// Read the user's stored channel preference from `~/.hq/menubar.json`,
/// or `None` if the file is missing / unparseable / the field is absent.
/// We deliberately do NOT propagate errors — a corrupted menubar.json
/// must never break the updater. The caller treats `None` as "no
/// preference" and lets `effective_channel` apply identity-aware
/// defaults.
fn read_stored_release_channel() -> Option<String> {
    let path = paths::menubar_json_path().ok()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let prefs: MenubarPrefs = serde_json::from_str(&contents).ok()?;
    prefs.release_channel
}

/// Resolve the per-channel updater endpoint URL the background loop and
/// on-demand command should poll. Combines the stored preference with
/// the indigo gate, then resolves to a `latest.json` URL via the
/// `release_channel` module.
async fn resolve_endpoint_url() -> String {
    let stored = read_stored_release_channel();
    let is_indigo = feature_gate::is_indigo_user().await;
    let channel: ReleaseChannel = effective_channel(stored.as_deref(), is_indigo);
    let url = resolve_channel_endpoint(channel).await;
    log(
        "updater",
        &format!("resolved channel={} endpoint={}", channel.as_str(), url),
    );
    url
}

/// Build a channel-aware `tauri_plugin_updater::Updater` for this
/// invocation. Tauri's `app.updater()` always uses the static endpoint
/// from `tauri.conf.json`; `app.updater_builder()` lets us override
/// per-call so the same binary serves three channels without rebuild.
async fn channel_aware_updater(
    app: &AppHandle,
) -> Result<tauri_plugin_updater::Updater, String> {
    let endpoint_str = resolve_endpoint_url().await;
    let endpoint =
        Url::parse(&endpoint_str).map_err(|e| format!("invalid updater endpoint: {e}"))?;
    app.updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| format!("updater_builder.endpoints: {e}"))?
        .build()
        .map_err(|e| format!("updater_builder.build: {e}"))
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = channel_aware_updater(&app).await?;
    match updater.check().await {
        Ok(Some(update)) => {
            let info = UpdateInfo {
                version: update.version.clone(),
                body: update.body.clone(),
                date: update.date.map(|d| d.to_string()),
            };
            // Store as pending
            if let Some(state) = app.try_state::<PendingUpdate>() {
                *state.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(info.clone());
            }
            // Emit event for frontend (popover update UI)
            let _ = app.emit("update:available", &info);
            // Also raise the custom banner so a version drop surfaces even with
            // the popover closed (gated on customBanner; purely additive — the
            // in-app UI above is unchanged).
            if crate::commands::banner::custom_banner_enabled() {
                let _ = crate::commands::banner::show_update_banner(
                    app.clone(),
                    info.version.clone(),
                    info.body.clone(),
                )
                .await;
            }
            Ok(Some(info))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    // Note: We must call updater.check() again here because the tauri_plugin_updater::Update
    // type cannot be stored (not Clone). The PendingUpdate state only holds metadata (UpdateInfo).
    // This is an architectural constraint of the plugin, not a redundant call.
    //
    // We re-resolve the channel endpoint at install time too — if the user
    // changes their channel preference between "Check Now" and "Install",
    // we honor the latest choice. The endpoint resolution is cheap (the
    // GH API result is uncached but the next 6h check will hit it again
    // anyway) and the stable fallback path skips the network entirely.
    let updater = channel_aware_updater(&app).await?;
    match updater.check().await {
        Ok(Some(update)) => {
            // Download and install
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|e| e.to_string())?;
            // On macOS, download_and_install typically terminates the process before reaching
            // this line. restart() is retained as a safety net for platforms where it returns.
            app.restart();
        }
        Ok(None) => Err("No update available".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Tauri command returning the list of channels the user is allowed to
/// pick from. `@getindigo.ai` users see all three (stable / beta /
/// alpha); everyone else sees only stable, which makes the picker
/// degenerate into a non-interactive label — the Svelte side keys off
/// `length > 1` to decide whether to render it.
///
/// Returned in display order: stable → beta → alpha (most stable first).
#[tauri::command]
pub async fn available_channels() -> Vec<String> {
    if feature_gate::is_indigo_user().await {
        vec![
            ReleaseChannel::Stable.as_str().to_string(),
            ReleaseChannel::Beta.as_str().to_string(),
            ReleaseChannel::Alpha.as_str().to_string(),
        ]
    } else {
        vec![ReleaseChannel::Stable.as_str().to_string()]
    }
}

/// Spawns a background task that checks for updates on launch (after 10s delay)
/// and every 6 hours thereafter. Emits `update:available` events but does NOT
/// auto-install — the user must initiate installation.
pub fn setup_update_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Wait 10 seconds for app to settle
        tokio::time::sleep(Duration::from_secs(10)).await;

        loop {
            // Check for updates silently — log errors for field debugging via Console.app
            match channel_aware_updater(&handle).await {
                Ok(updater) => match updater.check().await {
                    Ok(Some(update)) => {
                        let info = UpdateInfo {
                            version: update.version.clone(),
                            body: update.body.clone(),
                            date: update.date.map(|d| d.to_string()),
                        };
                        if let Some(state) = handle.try_state::<PendingUpdate>() {
                            *state.0.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(info.clone());
                        }
                        let _ = handle.emit("update:available", &info);
                        if crate::commands::banner::custom_banner_enabled() {
                            let _ = crate::commands::banner::show_update_banner(
                                handle.clone(),
                                info.version.clone(),
                                info.body.clone(),
                            )
                            .await;
                        }
                    }
                    Ok(None) => {} // No update available — nothing to do
                    Err(e) => eprintln!("[updater] background check failed: {e}"),
                },
                Err(e) => eprintln!("[updater] failed to build channel-aware updater: {e}"),
            }
            // Wait 6 hours before next check
            tokio::time::sleep(Duration::from_secs(21600)).await;
        }
    });
}
