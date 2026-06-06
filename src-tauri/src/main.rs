#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
use tauri::Manager;
use std::sync::Mutex;

mod commands;
mod events;
mod sentry_scrub;
mod tray;
mod updater;
mod util;

#[cfg(target_os = "macos")]
fn apply_liquid_glass(window: &tauri::WebviewWindow) {
    use util::logfile::log;
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};

    // window-vibrancy's apply_vibrancy returns Result<(), Error>. Earlier we
    // swallowed the error with `let _ =`, which made silent failures
    // indistinguishable from "vibrancy applied but visually subtle." Log on
    // both success and failure so the persistent diagnostic log can answer
    // "is vibrancy actually being applied?" without a debugger attached.
    match apply_vibrancy(
        window,
        NSVisualEffectMaterial::Popover,
        Some(NSVisualEffectState::Active),
        Some(18.0),
    ) {
        Ok(()) => log("ui", "apply_vibrancy: success (Popover material, blur 18, active)"),
        Err(e) => log("ui", &format!("apply_vibrancy FAILED: {e}")),
    }
}

/// Set the macOS application icon image at runtime.
///
/// We need this because the app's activation policy is `Accessory` (no Dock
/// icon, tray-only). When a detached window like the Meetings window is
/// open, macOS still shows the app in Mission Control and the window
/// switcher — but with NO bundled `.app` icon registered at runtime, the
/// representation is a generic folder/document. Setting
/// `NSApp.applicationIconImage` programmatically gives those surfaces an
/// HQ icon to render even though there's no Dock badge.
///
/// `cargo tauri dev` doesn't build a proper `.app` bundle either, so this
/// is the same fix in both dev and production.
///
/// Uses raw objc2 messaging so we don't pull in objc2-app-kit /
/// objc2-foundation just for one call. The image is leaked intentionally
/// — it's set once at startup and held by NSApplication for the lifetime
/// of the process, so manual release would be a use-after-free.
#[cfg(target_os = "macos")]
fn set_app_icon_from_bytes(bytes: &'static [u8]) {
    use objc2::{class, msg_send, runtime::AnyObject};
    use util::logfile::log;

    unsafe {
        let data_cls = class!(NSData);
        let data: *mut AnyObject = msg_send![
            data_cls,
            dataWithBytes: bytes.as_ptr() as *const std::ffi::c_void,
            length: bytes.len()
        ];
        if data.is_null() {
            log("ui", "set_app_icon: NSData::dataWithBytes returned nil");
            return;
        }

        let image_cls = class!(NSImage);
        let image_alloc: *mut AnyObject = msg_send![image_cls, alloc];
        let image: *mut AnyObject = msg_send![image_alloc, initWithData: data];
        if image.is_null() {
            log("ui", "set_app_icon: NSImage::initWithData returned nil");
            return;
        }

        let app_cls = class!(NSApplication);
        let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
        if app.is_null() {
            log("ui", "set_app_icon: NSApplication::sharedApplication returned nil");
            return;
        }
        let _: () = msg_send![app, setApplicationIconImage: image];
        log("ui", "set_app_icon: applied HQ icon to NSApp");
    }
}

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

    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

    // Opt+Shift+H — global hotkey to summon the popover from anywhere.
    // Defined up front so the plugin builder and the setup-time `register`
    // call agree on the exact key combo.
    let show_shortcut = Shortcut::new(Some(Modifiers::ALT | Modifiers::SHIFT), Code::KeyH);

    tauri::Builder::default()
        // single-instance MUST be the first plugin: it runs before any other
        // plugin can create a window or spawn a process, so a second launch is
        // collapsed back into the already-running instance. macOS routes a
        // notification click (and a re-open of the installed copy) through
        // Launch Services by bundle id, which would otherwise start a duplicate
        // menubar process. Here the callback surfaces the existing instance and
        // the second process exits instead of becoming a ghost duplicate.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Prefer the detached "HQ Meetings" (desktop-alt) window when it's
            // open, else the main popover. show + unminimize + focus is
            // idempotent, so re-firing on an already-visible window is a no-op.
            let target = app
                .get_webview_window("desktop-alt")
                .or_else(|| app.get_webview_window("main"));
            if let Some(window) = target {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
                crate::util::logfile::log(
                    "app",
                    "single-instance: focused existing window on second launch",
                );
            } else {
                crate::util::logfile::log(
                    "app",
                    "single-instance: second launch with no window to focus",
                );
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if shortcut == &show_shortcut && event.state() == ShortcutState::Pressed {
                        tray::show_window_at_tray(app);
                    }
                })
                .build(),
        )
        .manage(updater::PendingUpdate(Mutex::new(None)))
        .manage(commands::new_files::PendingNewFiles(Mutex::new(Vec::new())))
        .manage(commands::drift_detail::PendingDrift(Mutex::new(None)))
        .manage(commands::activity::SessionActivity::new())
        .manage(commands::share_notify::PendingShareEvents(Mutex::new(Vec::new())))
        .manage(commands::dm_notify::PendingDmEvents(Mutex::new(Vec::new())))
        .manage(commands::banner::PendingBanner(Mutex::new(None)))
        .manage(commands::packages::PendingPackages(Mutex::new(None)))
        // Menubar-app close behaviour: intercept window-close (traffic-light
        // red button, Cmd-W, File→Close) and hide the window instead of
        // terminating the process. The app only truly exits via the tray
        // context menu's "Quit" item (see tray.rs MENU_QUIT). This matches
        // native Cocoa NSStatusItem apps like Bartender, Rectangle, Raycast.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Only hide the main popover window — let other windows
                // (e.g. new-files-detail) close normally.
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::quit_app,
            commands::app::open_settings_window,
            commands::app::open_claude_code_link,
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
            commands::first_run::is_first_run,
            commands::first_run::should_show_auto_sync_notice,
            commands::first_run::mark_first_run_complete,
            commands::first_run::mark_auto_sync_notice_shown,
            commands::workspaces::list_syncable_workspaces,
            commands::workspaces::connect_workspace_to_cloud,
            commands::sync_mode::get_sync_mode,
            commands::sync_mode::set_sync_mode,
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
            updater::available_channels,
            commands::hq_cli_update::check_hq_cli_update,
            commands::hq_cli_update::install_hq_cli_update,
            commands::hq_core_update::get_hq_version,
            commands::hq_core_update::install_hq_core_update,
            commands::hq_core_drift::restore_from_upstream,
            commands::hq_core_staging::run_replace_from_staging,
            commands::hq_core_state::check_core_state,
            commands::drift_detail::open_drift_detail,
            commands::drift_detail::drift_window_ready,
            commands::new_files::open_new_files_detail,
            commands::new_files::detail_window_ready,
            commands::packages::list_packages,
            commands::packages::check_package_updates,
            commands::packages::install_package,
            commands::packages::update_package,
            commands::packages::uninstall_package,
            commands::packages::open_packages_window,
            commands::packages::packages_window_ready,
            commands::activity::open_activity_log,
            commands::activity::activity_window_ready,
            commands::activity::get_activity_log,
            commands::meetings::meetings_feature_enabled,
            commands::desktop_alt::desktop_alt_enabled,
            commands::desktop_alt::get_company_summary,
            commands::desktop_alt::get_company_board,
            commands::desktop_alt::get_company_activity,
            commands::desktop_alt::get_company_deployments,
            commands::desktop_alt::get_company_secrets,
            commands::projects_local::get_local_projects,
            commands::projects_local::get_local_project_prd,
            commands::projects_local::get_local_project_readme,
            commands::projects_local::get_local_company_goals,
            commands::projects_local::set_local_project_status,
            commands::projects_local::set_local_story_passes,
            commands::library_local::get_library_root,
            commands::library_local::get_library_company,
            commands::library_local::get_library_worker_detail,
            commands::library_local::get_library_skill_detail,
            commands::marketplace::list_marketplace_listings,
            commands::marketplace::get_marketplace_listing,
            commands::marketplace::install_marketplace_pack,
            commands::marketplace::yank_marketplace_listing,
            commands::marketplace::list_moderation_queue,
            commands::marketplace::decide_moderation_listing,
            commands::marketplace::list_creator_applications,
            commands::marketplace::decide_creator_application,
            commands::marketplace::record_marketplace_install,
            commands::marketplace::publish_marketplace_pack,
            commands::marketplace::request_creator_access,
            commands::marketplace::pick_pack_directory,
            commands::marketplace::claim_creator_handle,
            commands::marketplace::update_creator_profile,
            commands::marketplace::upload_creator_avatar,
            commands::marketplace::pick_avatar_file,
            commands::marketplace::get_creator_profile,
            commands::meetings::meetings_list_upcoming,
            commands::meetings::meetings_list_scheduled_bots,
            commands::meetings::meetings_list_memberships,
            commands::meetings::meetings_list_accounts,
            commands::meetings::meetings_list_calendars_for_account,
            commands::meetings::meetings_invite_bot,
            commands::meetings::meetings_join_bot_now,
            commands::meetings::meetings_cancel_bot,
            commands::meetings::open_meetings_window,
            commands::meetings::meetings_check_bot_for_url,
            commands::meetings::meetings_notify_detected,
            commands::meetings::meetings_clear_prompt_badge,
            commands::permissions::permissions_open_settings,
            commands::permissions::permissions_force_native_register,
            commands::permissions::meetings_permissions_state,
            commands::permissions::open_meeting_permissions_window,
            commands::recall_sdk::meeting_detect_feature_enabled,
            commands::recall_sdk::start_recording,
            commands::recall_sdk::stop_recording,
            commands::recall_sdk::meetings_list_active_detections,
            tray::meetings_set_prompt_badge,
            commands::desktop_alt::open_desktop_alt_window,
            commands::desktop_alt::desktop_alt_consume_pending_route,
            commands::share_notify::poll_shared_with_me,
            commands::share_notify::open_share_detail,
            commands::share_notify::share_detail_window_ready,
            commands::dm_notify::poll_dm_inbox,
            commands::dm_notify::open_dm_detail,
            commands::dm_notify::dm_detail_window_ready,
            commands::dm_notify::send_dm,
            commands::dm_notify::fetch_dm_thread,
            commands::notification_history::fetch_notification_history,
            commands::notification_history::open_notification_history,
            commands::notifications::notification_permission_state,
            commands::notifications::notification_request_permission,
            commands::banner::banner_window_ready,
            commands::banner::banner_action,
            commands::banner::dismiss_banner,
            commands::banner::resize_banner,
            commands::banner::show_main_window,
            commands::banner::preview_dm_banner,
            commands::banner::preview_share_banner,
            commands::banner::preview_update_banner,
            commands::banner::preview_meeting_banner,
        ])
        .setup(|app| {
            // Classify this launch (FirstRun / ExistingUpdate / Normal) and
            // cache it in managed state. MUST run before anything that can
            // write `machineId` to menubar.json (sync, telemetry, the
            // share/dm pollers below) — `machineId` is the tiebreaker that
            // distinguishes a brand-new install from a legacy user updating.
            // See commands/first_run.rs for the full rationale.
            commands::first_run::classify_launch(app.handle());

            // One-shot migration of any legacy `/deploy`-skill stub at
            // ~/.hq/config.json. Runs first so subsequent prewarm /
            // daemon / sync calls see a clean HqConfig (when a personal
            // person-entity.json is on disk) or a missing config that
            // surfaces SetupNeeded cleanly (when reconstruction isn't
            // possible). Best-effort and idempotent — failures log to the
            // diagnostic file and don't abort launch.
            commands::config::migrate_legacy_config_stub();

            // Default-on autostart: ensure the LaunchAgent plist matches the
            // effective `startAtLogin` pref (default true) so a fresh install
            // opens HQ Sync at login without the user opening Settings first.
            // Honours an explicit `"startAtLogin": false` opt-out. Best-effort
            // and idempotent — never aborts launch.
            #[cfg(target_os = "macos")]
            commands::autostart::ensure_autostart_on_launch();

            // macOS menubar-app activation policy. `Accessory` = no Dock
            // icon, no entry in CMD-Tab, no top-of-screen app menu bar.
            // The tray icon is the only surface. Without this the app
            // appears in the Dock whenever the window is shown.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Brand the app's runtime icon image. With Accessory activation
            // policy there's no Dock icon, but the meetings window (and any
            // future detached windows) still show up in Mission Control /
            // Cmd-Tab — by default with a generic folder icon because no
            // .app bundle icon is registered at runtime. Setting
            // NSApp.applicationIconImage gives those surfaces the HQ mark
            // to render even though the Dock stays empty.
            #[cfg(target_os = "macos")]
            {
                const HQ_ICON_PNG: &[u8] = include_bytes!("../icons/128x128@2x.png");
                set_app_icon_from_bytes(HQ_ICON_PNG);
            }

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                apply_liquid_glass(&window);
            }

            tray::setup_tray(app.handle())?;
            // Hard version-gate against hq-pro fires at 5s (BEFORE the soft
            // updater at 10s) so a known-bad release can be yanked before the
            // user touches anything sensitive. Server-side source of truth is
            // `apps/hq-pro/src/vault-service/handlers/client-version-check.ts`.
            // See `commands::version_gate` for the rationale.
            commands::version_gate::setup_version_gate(app.handle());
            updater::setup_update_checker(app.handle());

            // Share-notification poller. Gated solely on the shareNotifications
            // menubar preference (the @getindigo.ai dogfood gate was removed
            // 2026-05-26 — see share_notify::should_poll). Gate is checked
            // inside share_notify, not here, so it is never scattered.
            //
            // Delivery runs on an independent timer (launch poll + interval),
            // so notifications no longer depend on a sync completing — see the
            // 2026-05-28 incident report. The post-sync poll below is a
            // latency optimization layered on top, not the sole trigger.
            {
                use tauri::Listener;
                // (a) Launch poll (5s delay) + independent interval timer.
                commands::share_notify::setup_share_notify_poller(app.handle().clone());

                // (a') Instant-DM push receiver — MQTT-over-WSS to AWS IoT Core.
                // Gated on @getindigo.ai inside setup_dm_mqtt_receiver; wakes
                // `poll_dm_once` on push so DMs arrive in near-real-time instead
                // of waiting up to 60s. The interval poll above is the long-stop,
                // so this is purely additive — any MQTT failure falls back to it
                // silently. macOS-gated like the rest of the notification surface.
                #[cfg(target_os = "macos")]
                commands::dm_mqtt::setup_dm_mqtt_receiver(app.handle().clone());

                // (a'') Clickable meeting-detected notifications. Installs a
                // UNUserNotificationCenter delegate (once) and stashes the
                // AppHandle so a *cold* banner click — no desktop-alt window
                // open, hence no frontend listener — can still open the
                // "HQ Meetings" window straight from Rust. Safe to call when
                // unbundled (guards on bundleIdentifier internally).
                #[cfg(target_os = "macos")]
                commands::un_notify::register_delegate(app.handle());

                // (b) Post-sync poll — low-latency top-up after a sync run.
                let poll_handle = app.handle().clone();
                app.listen(
                    crate::events::EVENT_SYNC_ALL_COMPLETE,
                    move |_event| {
                        let h = poll_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            commands::share_notify::poll_once(h).await;
                        });
                    },
                );
            }

            // SPIKE: env-var trigger to preview the custom notification banner
            // without devtools / real inbound events. Pops one representative
            // banner per source — DM (2s), share (10s), update (18s), meeting
            // (26s) — spaced past the 6s auto-dismiss so each is seen in turn.
            // No-op when unset.
            //   HQ_SYNC_PREVIEW_BANNER=1     → DM only
            //   HQ_SYNC_PREVIEW_BANNER=all   → DM, share, update, meeting
            match std::env::var("HQ_SYNC_PREVIEW_BANNER").as_deref() {
                Ok("1") | Ok("all") => {
                    let all = std::env::var("HQ_SYNC_PREVIEW_BANNER").as_deref() == Ok("all");
                    let h = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        use std::time::Duration;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        let _ = commands::banner::preview_dm_banner(h.clone()).await;
                        if all {
                            tokio::time::sleep(Duration::from_secs(8)).await;
                            let _ = commands::banner::preview_share_banner(h.clone()).await;
                            tokio::time::sleep(Duration::from_secs(8)).await;
                            let _ = commands::banner::preview_update_banner(h.clone()).await;
                            tokio::time::sleep(Duration::from_secs(8)).await;
                            let _ = commands::banner::preview_meeting_banner(h.clone()).await;
                        }
                    });
                }
                _ => {}
            }

            // Register Opt+Shift+H globally so the popover can be summoned
            // from any app. The handler (configured on the plugin builder
            // above) calls `tray::show_window_at_tray`. Registration can
            // fail if another app already holds the chord — log and
            // continue so the rest of the app still launches.
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                let shortcut =
                    Shortcut::new(Some(Modifiers::ALT | Modifiers::SHIFT), Code::KeyH);
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    util::logfile::log(
                        "ui",
                        &format!("global shortcut Opt+Shift+H register FAILED: {e}"),
                    );
                }
            }

            commands::hq_cli_update::setup_hq_cli_update_checker(app.handle());
            commands::hq_core_state::setup_core_state_checker(app.handle());

            // Fire-and-forget: warm the npx cache for
            // `@indigoai-us/hq-cloud@<HQ_CLOUD_VERSION>` so the user's
            // first click of "Sync Now" doesn't eat the 3–10s first-time
            // download. No-ops if the cache is already warm. See
            // `commands::prewarm` for the rationale.
            commands::prewarm::spawn_prewarm();

            // Auto-start the watcher when either flag is on:
            //   - `autostart_daemon` (V2-prep devtools flag, default OFF)
            //   - `realtime_sync`   (user-facing Auto-sync toggle, default ON)
            if commands::daemon::is_autostart_enabled()
                || commands::daemon::is_realtime_sync_enabled()
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    // Small delay to let the app fully initialize
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let _ = commands::daemon::start_daemon(handle);
                });
            }

            // Force-register the .app bundle with macOS TCC for Accessibility
            // and Screen Recording, and start the Recall Desktop SDK sidecar.
            // Both are gated on `meeting_detect_eligible()` so users outside
            // the Phase-0 allowlist see no permission prompts, no SDK process,
            // and no Recall API calls. Best-effort: failures logged + ignored.
            // See `commands::recall_sdk` for the gate definition and the
            // graceful-degradation contract.
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if !commands::recall_sdk::meeting_detect_eligible().await {
                        util::logfile::log(
                            "recall-sdk",
                            "setup: user not in Phase-0 allowlist — skipping TCC register + SDK spawn",
                        );
                        return;
                    }

                    #[cfg(target_os = "macos")]
                    {
                        match commands::permissions::permissions_force_native_register() {
                            Ok((ax, sc)) => util::logfile::log(
                                "permissions",
                                &format!(
                                    "native register: accessibility={ax} screen-capture={sc}"
                                ),
                            ),
                            Err(e) => util::logfile::log(
                                "permissions",
                                &format!("native register failed: {e}"),
                            ),
                        }

                        // Guide the user when something the SDK needs is still
                        // missing. Native prompts only fire once, and
                        // Accessibility / Screen Recording never re-prompt
                        // after a denial — so without this an eligible user
                        // who dismissed or denied a prompt is left with
                        // meeting-detect silently broken and no idea why.
                        // Opening the wizard surfaces exactly which permissions
                        // are outstanding and deep-links each to the right
                        // System Settings pane. No-op once all are granted.
                        match commands::permissions::meetings_permissions_state() {
                            Ok(state) if !state.all_required_granted => {
                                util::logfile::log(
                                    "permissions",
                                    "startup: required meeting permissions missing — opening wizard",
                                );
                                if let Err(e) =
                                    commands::permissions::open_meeting_permissions_window(
                                        handle.clone(),
                                    )
                                    .await
                                {
                                    util::logfile::log(
                                        "permissions",
                                        &format!("open_meeting_permissions_window failed: {e}"),
                                    );
                                }
                            }
                            Ok(_) => util::logfile::log(
                                "permissions",
                                "startup: all required meeting permissions granted",
                            ),
                            Err(e) => util::logfile::log(
                                "permissions",
                                &format!("meetings_permissions_state failed: {e}"),
                            ),
                        }
                    }

                    if let Err(e) = commands::recall_sdk::start_recall_sdk(handle.clone()).await {
                        util::logfile::log(
                            "recall-sdk",
                            &format!("start_recall_sdk error (app continues): {e}"),
                        );
                    }

                    // Recover any recording that was in flight when the app
                    // last closed. The durable recordings ledger persists the
                    // windowId→recordingId mapping on start and clears it on a
                    // clean stop; a leftover entry means a crash/forced-quit
                    // mid-recording. This queries hq-pro for each such
                    // recording's status and surfaces a "still processing" /
                    // "ingest failed" thread instead of silently losing it.
                    // Best-effort: all failures are logged + swallowed inside.
                    commands::recall_sdk::reconcile_recordings_on_launch(handle).await;
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
