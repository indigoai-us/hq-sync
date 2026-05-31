//! Clickable meeting-detected notifications via `UNUserNotificationCenter`.
//!
//! On macOS Sequoia the legacy `NSUserNotification` / `mac-notification-sys`
//! deliver path is permanently denied once any code touches
//! `UNUserNotificationCenter` (which the permission probe in `notifications.rs`
//! does at launch). The app falls back to `osascript display notification`,
//! which renders fine but **cannot carry a click callback**. To get a banner
//! the user can click — to open the desktop-alt "HQ Meetings" window — we have
//! to deliver through `UNUserNotificationCenter` *and* install a
//! `UNUserNotificationCenterDelegate` to intercept the click.
//!
//! This module is compiled empty off macOS (inner `#![cfg]`), mirroring the
//! `dm_mqtt` pattern of an unconditional `pub mod` declaration plus gated use.
#![cfg(target_os = "macos")]

use std::sync::OnceLock;

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{class, define_class, msg_send, AnyThread};
use tauri::AppHandle;

/// AppHandle captured at delegate registration so a *cold* click (no
/// desktop-alt window open, hence no frontend `notification:meeting-action`
/// listener) can still open the window straight from Rust.
static DELEGATE_APP: OnceLock<AppHandle> = OnceLock::new();
/// Guards against re-installing the delegate (the center keeps only a weak
/// reference, so we leak exactly one delegate for the process lifetime).
static DELEGATE_REGISTERED: OnceLock<()> = OnceLock::new();

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HQMeetingNotificationDelegate"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    impl NotificationDelegate {
        /// Show the banner even when the app is frontmost.
        /// Options bitmask: banner(16) | list(8) | sound(2) = 26.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: *mut AnyObject,
            _notification: *mut AnyObject,
            completion: &Block<dyn Fn(usize)>,
        ) {
            completion.call((26usize,));
        }

        /// Body-click (default action) → open the desktop-alt Meetings window.
        /// Arrives on the main thread.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: *mut AnyObject,
            _response: *mut AnyObject,
            completion: &Block<dyn Fn()>,
        ) {
            if let Some(app) = DELEGATE_APP.get() {
                let pending = crate::tray::get_prompt_pending().saturating_sub(1);
                crate::tray::set_prompt_badge(app, pending);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    // Land on the Meetings screen — this banner is a
                    // meeting-detected prompt, so the click should surface the
                    // detected meeting (with its Record control), not the
                    // default Sync screen.
                    if let Err(e) = crate::commands::desktop_alt::open_desktop_alt_window_inner(
                        app,
                        Some("meetings"),
                    )
                    .await
                    {
                        crate::util::logfile::log(
                            "meetings",
                            &format!("UN didReceive: open desktop-alt failed: {e}"),
                        );
                    }
                });
            }
            completion.call(());
        }
    }
);

impl NotificationDelegate {
    fn new() -> Retained<Self> {
        // No ivars and no overridden `init`, so a plain `init` on the freshly
        // allocated instance dispatches up to `NSObject.init`. (A `super(this)`
        // init would require a `PartialInit` receiver via `set_ivars`, which
        // only exists for classes that declare ivars.)
        unsafe { msg_send![Self::alloc(), init] }
    }
}

/// Build an autoreleased `NSString` from a Rust `&str`.
unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let cstr = std::ffi::CString::new(s).unwrap_or_default();
    msg_send![class!(NSString), stringWithUTF8String: cstr.as_ptr()]
}

/// `UNUserNotificationCenter` is only valid inside a real `.app` bundle; calling
/// `currentNotificationCenter` from a bare binary throws. Guard every entry.
fn is_bundled() -> bool {
    unsafe {
        let main: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        if main.is_null() {
            return false;
        }
        let ident: *mut AnyObject = msg_send![main, bundleIdentifier];
        !ident.is_null()
    }
}

/// Install the notification-center delegate once, and stash the AppHandle.
/// Called from `main.rs` `.setup()` (macOS-gated). Safe to call repeatedly.
pub fn register_delegate(app: &AppHandle) {
    let _ = DELEGATE_APP.set(app.clone());
    if DELEGATE_REGISTERED.get().is_some() || !is_bundled() {
        return;
    }
    unsafe {
        let center: *mut AnyObject =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        if center.is_null() {
            return;
        }
        let delegate: Retained<NotificationDelegate> = NotificationDelegate::new();
        let _: () = msg_send![center, setDelegate: &*delegate];
        let _ = DELEGATE_REGISTERED.set(());
        // The center holds only a weak reference to its delegate, so we must
        // keep ours alive for the whole process. Leaking one object is the
        // intended lifetime here.
        std::mem::forget(delegate);
    }
}

/// Deliver a clickable meeting-detected banner. No-op (returns) off a bundle.
/// `window_id` / `platform` ride along in `userInfo` for the frontend handler
/// (warm-click path); the cold-click path opens the window from the delegate.
pub fn deliver_clickable(title: &str, body: &str, window_id: &str, platform: &str) {
    if !is_bundled() {
        return;
    }
    let fired = objc2::rc::autoreleasepool(|_pool| unsafe {
        // `new` = owned (+1); everything else here is autoreleased.
        let content: Retained<AnyObject> =
            msg_send![class!(UNMutableNotificationContent), new];
        let _: () = msg_send![&*content, setTitle: ns_string(title)];
        let _: () = msg_send![&*content, setBody: ns_string(body)];
        let sound: *mut AnyObject = msg_send![class!(UNNotificationSound), defaultSound];
        if !sound.is_null() {
            let _: () = msg_send![&*content, setSound: sound];
        }

        let user_info: *mut AnyObject = msg_send![class!(NSMutableDictionary), dictionary];
        if !user_info.is_null() {
            let _: () =
                msg_send![user_info, setObject: ns_string(window_id), forKey: ns_string("windowId")];
            let _: () =
                msg_send![user_info, setObject: ns_string(platform), forKey: ns_string("platform")];
            let _: () = msg_send![&*content, setUserInfo: user_info];
        }

        let identifier = ns_string(&format!("hq-meeting-{window_id}"));
        let trigger: *mut AnyObject = std::ptr::null_mut();
        let request: *mut AnyObject = msg_send![
            class!(UNNotificationRequest),
            requestWithIdentifier: identifier,
            content: &*content,
            trigger: trigger
        ];
        if request.is_null() {
            return false;
        }

        let center: *mut AnyObject =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        if center.is_null() {
            return false;
        }
        // `withCompletionHandler:` expects a block ("@?"); pass an empty one
        // rather than null so objc2's encoding check is satisfied.
        let completion = RcBlock::new(|_err: *mut AnyObject| {});
        let _: () =
            msg_send![center, addNotificationRequest: request, withCompletionHandler: &*completion];
        true
    });
    crate::util::logfile::log(
        "meetings",
        if fired {
            "UN clickable notification fired"
        } else {
            "UN clickable notification: setup failed"
        },
    );
}
