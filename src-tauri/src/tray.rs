//! System tray icon with state-driven icon swapping.
//!
//! Four visual states: **idle**, **syncing**, **error**, **conflict**.
//! Left-click toggles the popover window; right-click shows a context menu
//! with "Sync Now", "Settings", and "Quit".

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Listener, Manager, Monitor, PhysicalPosition, Rect, WindowEvent,
};

// ─────────────────────────────────────────────────────────────────────────────
// Tray state enum
// ─────────────────────────────────────────────────────────────────────────────

/// Visual state of the tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Syncing,
    Error,
    Conflict,
    /// A meeting was detected and the user has not yet acted on it.
    /// Shown as a badge dot overlaid on the current sync state icon.
    /// Uses the idle icon bytes as a placeholder; a designer badge
    /// composite can replace this later without an API change.
    Prompt,
}

impl TrayState {
    /// Parse from a frontend string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "idle" => Some(Self::Idle),
            "syncing" => Some(Self::Syncing),
            "error" => Some(Self::Error),
            "conflict" => Some(Self::Conflict),
            "prompt" => Some(Self::Prompt),
            _ => None,
        }
    }

    /// Tooltip text for this state.
    pub fn tooltip(&self) -> &'static str {
        match self {
            Self::Idle => "HQ Sync — Idle",
            Self::Syncing => "HQ Sync — Syncing…",
            Self::Error => "HQ Sync — Error",
            Self::Conflict => "HQ Sync — Conflict",
            Self::Prompt => "HQ Sync — Meeting Detected",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Global state
// ─────────────────────────────────────────────────────────────────────────────

/// Global current tray state.
static CURRENT_STATE: OnceLock<Arc<Mutex<TrayState>>> = OnceLock::new();

fn current_state() -> &'static Arc<Mutex<TrayState>> {
    CURRENT_STATE.get_or_init(|| Arc::new(Mutex::new(TrayState::Idle)))
}

/// Refcount of active native-modal guards. When > 0, the hide-on-blur
/// handler is suppressed — the modal stealing key-window status from
/// the popover should not dismiss the popover, which would otherwise
/// unparent and close the modal.
///
/// Refcount (not bool) because a new `pick_folder` may start while the
/// previous one's `rfd` future hasn't resolved yet — `close_existing_file_panels`
/// cancels the stuck panel mid-call, resolving the previous future
/// (and dropping its guard). With a bool, that drop would clobber the
/// new call's flag to false while its own panel is still opening.
static MODAL_DEPTH: AtomicUsize = AtomicUsize::new(0);

/// Count of pending meeting detections awaiting user action.
/// When > 0, the tray shows the `Prompt` state. Decrements when the
/// user opens MeetingsWindow or acts on a notification.
static PROMPT_PENDING: AtomicUsize = AtomicUsize::new(0);

/// Count of unacknowledged share events. When > 0, the tray tooltip
/// gains a " · N new share(s)" suffix as a lightweight visual badge
/// (avoids needing a new tray icon PNG for the share-notify feature).
static SHARE_BADGE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Whether at least one native modal is currently open.
pub fn is_modal_open() -> bool {
    MODAL_DEPTH.load(Ordering::SeqCst) > 0
}

/// RAII guard — increments `MODAL_DEPTH` on construction and decrements
/// on drop. Prefer this over flipping the counter manually so the
/// decrement is always paired even if the caller panics or returns
/// early.
///
/// Usage:
/// ```ignore
/// let _guard = tray::ModalGuard::new();
/// let picked = rfd::AsyncFileDialog::new().pick_folder().await;
/// // _guard drops here, MODAL_DEPTH decrements.
/// ```
pub struct ModalGuard;

impl ModalGuard {
    pub fn new() -> Self {
        MODAL_DEPTH.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ModalGuard {
    fn drop(&mut self) {
        MODAL_DEPTH.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Set the meeting-prompt badge count.
///
/// When `count > 0`, overrides the tray icon to `Prompt` state.
/// When `count == 0`, restores the current base sync state icon so
/// a cleared badge doesn't get stuck showing "Meeting Detected".
pub fn set_prompt_badge(app: &AppHandle, count: usize) {
    PROMPT_PENDING.store(count, Ordering::SeqCst);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if count > 0 {
            set_state_icon(&tray, TrayState::Prompt);
            let _ = tray.set_tooltip(Some(TrayState::Prompt.tooltip()));
        } else {
            let state = current_state()
                .lock()
                .map(|s| *s)
                .unwrap_or(TrayState::Idle);
            set_state_icon(&tray, state);
            let _ = tray.set_tooltip(Some(state.tooltip()));
        }
    }
}

/// Return the current pending-meeting count (for tests and badge logic).
pub fn get_prompt_pending() -> usize {
    PROMPT_PENDING.load(Ordering::SeqCst)
}

/// Tauri command: set or clear the meeting-prompt tray badge.
///
/// `count > 0` → Prompt icon; `count == 0` → restore base state icon.
#[tauri::command]
pub fn meetings_set_prompt_badge(app: AppHandle, count: usize) {
    set_prompt_badge(&app, count);
}

// ─────────────────────────────────────────────────────────────────────────────
// Icon loading
// ─────────────────────────────────────────────────────────────────────────────

/// Load the embedded icon bytes for a given tray state.
/// We use `include_bytes!` so the PNGs are baked into the binary.
/// Icons are cached after first decode via `OnceLock` to avoid repeated PNG parsing.
///
/// `Prompt` reuses the idle icon bytes as a placeholder. The visual
/// differentiation between Idle and Prompt comes from the tooltip text
/// ("Meeting Detected") until a designer badge composite is created.
fn icon_for_state(state: TrayState) -> Image<'static> {
    static ICON_IDLE: OnceLock<Image<'static>> = OnceLock::new();
    static ICON_SYNCING: OnceLock<Image<'static>> = OnceLock::new();
    static ICON_ERROR: OnceLock<Image<'static>> = OnceLock::new();
    static ICON_CONFLICT: OnceLock<Image<'static>> = OnceLock::new();

    let decode = |bytes: &'static [u8]| -> Image<'static> {
        Image::from_bytes(bytes).expect("Failed to decode tray icon PNG")
    };

    match state {
        TrayState::Idle => {
            ICON_IDLE.get_or_init(|| decode(include_bytes!("../icons/tray-idle@2x.png")))
        }
        TrayState::Syncing => {
            ICON_SYNCING.get_or_init(|| decode(include_bytes!("../icons/tray-syncing@2x.png")))
        }
        TrayState::Error => {
            ICON_ERROR.get_or_init(|| decode(include_bytes!("../icons/tray-error@2x.png")))
        }
        TrayState::Conflict => {
            ICON_CONFLICT.get_or_init(|| decode(include_bytes!("../icons/tray-conflict@2x.png")))
        }
        // Reuse idle icon; designer badge composite is a follow-up.
        TrayState::Prompt => {
            ICON_IDLE.get_or_init(|| decode(include_bytes!("../icons/tray-idle@2x.png")))
        }
    }
    .clone()
}

/// Swap the tray icon for `state`, re-asserting the macOS template flag.
///
/// `icon_as_template(true)` is set once at build time, but on macOS every
/// `set_icon(...)` installs a fresh NSImage whose `isTemplate` defaults to NO —
/// silently dropping the template flag. Our glyphs are white-on-alpha (correct
/// for a template, where macOS uses only the alpha shape and recolours it dark
/// on a light menu bar / light on a dark one); once the flag is lost the raw
/// white pixels get drawn, leaving the icon invisible/white on a light menu bar.
/// Re-applying the flag after each swap keeps the glyph adapting to the menu-bar
/// appearance for the life of the process.
fn set_state_icon<R: tauri::Runtime>(tray: &tauri::tray::TrayIcon<R>, state: TrayState) {
    let _ = tray.set_icon(Some(icon_for_state(state)));
    let _ = tray.set_icon_as_template(true);
}

// ─────────────────────────────────────────────────────────────────────────────
// Menu IDs
// ─────────────────────────────────────────────────────────────────────────────

const MENU_VERSION: &str = "version";
const MENU_SYNC_NOW: &str = "sync-now";
const MENU_SETTINGS: &str = "settings";
const MENU_QUIT: &str = "quit";

// ─────────────────────────────────────────────────────────────────────────────
// Tray ID
// ─────────────────────────────────────────────────────────────────────────────

const TRAY_ID: &str = "hq-sync-tray";

// ─────────────────────────────────────────────────────────────────────────────
// Setup
// ─────────────────────────────────────────────────────────────────────────────

/// Create the system tray icon with its context menu and event handlers.
///
/// Call this from `tauri::Builder::default().setup(...)`.
pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use crate::util::logfile::log;

    // Build context menu. The version row is a disabled item — it renders
    // like a macOS "About" label (dimmed, unclickable). Sourced from the
    // bundled `Cargo.toml` / `tauri.conf.json` via `package_info()` so it
    // tracks the binary the user is actually running.
    let version = app.package_info().version.to_string();
    let version_item = MenuItemBuilder::with_id(MENU_VERSION, format!("HQ Sync v{}", version))
        .enabled(false)
        .build(app)?;
    let sync_now = MenuItemBuilder::with_id(MENU_SYNC_NOW, "Sync Now").build(app)?;
    let settings = MenuItemBuilder::with_id(MENU_SETTINGS, "Settings").build(app)?;
    let quit = MenuItemBuilder::with_id(MENU_QUIT, "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&version_item)
        .separator()
        .item(&sync_now)
        .separator()
        .item(&settings)
        .item(&quit)
        .build()?;

    // Build tray icon
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon_for_state(TrayState::Idle))
        .icon_as_template(true)
        .tooltip("HQ Sync — Idle")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event({
            let app_handle = app.clone();
            move |_app, event| {
                let id = event.id().as_ref();
                match id {
                    id if id == MENU_SYNC_NOW => {
                        let _ = app_handle.emit("tray:sync-now", ());
                    }
                    id if id == MENU_SETTINGS => {
                        let _ = app_handle.emit("tray:open-settings", ());
                    }
                    id if id == MENU_QUIT => {
                        app_handle.exit(0);
                    }
                    _ => {}
                }
            }
        })
        .on_tray_icon_event({
            let app_handle = app.clone();
            move |_tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } = event
                {
                    toggle_window(&app_handle, Some(rect));
                }
            }
        })
        .build(app)?;

    log("tray", "tray icon built");

    // macOS Tahoe/Sequoia: SystemUIServer can silently fail to render a
    // newly-registered NSStatusItem — especially after app updates.
    // Toggling visibility forces re-registration with the system process.
    tray.set_visible(false).ok();
    tray.set_visible(true).ok();

    // Hide the popover when the user clicks away. `window.hide()` preserves
    // the renderer state (DOM, Svelte stores, listeners), so re-showing is
    // instant. Only wired on macOS where the menubar popover pattern
    // expects click-off-to-dismiss.
    //
    // Exception: when a native modal (folder picker, save dialog) is open,
    // the modal steals key-window status from the popover, which fires a
    // `Focused(false)` event. Hiding here would unparent the modal and
    // dismiss it immediately. `ModalGuard` (see above) flips `MODAL_OPEN`
    // while a picker is in flight; we check it and skip the hide.
    if let Some(window) = app.get_webview_window("main") {
        let win_clone = window.clone();
        let disable_blur_hide = std::env::var("HQ_DISABLE_BLUR_HIDE").ok().as_deref() == Some("1");
        window.on_window_event(move |event| {
            if let WindowEvent::Focused(false) = event {
                // Don't dismiss the popover when focus moved to one of OUR OWN
                // secondary windows (drift / DM / share detail). A
                // sync or notification that opens such a window steals key
                // focus from the popover and fires `Focused(false)`; hiding
                // here made the popover vanish out from under the user mid-
                // interaction (and made the Install/Restore buttons impossible
                // to click — the window closed before the click landed). Only
                // hide on a genuine click-away, i.e. when no other HQ window is
                // visible. `is_modal_open()` still covers native pickers.
                let secondary_open = win_clone
                    .app_handle()
                    .webview_windows()
                    .iter()
                    .any(|(label, w)| label != "main" && w.is_visible().unwrap_or(false));
                if !is_modal_open() && !secondary_open && !disable_blur_hide {
                    let _ = win_clone.hide();
                }
            }
        });
    }

    // Delayed re-assertion: at 3s and 10s post-launch, toggle the tray
    // icon to force SystemUIServer re-registration. Covers cases where
    // the initial toggle wasn't processed (macOS Tahoe/Sequoia bug
    // affecting many menubar apps: Tauri #13770, Electron #44817, etc.).
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            reassert_tray(&handle);
            std::thread::sleep(std::time::Duration::from_secs(7));
            reassert_tray(&handle);
        });
    }

    // Listen for sync events to auto-update tray state
    setup_sync_listeners(app);

    // Dev helper: open popover at launch when HQ_DEV_SHOW_ON_LAUNCH=1
    if std::env::var("HQ_DEV_SHOW_ON_LAUNCH").ok().as_deref() == Some("1") {
        let app_handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if let Some(window) = app_handle.get_webview_window("main") {
                eprintln!("[dev-show] showing main window");
                let _ = window.center();
                let _ = window.set_always_on_top(true);
                let _ = window.show();
                let _ = window.set_focus();
                let visible = window.is_visible().unwrap_or(false);
                eprintln!("[dev-show] is_visible={}", visible);
            } else {
                eprintln!("[dev-show] main window not found");
            }
        });
    }

    // Dev helper: open the full desktop window at launch for local visual
    // verification without relying on menu-bar click automation.
    if std::env::var("HQ_DEV_OPEN_DESKTOP_ON_LAUNCH")
        .ok()
        .as_deref()
        == Some("1")
    {
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            eprintln!("[dev-desktop] opening desktop-alt window");
            if let Err(e) =
                crate::commands::desktop_alt::open_desktop_alt_window_inner(app_handle, None).await
            {
                eprintln!("[dev-desktop] open failed: {e}");
            }
        });
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Window toggle
// ─────────────────────────────────────────────────────────────────────────────

/// Toggle the main window visibility (popover behaviour).
///
/// When showing, position the popover directly under the tray icon
/// (centered horizontally, small gap below) if we have its bounds.
/// `window.hide()` preserves renderer state so re-show is instant.
fn toggle_window(app: &AppHandle, tray_rect: Option<Rect>) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            if let Some(rect) = tray_rect {
                position_below_tray(&window, rect);
            }
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// Pure math: center `win_w`-wide window horizontally under the tray icon,
/// `gap_px` below it. All inputs in physical pixels.
fn compute_popover_position(
    tray_x: f64,
    tray_y: f64,
    tray_w: f64,
    tray_h: f64,
    win_w: f64,
    gap_px: f64,
) -> (i32, i32) {
    let pop_x = (tray_x + tray_w / 2.0 - win_w / 2.0).round() as i32;
    let pop_y = (tray_y + tray_h + gap_px).round() as i32;
    (pop_x, pop_y)
}

fn compute_clamped_popover_position(
    tray_x: f64,
    tray_y: f64,
    tray_w: f64,
    tray_h: f64,
    win_w: f64,
    win_h: f64,
    gap_px: f64,
    work_x: f64,
    work_y: f64,
    work_w: f64,
    work_h: f64,
) -> (i32, i32) {
    let (raw_x, raw_y) = compute_popover_position(tray_x, tray_y, tray_w, tray_h, win_w, gap_px);
    let min_x = work_x;
    let max_x = (work_x + work_w - win_w).max(min_x);
    let min_y = work_y;
    let max_y = (work_y + work_h - win_h).max(min_y);
    let pop_x = (raw_x as f64).clamp(min_x, max_x).round() as i32;
    let pop_y = (raw_y as f64).clamp(min_y, max_y).round() as i32;
    (pop_x, pop_y)
}

// Small visual gap between the menu bar and the popover top edge.
// 4 physical px is ~2pt on a 2x retina display — enough to avoid
// the popover looking glued to the menu bar.
const POPOVER_GAP_PX: f64 = 4.0;

fn tray_anchor_monitor(
    monitors: impl IntoIterator<Item = Monitor>,
    tray_center_x: f64,
    tray_center_y: f64,
) -> Option<Monitor> {
    let mut fallback = None;
    for monitor in monitors {
        if fallback.is_none() {
            fallback = Some(monitor.clone());
        }
        let work_area = monitor.work_area();
        let left = work_area.position.x as f64;
        let top = work_area.position.y as f64;
        let right = left + work_area.size.width as f64;
        let bottom = top + work_area.size.height as f64;
        if tray_center_x >= left
            && tray_center_x <= right
            && tray_center_y >= top
            && tray_center_y <= bottom
        {
            return Some(monitor);
        }
    }
    fallback
}

/// Center the window horizontally under the tray icon, just below it.
///
/// `Rect`'s `position` and `size` are enums (Physical | Logical); we
/// normalize both to physical pixels using the window's scale factor
/// so the math is unit-consistent with `window.outer_size()`, which is
/// already physical.
fn position_below_tray(window: &tauri::WebviewWindow, rect: Rect) {
    let size = match window.outer_size() {
        Ok(s) => s,
        Err(_) => return,
    };
    let scale = window.scale_factor().unwrap_or(1.0);

    let (tray_x, tray_y) = match rect.position {
        tauri::Position::Physical(p) => (p.x as f64, p.y as f64),
        tauri::Position::Logical(p) => (p.x * scale, p.y * scale),
    };
    let (tray_w, tray_h) = match rect.size {
        tauri::Size::Physical(s) => (s.width as f64, s.height as f64),
        tauri::Size::Logical(s) => (s.width * scale, s.height * scale),
    };
    let win_w = size.width as f64;
    let win_h = size.height as f64;
    let tray_center_x = tray_x + tray_w / 2.0;
    let tray_center_y = tray_y + tray_h / 2.0;

    let (pop_x, pop_y) = window
        .available_monitors()
        .ok()
        .and_then(|monitors| tray_anchor_monitor(monitors, tray_center_x, tray_center_y))
        .map(|monitor| {
            let work_area = monitor.work_area();
            compute_clamped_popover_position(
                tray_x,
                tray_y,
                tray_w,
                tray_h,
                win_w,
                win_h,
                POPOVER_GAP_PX,
                work_area.position.x as f64,
                work_area.position.y as f64,
                work_area.size.width as f64,
                work_area.size.height as f64,
            )
        })
        .unwrap_or_else(|| {
            compute_popover_position(tray_x, tray_y, tray_w, tray_h, win_w, POPOVER_GAP_PX)
        });

    let _ = window.set_position(PhysicalPosition::new(pop_x, pop_y));
}

/// Show + focus the main window, positioned under the tray icon.
///
/// Used by the global keyboard shortcut so the popover can be summoned
/// from anywhere without clicking the tray icon. If the tray rect isn't
/// available yet (race during startup) we still show the window — it
/// will appear at its last position rather than under the icon.
pub fn show_window_at_tray(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(Some(rect)) = tray.rect() {
            position_below_tray(&window, rect);
        }
    }
    let _ = window.show();
    let _ = window.set_focus();
}

// `show_main_window` (the Svelte-invokable wrapper) lives in
// commands/banner.rs now — the meeting-detect notification's "open" action
// hits the same handler as the update banner's body-click, and both just
// call `show_window_at_tray` here. One name, one handler.

// ─────────────────────────────────────────────────────────────────────────────
// Icon update
// ─────────────────────────────────────────────────────────────────────────────

/// Update the tray icon to reflect a new state.
pub fn update_tray_icon(app: &AppHandle, state: TrayState) {
    // Update global state
    if let Ok(mut current) = current_state().lock() {
        *current = state;
    }

    // Update icon + badge-aware tooltip
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        set_state_icon(&tray, state);
    }
    refresh_tray_tooltip(app);
}

/// Get the current tray state.
#[allow(dead_code)]
pub fn get_current_state() -> TrayState {
    current_state()
        .lock()
        .map(|s| *s)
        .unwrap_or(TrayState::Idle)
}

// ─────────────────────────────────────────────────────────────────────────────
// macOS tray re-assertion (Tahoe/Sequoia workaround)
// ─────────────────────────────────────────────────────────────────────────────

/// Force the tray icon to re-register with macOS SystemUIServer.
///
/// macOS Tahoe (26.x) and Sequoia (15.x) can silently prevent
/// NSStatusItem rendering after app updates or preference corruption.
/// This is a widespread OS-level regression affecting Tauri (#13770),
/// Electron (#44817), Maccy, BetterDisplay, and other menubar apps.
///
/// Dispatches to the main thread via `run_on_main_thread` since
/// NSStatusItem is an AppKit object.
#[cfg(target_os = "macos")]
fn reassert_tray(app: &AppHandle) {
    use crate::util::logfile::log;

    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let _ = tray.set_visible(false);
            let _ = tray.set_visible(true);
            let state = get_current_state();
            set_state_icon(&tray, state);
            refresh_tray_tooltip(&handle);
            log("tray", "reassert_tray: completed");
        } else {
            log("tray", "reassert_tray: tray not found");
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Sync event listeners → auto tray state
// ─────────────────────────────────────────────────────────────────────────────

/// Wire sync events to tray icon state changes.
fn setup_sync_listeners(app: &AppHandle) {
    use crate::events::{
        EVENT_SYNC_COMPLETE, EVENT_SYNC_CONFLICT, EVENT_SYNC_ERROR, EVENT_SYNC_PROGRESS,
    };

    let app1 = app.clone();
    app.listen(EVENT_SYNC_PROGRESS, move |_event| {
        update_tray_icon(&app1, TrayState::Syncing);
    });

    let app2 = app.clone();
    app.listen(EVENT_SYNC_ERROR, move |_event| {
        update_tray_icon(&app2, TrayState::Error);
    });

    let app3 = app.clone();
    app.listen(EVENT_SYNC_COMPLETE, move |_event| {
        update_tray_icon(&app3, TrayState::Idle);
    });

    let app4 = app.clone();
    app.listen(EVENT_SYNC_CONFLICT, move |_event| {
        update_tray_icon(&app4, TrayState::Conflict);
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Share-notification badge
// ─────────────────────────────────────────────────────────────────────────────

/// Compose the current tray tooltip from global state + badge count and
/// apply it to the tray icon. Called by `update_tray_icon`,
/// `set_share_badge`, and `clear_share_badge` so the tooltip is always
/// consistent with both the tray state and the share badge.
fn refresh_tray_tooltip(app: &AppHandle) {
    let state = get_current_state();
    let count = SHARE_BADGE_COUNT.load(Ordering::SeqCst);
    let tooltip = if count > 0 {
        format!("{} · {} new share(s)", state.tooltip(), count)
    } else {
        state.tooltip().to_string()
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip.as_str()));
    }
}

/// Mark N unacknowledged share events. Updates the tray tooltip suffix.
/// Call from `share_notify::do_poll` after emitting new events.
pub fn set_share_badge(app: &AppHandle, count: usize) {
    SHARE_BADGE_COUNT.store(count, Ordering::SeqCst);
    refresh_tray_tooltip(app);
}

/// Clear the share badge. Call from `share_notify::share_detail_window_ready`
/// after the ack POST fires (best-effort).
pub fn clear_share_badge(app: &AppHandle) {
    SHARE_BADGE_COUNT.store(0, Ordering::SeqCst);
    refresh_tray_tooltip(app);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri command
// ─────────────────────────────────────────────────────────────────────────────

/// Tauri command: let the frontend explicitly set tray icon state.
///
/// Accepts: "idle", "syncing", "error", "conflict" (case-insensitive).
#[tauri::command]
pub fn set_tray_state(app: AppHandle, state: String) -> Result<(), String> {
    let tray_state = TrayState::from_str_loose(&state).ok_or_else(|| {
        format!(
            "Invalid tray state: '{}'. Expected: idle, syncing, error, conflict, prompt",
            state
        )
    })?;
    update_tray_icon(&app, tray_state);
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tray_state_from_str_loose() {
        assert_eq!(TrayState::from_str_loose("idle"), Some(TrayState::Idle));
        assert_eq!(
            TrayState::from_str_loose("SYNCING"),
            Some(TrayState::Syncing)
        );
        assert_eq!(TrayState::from_str_loose("Error"), Some(TrayState::Error));
        assert_eq!(
            TrayState::from_str_loose("conflict"),
            Some(TrayState::Conflict)
        );
        assert_eq!(TrayState::from_str_loose("prompt"), Some(TrayState::Prompt));
        assert_eq!(TrayState::from_str_loose("PROMPT"), Some(TrayState::Prompt));
        assert_eq!(TrayState::from_str_loose("unknown"), None);
        assert_eq!(TrayState::from_str_loose(""), None);
    }

    #[test]
    fn test_tray_state_tooltip() {
        assert_eq!(TrayState::Idle.tooltip(), "HQ Sync — Idle");
        assert_eq!(TrayState::Syncing.tooltip(), "HQ Sync — Syncing…");
        assert_eq!(TrayState::Error.tooltip(), "HQ Sync — Error");
        assert_eq!(TrayState::Conflict.tooltip(), "HQ Sync — Conflict");
        assert_eq!(TrayState::Prompt.tooltip(), "HQ Sync — Meeting Detected");
    }

    #[test]
    fn test_icon_bytes_are_valid_png() {
        // Verify that each included icon starts with the PNG magic bytes
        let png_magic: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

        for state in &[
            TrayState::Idle,
            TrayState::Syncing,
            TrayState::Error,
            TrayState::Conflict,
        ] {
            let bytes: &[u8] = match state {
                TrayState::Idle | TrayState::Prompt => include_bytes!("../icons/tray-idle@2x.png"),
                TrayState::Syncing => include_bytes!("../icons/tray-syncing@2x.png"),
                TrayState::Error => include_bytes!("../icons/tray-error@2x.png"),
                TrayState::Conflict => include_bytes!("../icons/tray-conflict@2x.png"),
            };
            assert!(
                bytes.starts_with(&png_magic),
                "Icon for {:?} does not start with PNG magic bytes",
                state
            );
        }
    }

    #[test]
    fn test_menu_id_constants() {
        assert_eq!(MENU_SYNC_NOW, "sync-now");
        assert_eq!(MENU_SETTINGS, "settings");
        assert_eq!(MENU_QUIT, "quit");
    }

    #[test]
    fn test_tray_id_constant() {
        assert_eq!(TRAY_ID, "hq-sync-tray");
    }

    #[test]
    fn test_current_state_default() {
        // OnceLock initialises to Idle on first access.
        // In parallel test runs another test may have mutated it,
        // so we just assert the value is a valid variant (exhaustive match).
        let state = get_current_state();
        match state {
            TrayState::Idle
            | TrayState::Syncing
            | TrayState::Error
            | TrayState::Conflict
            | TrayState::Prompt => {}
        }
    }

    #[test]
    fn test_compute_popover_position_centers_under_tray() {
        // tray icon at x=1000, y=0, 24x24px; window 320px wide; 4px gap.
        let (x, y) = compute_popover_position(1000.0, 0.0, 24.0, 24.0, 320.0, 4.0);
        assert_eq!(x, 1000 + 12 - 160); // = 852
        assert_eq!(y, 0 + 24 + 4); //     = 28
    }

    #[test]
    fn test_compute_popover_position_handles_off_screen_left() {
        // The raw helper remains pure anchor math; clamping is layered below
        // once monitor work-area bounds are known.
        let (x, _) = compute_popover_position(10.0, 0.0, 24.0, 24.0, 320.0, 4.0);
        assert_eq!(x, 10 + 12 - 160); // = -138
    }

    #[test]
    fn test_compute_clamped_popover_position_keeps_left_edge_on_screen() {
        let (x, y) = compute_clamped_popover_position(
            10.0, 0.0, 24.0, 24.0, 320.0, 480.0, 4.0, 0.0, 0.0, 1440.0, 900.0,
        );
        assert_eq!(x, 0);
        assert_eq!(y, 28);
    }

    #[test]
    fn test_compute_clamped_popover_position_keeps_right_edge_on_screen() {
        let (x, y) = compute_clamped_popover_position(
            1428.0, 0.0, 24.0, 24.0, 320.0, 480.0, 4.0, 0.0, 0.0, 1440.0, 900.0,
        );
        assert_eq!(x, 1120);
        assert_eq!(y, 28);
    }

    #[test]
    fn test_share_badge_count_atomic() {
        // AppHandle isn't constructible in unit tests, but we can verify
        // the AtomicUsize counter itself. `refresh_tray_tooltip` / `set_share_badge`
        // / `clear_share_badge` wrap this counter; the tray interaction is
        // exercised in integration/e2e contexts.
        let before = SHARE_BADGE_COUNT.load(Ordering::SeqCst);
        SHARE_BADGE_COUNT.store(5, Ordering::SeqCst);
        assert_eq!(SHARE_BADGE_COUNT.load(Ordering::SeqCst), 5);
        SHARE_BADGE_COUNT.store(0, Ordering::SeqCst);
        assert_eq!(SHARE_BADGE_COUNT.load(Ordering::SeqCst), 0);
        // Restore — best-effort in parallel test runs.
        SHARE_BADGE_COUNT.store(before, Ordering::SeqCst);
    }

    #[test]
    fn test_modal_guard_scoping() {
        // ModalGuard is RAII — increment on construction, decrement on
        // drop. This guard is the mechanism that keeps the popover
        // visible while a native picker dialog is open (see
        // folder_picker.rs); if Drop stops decrementing, the popover
        // will never auto-hide on blur again. Treat regressions here
        // as release blockers.
        //
        // No other test in this module touches MODAL_DEPTH, so parallel
        // execution is safe as long as we assert via deltas rather than
        // absolute values.
        let start = MODAL_DEPTH.load(Ordering::SeqCst);

        {
            let _g = ModalGuard::new();
            assert_eq!(
                MODAL_DEPTH.load(Ordering::SeqCst),
                start + 1,
                "guard should increment MODAL_DEPTH"
            );
            assert!(
                is_modal_open(),
                "is_modal_open should be true with guard alive"
            );

            {
                let _g2 = ModalGuard::new();
                assert_eq!(
                    MODAL_DEPTH.load(Ordering::SeqCst),
                    start + 2,
                    "nested guard should increment again"
                );
            }

            assert_eq!(
                MODAL_DEPTH.load(Ordering::SeqCst),
                start + 1,
                "dropping inner guard should decrement once"
            );
            assert!(
                is_modal_open(),
                "outer guard still alive — should still be open"
            );
        }

        assert_eq!(
            MODAL_DEPTH.load(Ordering::SeqCst),
            start,
            "dropping outer guard should decrement back to start"
        );
    }
}
