//! macOS notification-permission surface for HQ Sync.
//!
//! The app already *fires* notifications (share events via
//! `mac-notification-sys` in `share_notify.rs`), but it never inspected or
//! requested the OS-level authorization. These two commands wrap the
//! `tauri-plugin-notification` Rust API so the frontend can:
//!
//!   * read the current permission status (Settings monitor pill), and
//!   * request authorization once at launch.
//!
//! We wrap the plugin in our own commands rather than exposing the plugin's
//! IPC permissions to the webview — keeps the capability grant minimal (no
//! `notification:*` permissions in `capabilities/default.json`) and gives the
//! frontend a stable tri-state string contract independent of the plugin's
//! serde representation.
//!
//! ## macOS "prompt once" semantics
//!
//! `request_permission()` maps to `UNUserNotificationCenter` authorization.
//! macOS only shows the system dialog while the status is *not determined*
//! (`prompt`); once granted or denied it silently returns the existing status
//! with no dialog. So the OS itself guarantees the prompt fires at most once —
//! the frontend can call this on every launch without re-nagging the user.

use tauri::AppHandle;
use tauri_plugin_notification::{NotificationExt, PermissionState};

/// Stable tri-state contract for the frontend. `prompt` collapses both
/// `Prompt` and the Android-only `PromptWithRationale` — on macOS only the
/// former occurs, but the match stays exhaustive.
fn state_to_str(state: PermissionState) -> &'static str {
    match state {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        _ => "prompt",
    }
}

/// Read the current OS notification authorization without prompting.
/// Returns `"granted" | "denied" | "prompt"`.
#[tauri::command]
pub async fn notification_permission_state(app: AppHandle) -> Result<String, String> {
    let state = app
        .notification()
        .permission_state()
        .map_err(|e| format!("Failed to read notification permission: {e}"))?;
    Ok(state_to_str(state).to_string())
}

/// Request OS notification authorization. On macOS this shows the system
/// dialog only when the status is not yet determined; afterwards it silently
/// returns the existing status. Returns `"granted" | "denied" | "prompt"`.
#[tauri::command]
pub async fn notification_request_permission(app: AppHandle) -> Result<String, String> {
    let state = app
        .notification()
        .request_permission()
        .map_err(|e| format!("Failed to request notification permission: {e}"))?;
    Ok(state_to_str(state).to_string())
}
