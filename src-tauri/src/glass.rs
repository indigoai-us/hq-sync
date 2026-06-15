//! Native macOS "Liquid Glass" window backing.
//!
//! On macOS 26 (Tahoe) the desktop window's background becomes a real
//! `NSGlassEffectView` — Apple's Liquid Glass material — inserted *behind* the
//! (transparent) WKWebView so the window itself reads as live glass over the
//! desktop. On older macOS, where that class does not exist, we fall back to
//! the same `NSVisualEffectView` frosted vibrancy the menubar popover already
//! uses (`main.rs::apply_liquid_glass`), so every supported OS still gets a
//! translucent glass window rather than a see-through hole.
//!
//! Why a backing view instead of styling the webview: Liquid Glass is a native
//! `NSView` effect. It can sit behind the transparent webview (sampling the
//! desktop and windows behind ours) but it cannot refract the webview's own DOM
//! content — so in-window panels get matched translucent styling in CSS, while
//! the *window* gets the genuine material here.
//!
//! AppKit is main-thread-only; callers MUST invoke this from
//! `app.run_on_main_thread`. Mirrors the raw-objc2 idiom in
//! `commands/banner.rs` (no objc2-app-kit dependency for the messaging itself —
//! only `objc2-core-foundation` for the `CGRect` returned by `-bounds`).

/// Insert the Liquid Glass (or vibrancy fallback) backing view on the given
/// window. Idempotent enough for our use — only ever called once, right after a
/// fresh window build. No-op on non-macOS targets.
#[cfg(target_os = "macos")]
pub fn apply_liquid_glass_window(window: &tauri::WebviewWindow) {
    use crate::util::logfile::log;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_core_foundation::CGRect;

    const LOG_TAG: &str = "ui";

    let ns_win = match window.ns_window() {
        Ok(ptr) => ptr as *mut AnyObject,
        Err(e) => {
            log(LOG_TAG, &format!("liquid-glass: ns_window() unavailable: {e}"));
            return;
        }
    };

    // NSGlassEffectView only exists on macOS 26+. Resolve it at runtime so the
    // same binary still links and runs on older macOS, where we drop to the
    // vibrancy fallback below.
    let glass_class = AnyClass::get(c"NSGlassEffectView");

    // SAFETY: invoked on the main thread (run_on_main_thread); every selector
    // here is a standard AppKit message sent to a live object, and the pointers
    // are validated non-null before use.
    unsafe {
        let content: *mut AnyObject = msg_send![ns_win, contentView];
        if content.is_null() {
            log(LOG_TAG, "liquid-glass: window has no contentView");
            return;
        }

        if let Some(class) = glass_class {
            let bounds: CGRect = msg_send![content, bounds];
            let glass: *mut AnyObject = msg_send![class, alloc];
            let glass: *mut AnyObject = msg_send![glass, initWithFrame: bounds];
            if glass.is_null() {
                log(LOG_TAG, "liquid-glass: NSGlassEffectView init returned nil");
                return;
            }
            // Fill the content view and track it as the window resizes:
            // NSViewWidthSizable (1<<1) | NSViewHeightSizable (1<<4).
            let autoresize: usize = (1 << 1) | (1 << 4);
            let _: () = msg_send![glass, setAutoresizingMask: autoresize];
            // Square corners — the macOS window frame already rounds the content.
            let _: () = msg_send![glass, setCornerRadius: 0.0_f64];
            // Insert at the very back (NSWindowBelow) so the webview and all its
            // content paint over the glass.
            let below: isize = -1;
            let null_view: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![
                content,
                addSubview: glass,
                positioned: below,
                relativeTo: null_view
            ];
            log(LOG_TAG, "liquid-glass: NSGlassEffectView applied (macOS 26+)");
            return;
        }
    }

    // Pre-Tahoe fallback: NSVisualEffectView frosted vibrancy. UnderWindowBackground
    // is the calmest large-surface material (the popover uses the brighter
    // Popover material for its small card).
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
    match apply_vibrancy(
        window,
        NSVisualEffectMaterial::UnderWindowBackground,
        Some(NSVisualEffectState::Active),
        None,
    ) {
        Ok(()) => log(
            LOG_TAG,
            "liquid-glass: vibrancy fallback applied (UnderWindowBackground)",
        ),
        Err(e) => log(LOG_TAG, &format!("liquid-glass: vibrancy fallback FAILED: {e}")),
    }
}
