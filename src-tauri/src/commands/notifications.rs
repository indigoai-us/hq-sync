//! macOS notification-permission surface for HQ Sync.
//!
//! The app already *fires* notifications (share + DM events via
//! `mac-notification-sys`), but it needs to report the *real* OS-level
//! authorization so the Settings monitor pill is honest. These two commands
//! expose a stable tri-state string contract to the frontend:
//!
//!   * read the current permission status (Settings monitor pill), and
//!   * request authorization (shows the system dialog when not-yet-determined).
//!
//! ## Why we don't use `tauri-plugin-notification` for this
//!
//! `tauri-plugin-notification` 2.3.3's **desktop** implementation hardcodes
//! both `permission_state()` and `request_permission()` to
//! `PermissionState::Granted` (see the plugin's `src/desktop.rs`) — the real
//! `UNUserNotificationCenter` logic exists only in its iOS Swift sources. So
//! on the macOS build we ship, the plugin always reports "granted" and never
//! shows a dialog. The pill was therefore always green ("Enabled") regardless
//! of the actual System Settings state.
//!
//! Instead we query `UNUserNotificationCenter` directly via `objc2` and map
//! its `authorizationStatus` to our tri-state contract.
//!
//! ## macOS "prompt once" semantics
//!
//! `requestAuthorizationWithOptions:completionHandler:` shows the system dialog
//! only while the status is `notDetermined`; once granted or denied it returns
//! the existing status silently. So the frontend can call it without re-nagging.
//!
//! ## Bundle guard (dev-mode safety)
//!
//! `UNUserNotificationCenter.currentNotificationCenter()` requires the process
//! to be a bundled, signed `.app` — under `npm run tauri dev` (an unbundled
//! binary with a nil `bundleIdentifier`) it traps. We detect a missing bundle
//! identifier and return `"unknown"` instead of crashing. Production DMGs are
//! bundled, so this only affects local dev.

use tauri::AppHandle;

/// Stable string contract for the frontend:
/// `"granted" | "denied" | "prompt" | "unknown"`.
///
/// `UNAuthorizationStatus` raw values (from `UserNotifications.framework`):
/// 0 = notDetermined, 1 = denied, 2 = authorized, 3 = provisional, 4 = ephemeral.
/// We treat provisional/ephemeral as effectively granted (notifications deliver),
/// notDetermined as `prompt` (a request would show the dialog), and any
/// unexpected value as `unknown` so the UI can stay neutral rather than lie.
#[cfg(target_os = "macos")]
fn auth_status_to_str(status: isize) -> &'static str {
    match status {
        2 | 3 | 4 => "granted",
        1 => "denied",
        0 => "prompt",
        _ => "unknown",
    }
}

/// Read the current OS notification authorization without prompting.
/// Returns `"granted" | "denied" | "prompt" | "unknown"`.
#[cfg_attr(target_os = "macos", allow(unused_variables))]
#[tauri::command]
pub async fn notification_permission_state(app: AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(macos::read_authorization_status())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Non-macOS targets don't ship this UI; report neutral.
        let _ = app;
        Ok("unknown".to_string())
    }
}

/// Request OS notification authorization. On macOS this shows the system dialog
/// only when the status is not yet determined; afterwards it silently returns
/// the existing status. Returns the freshly-read state
/// `"granted" | "denied" | "prompt" | "unknown"`.
#[cfg_attr(target_os = "macos", allow(unused_variables))]
#[tauri::command]
pub async fn notification_request_permission(app: AppHandle) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        macos::request_authorization();
        // Re-read the real status — more truthful than the request callback's
        // bool (which only reflects the just-made decision, not denials made
        // earlier in System Settings).
        Ok(macos::read_authorization_status())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok("unknown".to_string())
    }
}

/// Synchronous, side-effect-free read of the current notification authorization
/// status. Unlike `notification_request_permission`, this never shows the system
/// dialog — it just reports the tri-state contract (`"granted" | "denied" |
/// "prompt" | "unknown"`). Used by the meeting-detection path to decide whether
/// the clickable `UNUserNotificationCenter` delivery will actually surface
/// (UN silently drops requests when status != granted) before choosing it over
/// the always-visible `osascript` fallback.
pub(crate) fn current_authorization_status() -> String {
    #[cfg(target_os = "macos")]
    {
        macos::read_authorization_status()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "unknown".to_string()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::auth_status_to_str;
    use block2::RcBlock;
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    use std::sync::mpsc;
    use std::time::Duration;

    /// `UNUserNotificationCenter.currentNotificationCenter()` traps when the
    /// main bundle has no `bundleIdentifier` (unbundled dev binary). Guard on it.
    fn is_bundled() -> bool {
        unsafe {
            let bundle_cls = class!(NSBundle);
            let main: *mut AnyObject = msg_send![bundle_cls, mainBundle];
            if main.is_null() {
                return false;
            }
            let ident: *mut AnyObject = msg_send![main, bundleIdentifier];
            !ident.is_null()
        }
    }

    /// Read `UNUserNotificationCenter` authorization status, mapped to our
    /// tri-state contract. Returns `"unknown"` when unbundled, when the center
    /// is unavailable, or when the async callback doesn't fire within 2s.
    pub fn read_authorization_status() -> String {
        if !is_bundled() {
            return "unknown".to_string();
        }
        let (tx, rx) = mpsc::channel::<isize>();
        unsafe {
            let center_cls = class!(UNUserNotificationCenter);
            let center: *mut AnyObject = msg_send![center_cls, currentNotificationCenter];
            if center.is_null() {
                return "unknown".to_string();
            }
            // RcBlock is heap-allocated + retained by the framework for the
            // duration of the async call, so a late callback (after our recv
            // timeout returns) can't use-after-free a stack block.
            let handler = RcBlock::new(move |settings: *mut AnyObject| {
                let status: isize = if settings.is_null() {
                    -1
                } else {
                    msg_send![settings, authorizationStatus]
                };
                let _ = tx.send(status);
            });
            let _: () = msg_send![center, getNotificationSettingsWithCompletionHandler: &*handler];
        }
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(status) => auth_status_to_str(status).to_string(),
            Err(_) => "unknown".to_string(),
        }
    }

    /// Request authorization (alert | badge | sound). Shows the system dialog
    /// only when status is `notDetermined`. Best-effort: returns once the
    /// callback fires or after a 2s timeout. The caller re-reads the real state.
    pub fn request_authorization() {
        if !is_bundled() {
            return;
        }
        // UNAuthorizationOptions: badge=1, sound=2, alert=4.
        const OPTIONS: usize = 1 | 2 | 4;
        let (tx, rx) = mpsc::channel::<bool>();
        unsafe {
            let center_cls = class!(UNUserNotificationCenter);
            let center: *mut AnyObject = msg_send![center_cls, currentNotificationCenter];
            if center.is_null() {
                return;
            }
            let handler = RcBlock::new(move |granted: Bool, _err: *mut AnyObject| {
                let _ = tx.send(granted.as_bool());
            });
            let _: () = msg_send![
                center,
                requestAuthorizationWithOptions: OPTIONS,
                completionHandler: &*handler
            ];
        }
        // Block until the user dismisses the dialog (or a generous timeout):
        // the dialog is modal-ish, so allow longer than the read path.
        let _ = rx.recv_timeout(Duration::from_secs(60));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::auth_status_to_str;

    #[test]
    fn maps_un_authorization_status_to_tristate() {
        // 0 notDetermined -> prompt
        assert_eq!(auth_status_to_str(0), "prompt");
        // 1 denied -> denied (the case the old hardcoded "granted" hid)
        assert_eq!(auth_status_to_str(1), "denied");
        // 2 authorized -> granted
        assert_eq!(auth_status_to_str(2), "granted");
        // 3 provisional, 4 ephemeral -> granted (notifications still deliver)
        assert_eq!(auth_status_to_str(3), "granted");
        assert_eq!(auth_status_to_str(4), "granted");
        // unexpected / sentinel -> unknown (UI stays neutral, never lies)
        assert_eq!(auth_status_to_str(-1), "unknown");
        assert_eq!(auth_status_to_str(99), "unknown");
    }
}
