---
id: hq-sync-apply-vibrancy-needs-main-thread
title: apply_vibrancy must be dispatched to the main thread from a Tauri command
scope: repo
trigger: Any call to `window_vibrancy::apply_vibrancy(...)` (or `apply_blur`, `apply_acrylic`, sibling AppKit-touching helpers) inside hq-sync
enforcement: hard
version: 1
created: 2026-05-21
updated: 2026-05-21
public: false
source: user-correction
learned_from: session/2026-05-21-hq-sync-v0185
---

## Rule

NEVER call `window_vibrancy::apply_vibrancy(&window, ...)` (or any sibling AppKit-touching helper from the `window-vibrancy` crate) directly from inside a `#[tauri::command]` handler. Tauri commands run on the async runtime's worker pool, but AppKit is main-thread-only — the call panics with:

```
apply_vibrancy() can only be used on the main thread.
```

Instead, dispatch the AppKit work to the main thread via `app.run_on_main_thread(...)`:

```rust
#[tauri::command]
fn open_glassy_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = WebviewWindowBuilder::new(&app, "label", url)
        .transparent(true)
        .build()
        .map_err(|e| e.to_string())?;

    let w = window.clone();
    app.run_on_main_thread(move || {
        let _ = window_vibrancy::apply_vibrancy(
            &w,
            window_vibrancy::NSVisualEffectMaterial::HudWindow,
            None,
            None,
        );
    }).map_err(|e| e.to_string())?;

    Ok(())
}
```

## Rationale

The popover window gets vibrancy correctly because it is created and styled from inside the `tauri::Builder::setup()` closure, which already runs on the main thread. Secondary windows built via `WebviewWindowBuilder` from inside a `#[tauri::command]` handler do not — they run on a tokio worker thread.

Without the `run_on_main_thread` dispatch, one of two things happens:

1. **Debug builds** — panic with the message above, surfaced in the Rust log; the window opens but is fully see-through (content behind the app shows through) instead of glassy, because `transparent: true` was set but the vibrancy material was never applied.
2. **Release builds** — the panic may be swallowed by the runtime; the window still ships without vibrancy and looks broken.

The symptom is "transparent window with no frosted-glass effect" — easy to misdiagnose as a CSS / `transparent` flag / NSVisualEffectMaterial choice issue when the actual cause is thread affinity.

## How to comply

1. Audit every `apply_vibrancy` / `apply_blur` / `apply_acrylic` call in `src-tauri/src/commands/` for the calling context.
2. If the call is inside a `#[tauri::command]` (or any function reached only from one), wrap it in `app.run_on_main_thread(move || { ... })`.
3. If the call is inside `tauri::Builder::setup(|app| { ... })`, no dispatch is needed — that closure already runs on the main thread.
4. Verify by booting the app in a debug build and confirming the window renders with frosted vibrancy (not just see-through).

## Exceptions

None on macOS. On other platforms `window-vibrancy` is a no-op so the dispatch is harmless; keep it unconditional for portability.
