//! Permissions commands — open System Settings to specific Privacy panes,
//! and read the current TCC status for the meeting-detect permission set.
//!
//! macOS supports the `x-apple.systempreferences:` URL scheme for deep-linking
//! into individual Privacy & Security panes. The mapping below is the public
//! contract documented by Apple plus a few panes (`Privacy_ScreenCapture`,
//! `Privacy_ListenEvent`) that have shipped in modern macOS releases.
//!
//! Recall Desktop SDK requires 5 macOS permissions: accessibility,
//! screen-capture, microphone, system-audio (tied to screen-capture in
//! Sequoia+), and full-disk-access. The Svelte UI deep-links each to the
//! right pane so the user goes straight there. `meetings_permissions_state`
//! returns the current TCC status for each so Settings + the wizard window
//! can render an at-a-glance "Enabled / Setup needed" pill.

use std::process::Command;

use serde::Serialize;

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

    /// Returns the current AVAuthorizationStatus for the given media type
    /// WITHOUT prompting. Maps to:
    ///   0 = NotDetermined (we report as `Prompt`)
    ///   1 = Restricted    (we report as `Denied` — user can't grant)
    ///   2 = Denied
    ///   3 = Authorized
    /// Safe to call from any thread; takes no Rust-owned references.
    pub fn authorization_status_for_audio() -> i64 {
        use objc2::{class, msg_send, runtime::{AnyClass, AnyObject}};
        unsafe {
            let ns_string_cls: &AnyClass = class!(NSString);
            let audio_type: *mut AnyObject = msg_send![
                ns_string_cls,
                stringWithUTF8String: b"soun\0".as_ptr() as *const i8
            ];
            if audio_type.is_null() {
                return 0; // NotDetermined as a safe default
            }
            let av_cls: &AnyClass = class!(AVCaptureDevice);
            let status: i64 = msg_send![av_cls, authorizationStatusForMediaType: audio_type];
            status
        }
    }
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

// ─────────────────────────────────────────────────────────────────────────────
// Permission status enumeration
// ─────────────────────────────────────────────────────────────────────────────

/// Status of a single TCC permission, as surfaced to the frontend.
///
/// `unknown` is the carve-out for permissions where macOS has no public API
/// to read the current state (Full Disk Access is the live case — Apple
/// only exposes a private SPI, and we'd rather show "Open Settings to
/// check" than guess).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PermStatus {
    /// User has granted the permission.
    Granted,
    /// User has denied the permission. macOS will NOT re-prompt — the only
    /// path back is the System Settings pane (the wizard's CTA).
    Denied,
    /// User has not yet been asked. The native macOS prompt will fire the
    /// first time we call the corresponding API; this is the lightest-touch
    /// state to be in.
    Prompt,
    /// No programmatic check is available for this permission. The
    /// frontend should render "Open Settings to check" rather than a
    /// false-positive Granted / Denied pill.
    Unknown,
}

/// At-a-glance status of every permission the meeting-detect-notify feature
/// needs. The Settings row collapses the four required permissions to a
/// single Enabled/Setup-needed pill; the wizard window renders one row per
/// permission with this struct's fields.
///
/// `system_audio` is intentionally aliased to `screen_capture` on macOS
/// Sequoia+: the system collapses both grants into the Screen & System
/// Audio Recording pane. Pre-Sequoia they were distinct; we report the
/// best-available status from either source.
///
/// `all_required_granted` is true iff `accessibility`, `screen_capture`, and
/// `microphone` are all `Granted`. Full Disk Access is reported but NOT
/// gating — the SDK gracefully degrades without it for the meeting-detect
/// path (FDA is only needed for some on-disk capture modes we don't use).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPermissionsState {
    pub accessibility: PermStatus,
    pub screen_capture: PermStatus,
    pub microphone: PermStatus,
    pub system_audio: PermStatus,
    pub full_disk_access: PermStatus,
    pub all_required_granted: bool,
}

#[cfg(target_os = "macos")]
fn microphone_status() -> PermStatus {
    match macos::authorization_status_for_audio() {
        // 0 = NotDetermined → we haven't asked yet.
        0 => PermStatus::Prompt,
        // 1 = Restricted (parental controls / MDM) — user cannot grant.
        // 2 = Denied — user denied, only recoverable via System Settings.
        1 | 2 => PermStatus::Denied,
        // 3 = Authorized.
        3 => PermStatus::Granted,
        // Future status codes — treat as unknown so the wizard offers
        // "Open Settings" rather than asserting a bogus pill.
        _ => PermStatus::Unknown,
    }
}

/// Read all five TCC statuses WITHOUT prompting. Idempotent and safe to
/// call on every Settings open.
///
/// macOS specifics:
/// - Accessibility: `AXIsProcessTrusted()` returns the current state.
///   It does NOT prompt. (`AXIsProcessTrustedWithOptions(..prompt: true)`
///   is the prompting variant; we use the prompt-less form here.)
/// - Screen Recording: `CGPreflightScreenCaptureAccess()` is the
///   prompt-less read. Returns the cached TCC verdict; `prompt` is not
///   distinguishable from `denied` at the API level, so we report
///   `Granted` / `Denied` only (the frontend can still surface "Enable"
///   for both cases — the deep-linked System Settings pane shows the
///   right state once opened).
/// - Microphone: `AVCaptureDevice.authorizationStatusForMediaType:` returns
///   the four-state enum (NotDetermined/Restricted/Denied/Authorized).
/// - System Audio: aliased to Screen Recording on macOS 13+ — the user
///   grants both in one pane.
/// - Full Disk Access: no public read API. Always reports `Unknown`; the
///   wizard surfaces an "Open Settings" CTA so the user can verify
///   manually.
#[tauri::command]
pub fn meetings_permissions_state() -> Result<MeetingPermissionsState, String> {
    #[cfg(target_os = "macos")]
    {
        use macos::*;

        // SAFETY: AXIsProcessTrusted and CGPreflightScreenCaptureAccess
        // are documented thread-safe C functions; they take no Rust
        // references and never panic on the FFI boundary.
        let ax_trusted = unsafe { AXIsProcessTrusted() };
        let sc_authorized = unsafe { CGPreflightScreenCaptureAccess() };

        let accessibility = if ax_trusted {
            PermStatus::Granted
        } else {
            // CGPreflightScreenCaptureAccess doesn't distinguish prompt
            // from denied. AXIsProcessTrusted has the same limitation —
            // the binary returns `false` for both "not yet asked" and
            // "user said no". Report Denied so the wizard shows "Open
            // Settings" (which works for both cases); a true Prompt
            // state would be surfaced by the first SDK call anyway.
            PermStatus::Denied
        };
        let screen_capture = if sc_authorized {
            PermStatus::Granted
        } else {
            PermStatus::Denied
        };
        // Sequoia+: System Audio is granted in the Screen Recording pane.
        // On older macOS, they had separate panes — but we ship the
        // meeting-detect feature only on modern systems, so the alias is
        // safe in practice.
        let system_audio = screen_capture;

        let microphone = microphone_status();

        // No public API for FDA; the wizard surfaces an "Open Settings"
        // CTA so the user can verify manually.
        let full_disk_access = PermStatus::Unknown;

        let all_required_granted = accessibility == PermStatus::Granted
            && screen_capture == PermStatus::Granted
            && microphone == PermStatus::Granted;

        log(
            LOG_TAG,
            &format!(
                "meetings_permissions_state: ax={:?} sc={:?} mic={:?} sa={:?} fda={:?} all_required={}",
                accessibility,
                screen_capture,
                microphone,
                system_audio,
                full_disk_access,
                all_required_granted,
            ),
        );

        Ok(MeetingPermissionsState {
            accessibility,
            screen_capture,
            microphone,
            system_audio,
            full_disk_access,
            all_required_granted,
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Non-macOS: feature is macOS-only; report everything as Unknown
        // so the UI hides the row entirely (frontend gates on
        // all_required_granted = false AND any non-Granted status).
        Ok(MeetingPermissionsState {
            accessibility: PermStatus::Unknown,
            screen_capture: PermStatus::Unknown,
            microphone: PermStatus::Unknown,
            system_audio: PermStatus::Unknown,
            full_disk_access: PermStatus::Unknown,
            all_required_granted: false,
        })
    }
}

/// Open (or focus) the meeting-permissions wizard window. Same handshake
/// pattern as `open_meetings_window` / `open_share_detail`: if the window
/// already exists, just bring it to the front; otherwise build it fresh.
///
/// The wizard is a single-page Svelte view at the `meeting-permissions`
/// window label. It calls `meetings_permissions_state` on mount + window
/// focus, and surfaces an "Open Settings" CTA per permission via
/// `permissions_open_settings`. No event handshake needed — the view
/// self-fetches.
#[tauri::command]
pub async fn open_meeting_permissions_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    const LABEL: &str = "meeting-permissions";

    if let Some(window) = app.get_webview_window(LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Re-use the bundled HQ app icon so the wizard window has the right
    // dock / Cmd-Tab / window-switcher representation. Same comment as
    // `open_meetings_window` — composite badge skipped for legibility-at-
    // small-sizes reasons.
    const HQ_ICON_PNG: &[u8] = include_bytes!("../../icons/128x128@2x.png");
    let icon = tauri::image::Image::from_bytes(HQ_ICON_PNG)
        .map_err(|e| format!("load window icon: {e}"))?;

    tauri::WebviewWindowBuilder::new(
        &app,
        LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Meeting Permissions")
    // Sized so all four permission rows + footer fit without the inner
    // scrollbar appearing (`.perm-list` overflow:auto). Width gives the
    // `.perm-reason` text a 480px column so the SDK rationale doesn't wrap
    // onto a third line per row, which made the wizard look cramped.
    .inner_size(640.0, 700.0)
    .min_inner_size(560.0, 600.0)
    .resizable(true)
    .decorations(true)
    .icon(icon)
    .map_err(|e| format!("attach window icon: {e}"))?
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
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
