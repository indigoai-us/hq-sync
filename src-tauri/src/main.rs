#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

mod commands;
mod events;
mod sentry_scrub;
mod tray;
mod updater;
mod util;

fn main() {
    use sentry::ClientOptions;
    use sentry_scrub::before_send;
    use std::sync::Arc;
    // `SENTRY_DSN` is set at compile time by build.rs, which reads
    // `HQ_SYNC_SENTRY_DSN` from the CI env. On local `cargo build`
    // / `cargo tauri dev` / PR CI (where the release-only secret is not
    // in scope), build.rs emits `cargo:rustc-env=SENTRY_DSN=` (empty),
    // so `env!("SENTRY_DSN")` evaluates to `""` — gate on emptiness → None
    // so the Sentry client no-ops cleanly in dev instead of crashing at startup.
    let dsn_str = env!("SENTRY_DSN");
    let dsn: Option<sentry::types::Dsn> = if dsn_str.is_empty() {
        None
    } else {
        Some(dsn_str.parse().expect("SENTRY_DSN invalid at build time"))
    };
    let _guard = sentry::init(ClientOptions {
        dsn,
        release: Some(format!("hq-sync@{}", env!("CARGO_PKG_VERSION")).into()),
        environment: Some(
            option_env!("SENTRY_ENVIRONMENT")
                .unwrap_or("production")
                .into(),
        ),
        sample_rate: std::env::var("SENTRY_SAMPLE_RATE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0),
        before_send: Some(Arc::new(before_send)),
        ..Default::default()
    });
    sentry::configure_scope(|scope| {
        scope.set_tag("repo", "hq-sync");
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updater::PendingUpdate(Mutex::new(None)))
        // Menubar-app close behaviour: intercept window-close (traffic-light
        // red button, Cmd-W, File→Close) and hide the window instead of
        // terminating the process. The app only truly exits via the tray
        // context menu's "Quit" item (see tray.rs MENU_QUIT). This matches
        // native Cocoa NSStatusItem apps like Bartender, Rectangle, Raycast.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::process::spawn_process,
            commands::process::cancel_process,
            commands::oauth::start_oauth_login,
            commands::oauth::oauth_listen_for_code,
            commands::oauth::oauth_exchange_code,
            commands::auth::get_auth_state,
            commands::auth::has_stored_token,
            commands::auth::refresh_tokens,
            commands::config::get_config,
            commands::status::get_sync_status,
            commands::sync::start_sync,
            commands::sync::cancel_sync,
            commands::workspaces::list_syncable_workspaces,
            commands::workspaces::connect_workspace_to_cloud,
            commands::conflicts::resolve_conflict,
            commands::conflicts::open_in_editor,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::folder_picker::pick_folder,
            commands::autostart::get_autostart_enabled,
            commands::autostart::set_autostart_enabled,
            commands::daemon::start_daemon,
            commands::daemon::stop_daemon,
            commands::daemon::daemon_status,
            tray::set_tray_state,
            updater::check_for_updates,
            updater::install_update,
        ])
        .setup(|app| {
            // macOS menubar-app activation policy. `Accessory` = no Dock
            // icon, no entry in CMD-Tab, no top-of-screen app menu bar.
            // The tray icon is the only surface. Without this the app
            // appears in the Dock whenever the window is shown.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            tray::setup_tray(&app.handle())?;
            updater::setup_update_checker(&app.handle());

            // Fire-and-forget: warm the npx cache for
            // `@indigoai-us/hq-cloud@<HQ_CLOUD_VERSION>` so the user's
            // first click of "Sync Now" doesn't eat the 3–10s first-time
            // download. No-ops if the cache is already warm. See
            // `commands::prewarm` for the rationale.
            commands::prewarm::spawn_prewarm();

            // Feature-flagged daemon autostart (V2 prep — default OFF)
            if commands::daemon::is_autostart_enabled() {
                std::thread::spawn(|| {
                    // Small delay to let the app fully initialize
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let _ = commands::daemon::start_daemon();
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
