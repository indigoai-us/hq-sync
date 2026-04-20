use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub body: Option<String>,
    pub date: Option<String>,
}

/// Stores pending update info so the frontend can query it.
pub struct PendingUpdate(pub Mutex<Option<UpdateInfo>>);

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            let info = UpdateInfo {
                version: update.version.clone(),
                body: update.body.clone(),
                date: update.date.map(|d| d.to_string()),
            };
            // Store as pending
            if let Some(state) = app.try_state::<PendingUpdate>() {
                *state.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(info.clone());
            }
            // Emit event for frontend
            let _ = app.emit("update:available", &info);
            Ok(Some(info))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    // Note: We must call updater.check() again here because the tauri_plugin_updater::Update
    // type cannot be stored (not Clone). The PendingUpdate state only holds metadata (UpdateInfo).
    // This is an architectural constraint of the plugin, not a redundant call.
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            // Download and install
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|e| e.to_string())?;
            // On macOS, download_and_install typically terminates the process before reaching
            // this line. restart() is retained as a safety net for platforms where it returns.
            app.restart();
        }
        Ok(None) => return Err("No update available".to_string()),
        Err(e) => return Err(e.to_string()),
    }
}

/// Spawns a background task that checks for updates on launch (after 10s delay)
/// and every 6 hours thereafter. Emits `update:available` events but does NOT
/// auto-install — the user must initiate installation.
pub fn setup_update_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Wait 10 seconds for app to settle
        tokio::time::sleep(Duration::from_secs(10)).await;

        loop {
            // Check for updates silently — log errors for field debugging via Console.app
            match handle.updater() {
                Ok(updater) => match updater.check().await {
                    Ok(Some(update)) => {
                        let info = UpdateInfo {
                            version: update.version.clone(),
                            body: update.body.clone(),
                            date: update.date.map(|d| d.to_string()),
                        };
                        if let Some(state) = handle.try_state::<PendingUpdate>() {
                            *state.0.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(info.clone());
                        }
                        let _ = handle.emit("update:available", &info);
                    }
                    Ok(None) => {} // No update available — nothing to do
                    Err(e) => eprintln!("[updater] background check failed: {e}"),
                },
                Err(e) => eprintln!("[updater] failed to get updater instance: {e}"),
            }
            // Wait 6 hours before next check
            tokio::time::sleep(Duration::from_secs(21600)).await;
        }
    });
}
