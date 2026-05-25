//! Permissions commands — open System Settings to specific Privacy panes.
//!
//! macOS supports the `x-apple.systempreferences:` URL scheme for deep-linking
//! into individual Privacy & Security panes. The mapping below is the public
//! contract documented by Apple plus a few panes (`Privacy_ScreenCapture`,
//! `Privacy_ListenEvent`) that have shipped in modern macOS releases.
//!
//! Recall Desktop SDK requires 5 macOS permissions: accessibility,
//! screen-capture, microphone, system-audio (tied to screen-capture in
//! Sequoia+), and full-disk-access. The Svelte UI deep-links each to the
//! right pane so the user goes straight there.

use std::process::Command;

use crate::util::logfile::log;

const LOG_TAG: &str = "permissions";

// ─────────────────────────────────────────────────────────────────────────────
// Native macOS APIs — used to force TCC to register `hq-sync-menubar` (and
// hence the parent .app bundle) for permissions the Recall SDK's child
// process can't claim on our behalf.
//
// **Why this is necessary:** when the SDK's `desktop_sdk_macos_exe` calls
// TCC APIs (CGRequestScreenCaptureAccess, AXIsProcessTrusted, etc.), macOS
// attributes the request to the calling binary's responsible-code chain.
// In production .app bundles the chain resolves to the bundle, but for
// permissions where the user has previously denied a DIFFERENT binary
// (e.g. an old dev build), macOS will silently return the cached "denied"
// state and never re-prompt — so HQ Sync.app never appears in the privacy
// pane.
//
// By calling these APIs from `hq-sync-menubar` directly (which lives inside
// the .app bundle, so its responsible code IS the bundle), we force a fresh
// TCC entry for the .app itself.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    // Accessibility — exists in the ApplicationServices framework.
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        /// Returns true if the process is trusted for Accessibility. Calling
        /// it registers the app in the Accessibility list (denied by default).
        pub fn AXIsProcessTrusted() -> bool;

        /// Variant that accepts an options dict — passing
        /// `kAXTrustedCheckOptionPrompt = true` shows the system prompt
        /// dialog explaining how to grant. We don't pass options (null) so
        /// we just get registration without a modal.
        pub fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    }

    // Screen Recording — CoreGraphics functions (macOS 10.15+).
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// Triggers the system prompt on first call; silently returns
        /// current authorization status on subsequent calls. Either way,
        /// the calling binary gets registered in Screen Recording.
        pub fn CGRequestScreenCaptureAccess() -> bool;

        /// Returns the current status without prompting.
        pub fn CGPreflightScreenCaptureAccess() -> bool;
    }

    // AVFoundation — needed to link the framework so AVCaptureDevice symbols
    // resolve at load time. The Rust side calls into it via objc2 / block2
    // (see `request_microphone_access`), not via these extern declarations.
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}
}

/// Register the .app bundle with macOS TCC for Microphone (and System Audio
/// by extension — on Sequoia+ they're consolidated into one privacy pane).
///
/// macOS Microphone only gets populated when an app actively *requests*
/// access via `+[AVCaptureDevice requestAccessForMediaType:completionHandler:]`.
/// There's no `+` button in System Settings to add an app manually like
/// there is for Accessibility or Screen Recording.
///
/// The Recall SDK's helper binary already makes this request from inside
/// the bundle, BUT TCC attributes it to the helper's own (ad-hoc) identity
/// rather than the parent .app, so the prompt never fires and HQ Sync never
/// appears in the Microphone list. Calling the same API from `hq-sync-menubar`
/// (whose identity is the stable signed .app bundle) makes TCC attach the
/// grant to "HQ Sync".
///
/// Fire-and-forget — the completion handler is a no-op stack block. We
/// don't need the granted bool here; the SDK's own polling reads back the
/// status once TCC has the entry.
#[cfg(target_os = "macos")]
fn request_microphone_access() {
    use block2::RcBlock;
    use objc2::{
        class, msg_send,
        runtime::{AnyClass, AnyObject, Bool},
    };

    // SAFETY: every msg_send below targets a class method on a documented
    // Apple framework class. AVCaptureDevice + NSString are always present
    // at runtime on macOS 10.7+. Pointer ownership is autorelease-pool
    // managed by ObjC; we never retain across the call boundary.
    unsafe {
        let av_cls: &AnyClass = class!(AVCaptureDevice);

        // AVMediaTypeAudio is the NSString constant @"soun". Build it from
        // a C string literal rather than dlsym'ing the global to keep this
        // self-contained.
        let ns_string_cls: &AnyClass = class!(NSString);
        let audio_type: *mut AnyObject = msg_send![
            ns_string_cls,
            stringWithUTF8String: b"soun\0".as_ptr() as *const i8
        ];
        if audio_type.is_null() {
            log(LOG_TAG, "request_microphone_access: NSString init failed");
            return;
        }

        // The completion handler is non-optional per Apple's header (passing
        // nil crashes inside AVFoundation). RcBlock heap-allocates the block
        // with an internal retain count, so AVFoundation can hold its own
        // strong ref while we drop our handle here — the block stays alive
        // until AVFoundation invokes it once and releases.
        //
        // The selector signature is `^(BOOL granted)`. ObjC `BOOL` maps to
        // `objc2::runtime::Bool` (NOT Rust bool, which has a different ABI
        // on i386 — Bool is a u8 wrapper that always matches ObjC's contract).
        let handler = RcBlock::new(|granted: Bool| {
            log(
                LOG_TAG,
                &format!(
                    "AVCaptureDevice.requestAccess(audio) -> granted={}",
                    granted.as_bool()
                ),
            );
        });

        log(LOG_TAG, "AVCaptureDevice.requestAccess(audio): calling");
        let _: () = msg_send![
            av_cls,
            requestAccessForMediaType: audio_type,
            completionHandler: &*handler
        ];
    }
}

/// Trigger native macOS API calls for Accessibility + Screen Recording +
/// Microphone from the `hq-sync-menubar` process itself. Idempotent — safe
/// to call on every app launch.
///
/// Returns a status report `(accessibility_trusted, screen_capture_authorized)`
/// that the caller logs. Microphone is fire-and-forget (the prompt is
/// async; the bool isn't useful in the return value).
#[tauri::command]
pub fn permissions_force_native_register() -> Result<(bool, bool), String> {
    #[cfg(target_os = "macos")]
    {
        use macos::*;

        // SAFETY: both C functions are safe to call from any thread; they
        // take no Rust references that could outlive the call.
        let ax = unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) };
        log(
            LOG_TAG,
            &format!("AXIsProcessTrustedWithOptions(null) -> {ax}"),
        );

        // Preflight first so we don't keep popping prompts after first deny.
        let pre = unsafe { CGPreflightScreenCaptureAccess() };
        log(LOG_TAG, &format!("CGPreflightScreenCaptureAccess -> {pre}"));

        // Call request even when preflight is false — first time triggers
        // the dialog; subsequent calls are silent no-ops but still register
        // the binary in the Screen Recording list if it's not there yet.
        let sc = unsafe { CGRequestScreenCaptureAccess() };
        log(LOG_TAG, &format!("CGRequestScreenCaptureAccess -> {sc}"));

        // Microphone: fire a request from THIS process so TCC attributes
        // it to the .app bundle, not the SDK helper. First call shows the
        // prompt; subsequent calls are silent no-ops once a decision is
        // cached. Also covers System Audio on macOS Sequoia+, where the
        // two are consolidated under "Screen & System Audio Recording".
        request_microphone_access();

        Ok((ax, sc))
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("permissions_force_native_register is macOS-only".into())
    }
}

/// Maps a permission name (kebab-case, matches `RecallPermission` enum
/// serialization) to the System Settings URL.
///
/// Returns `None` for unknown permissions — the caller defaults to opening
/// the top-level Privacy & Security pane.
fn settings_url(permission: &str) -> Option<&'static str> {
    match permission {
        "accessibility" => Some(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        ),
        "screen-capture" => Some(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
        ),
        "microphone" => Some(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
        ),
        // system-audio doesn't have its own pane on modern macOS — the user
        // grants it as part of Screen Recording. Send them there.
        "system-audio" => Some(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
        ),
        "full-disk-access" => Some(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
        ),
        _ => None,
    }
}

/// Open System Settings to the privacy pane for `permission`.
///
/// `permission` is the kebab-case form (`screen-capture`, `microphone`, etc.).
/// Unknown values open the top-level Privacy & Security pane as a fallback.
///
/// Returns `Ok(())` on successful spawn even if macOS later fails to focus
/// the pane — the `open` command is fire-and-forget.
#[tauri::command]
pub fn permissions_open_settings(permission: String) -> Result<(), String> {
    let url = settings_url(&permission).unwrap_or(
        "x-apple.systempreferences:com.apple.preference.security?Privacy",
    );

    log(
        LOG_TAG,
        &format!("permissions_open_settings: opening {url} for {permission}"),
    );

    Command::new("open")
        .arg(url)
        .spawn()
        .map_err(|e| format!("open System Settings failed: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_url_covers_all_recall_permissions() {
        for perm in &[
            "accessibility",
            "screen-capture",
            "microphone",
            "system-audio",
            "full-disk-access",
        ] {
            assert!(settings_url(perm).is_some(), "missing url for {perm}");
        }
    }

    #[test]
    fn settings_url_returns_none_for_unknown() {
        assert!(settings_url("input-monitoring").is_none());
        assert!(settings_url("").is_none());
    }

    #[test]
    fn settings_url_uses_screen_capture_pane_for_system_audio() {
        // macOS Sequoia+ ties system-audio capture to the Screen Recording
        // permission — keep them in lock-step so the UI doesn't dead-end
        // the user on a non-existent pane.
        assert_eq!(
            settings_url("system-audio"),
            settings_url("screen-capture"),
        );
    }
}
