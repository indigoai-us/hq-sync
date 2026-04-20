/// Force-terminate any stuck NSOpenPanel / NSSavePanel modal session.
///
/// `rfd::AsyncFileDialog::pick_folder()` opens an application-modal
/// NSOpenPanel via `[NSApp runModalForWindow:]`. The Rust future
/// resolves only when the panel's completion handler fires — which
/// happens when the modal session ends with a response code.
///
/// Observed failure mode on the menubar popover: clicking outside the
/// panel leaves it in `NSApp.windows` with `isVisible=false` but the
/// rfd future still pending. `[panel close]` on the zombied panel is
/// a no-op — the completion handler never fires, the future hangs,
/// and the next `pick_folder()` trips AppKit's "modal already active"
/// guard and produces NSBeep on every subsequent click.
///
/// Fix: send `cancel:` to every panel (fires the Cancel IBAction which
/// ends the modal session with NSModalResponseCancel; rfd resolves the
/// prior future with `None`). Then call `[NSApp abortModal]` if AppKit
/// still reports a live modalWindow — belt-and-suspenders for any
/// session that didn't have a panel attached to cancel.
///
/// Must run on the main thread — AppKit is not thread-safe. Callers
/// use `AppHandle::run_on_main_thread` to guarantee this.
#[cfg(target_os = "macos")]
fn close_existing_file_panels() {
    use objc2::{class, msg_send, runtime::AnyObject};

    unsafe {
        let app_cls = class!(NSApplication);
        let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
        if app.is_null() {
            return;
        }
        let windows: *mut AnyObject = msg_send![app, windows];
        if windows.is_null() {
            return;
        }
        let count: usize = msg_send![windows, count];

        // Snapshot handles first. `cancel:` / `close` mutate the
        // `windows` array — iterating it live would be undefined.
        let mut handles: Vec<*mut AnyObject> = Vec::with_capacity(count);
        for i in 0..count {
            let w: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            handles.push(w);
        }

        let nil: *mut AnyObject = std::ptr::null_mut();
        for window in handles {
            if window.is_null() {
                continue;
            }
            let class_name: *mut AnyObject = msg_send![window, className];
            if class_name.is_null() {
                continue;
            }
            let utf8: *const std::os::raw::c_char = msg_send![class_name, UTF8String];
            if utf8.is_null() {
                continue;
            }
            let name = std::ffi::CStr::from_ptr(utf8).to_string_lossy();
            if name == "NSOpenPanel" || name == "NSSavePanel" {
                let _: () = msg_send![window, cancel: nil];
                let _: () = msg_send![window, close];
            }
        }

        let modal_window: *mut AnyObject = msg_send![app, modalWindow];
        if !modal_window.is_null() {
            let _: () = msg_send![app, abortModal];
        }
    }
}

/// Open a native macOS folder picker dialog.
/// Returns the selected path, or None if the user cancelled.
///
/// Behaviour:
/// - Holds a `ModalGuard` for the lifetime of the dialog. Without it
///   the NSOpenPanel would steal key-window status from the popover,
///   which triggers the `Focused(false)` hide handler in `tray.rs` —
///   and once the popover hides, macOS unparents and immediately
///   dismisses the open panel.
/// - Before invoking rfd, closes any existing NSOpenPanel/NSSavePanel
///   so repeated Change clicks don't stack panels. The prior rfd
///   future resolves with `None` (like the user hit Cancel) and the
///   new picker opens cleanly.
#[tauri::command]
pub async fn pick_folder(#[allow(unused_variables)] app: tauri::AppHandle) -> Result<Option<String>, String> {
    let _guard = crate::tray::ModalGuard::new();

    #[cfg(target_os = "macos")]
    {
        // run_on_main_thread is fire-and-forget; we don't await it
        // because its callback runs to completion before the NSApp's
        // next event-loop iteration — which is when the new rfd
        // panel would be shown anyway.
        let _ = app.run_on_main_thread(close_existing_file_panels);
    }

    let result = rfd::AsyncFileDialog::new()
        .set_title("Choose HQ Folder")
        .pick_folder()
        .await;

    Ok(result.map(|handle| handle.path().to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_dialog_builder_compiles() {
        // Verify rfd API is available and the builder pattern works.
        // We can't actually open a dialog in tests, but we can confirm
        // the builder chain compiles correctly.
        let _builder = rfd::AsyncFileDialog::new().set_title("Choose HQ Folder");
    }
}
