use crate::commands::config::MenubarPrefs;
use crate::util::logfile::log;
use crate::util::paths;
use std::path::PathBuf;

const BUNDLE_ID: &str = "ai.indigo.hq-sync-menubar";
const FALLBACK_APP_PATH: &str = "/Applications/HQ Sync.app/Contents/MacOS/HQ Sync";

/// Returns the path to ~/Library/LaunchAgents/{BUNDLE_ID}.plist.
fn plist_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Cannot determine home directory".to_string())?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", BUNDLE_ID)))
}

/// Resolve the app executable path by walking up from the current binary
/// to find the .app bundle, then pointing at Contents/MacOS/<name>.
/// Falls back to FALLBACK_APP_PATH if resolution fails.
fn resolve_app_path() -> String {
    if let Ok(exe) = std::env::current_exe() {
        // Walk up looking for a directory ending in .app
        let mut current = exe.as_path();
        while let Some(parent) = current.parent() {
            if let Some(name) = current.file_name() {
                if name.to_string_lossy().ends_with(".app") {
                    // Found the .app bundle — derive the executable path inside it
                    let app_name = name
                        .to_string_lossy()
                        .trim_end_matches(".app")
                        .to_string();
                    let bin_path = current
                        .join("Contents")
                        .join("MacOS")
                        .join(&app_name);
                    return bin_path.to_string_lossy().to_string();
                }
            }
            current = parent;
        }
    }
    FALLBACK_APP_PATH.to_string()
}

/// Generate the LaunchAgent plist XML content for the given app path.
fn generate_plist(app_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        BUNDLE_ID, app_path
    )
}

/// Check whether the LaunchAgent plist exists (i.e. autostart is enabled).
#[tauri::command]
pub async fn get_autostart_enabled() -> Result<bool, String> {
    let path = plist_path()?;
    Ok(path.exists())
}

/// Enable or disable autostart by creating or removing the LaunchAgent plist.
#[tauri::command]
pub async fn set_autostart_enabled(enabled: bool) -> Result<(), String> {
    let path = plist_path()?;

    if enabled {
        // Ensure ~/Library/LaunchAgents/ exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create LaunchAgents directory: {}", e))?;
        }

        let app_path = resolve_app_path();
        let plist_content = generate_plist(&app_path);

        std::fs::write(&path, plist_content)
            .map_err(|e| format!("Failed to write LaunchAgent plist: {}", e))?;
    } else {
        // Remove the plist if it exists
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove LaunchAgent plist: {}", e))?;
        }
    }

    Ok(())
}

/// Resolve the effective `startAtLogin` preference.
///
/// Defaults to `true` when menubar.json is absent or the field is missing —
/// matching the Settings UI default (`settings.rs`) and the `realtime_sync`
/// default-on convention in `daemon.rs`. Only an explicit
/// `"startAtLogin": false` opts out. Kept pure (takes parsed prefs) so the
/// default semantics are unit-testable without touching the real home dir.
fn effective_start_at_login(prefs: Option<&MenubarPrefs>) -> bool {
    prefs.and_then(|p| p.start_at_login).unwrap_or(true)
}

/// Read `startAtLogin` from ~/.hq/menubar.json (best-effort), applying the
/// default-on semantics of `effective_start_at_login`.
fn start_at_login_pref() -> bool {
    let path = match paths::menubar_json_path() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let prefs: Option<MenubarPrefs> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    effective_start_at_login(prefs.as_ref())
}

/// Idempotent launch-time autostart reconciliation.
///
/// Called from `main.rs` `.setup()`. Ensures the LaunchAgent plist matches
/// the effective `startAtLogin` preference so a fresh install autostarts by
/// default without the user having to open Settings — while still honouring
/// an explicit `"startAtLogin": false` opt-out (in which case a stale plist
/// is removed). Best-effort: every IO error is logged and swallowed so a
/// failure here can never abort app launch.
pub fn ensure_autostart_on_launch() {
    let enabled = start_at_login_pref();
    let path = match plist_path() {
        Ok(p) => p,
        Err(e) => {
            log("autostart", &format!("ensure: cannot resolve plist path: {e}"));
            return;
        }
    };
    let exists = path.exists();

    if enabled && !exists {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log("autostart", &format!("ensure: mkdir LaunchAgents failed: {e}"));
                return;
            }
        }
        let plist = generate_plist(&resolve_app_path());
        match std::fs::write(&path, plist) {
            Ok(()) => log("autostart", "ensure: created LaunchAgent plist (default-on)"),
            Err(e) => log("autostart", &format!("ensure: write plist failed: {e}")),
        }
    } else if !enabled && exists {
        match std::fs::remove_file(&path) {
            Ok(()) => log("autostart", "ensure: removed LaunchAgent plist (explicit opt-out)"),
            Err(e) => log("autostart", &format!("ensure: remove plist failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn prefs_with_start(start: Option<bool>) -> MenubarPrefs {
        MenubarPrefs {
            hq_path: None,
            sync_on_launch: None,
            notifications: None,
            start_at_login: start,
            autostart_daemon: None,
            realtime_sync: None,
            personal_sync_enabled: None,
            instant_sync: None,
            drift_staging_repo: None,
            share_notifications: None,
            staging_channel: None,
            release_channel: None,
            meeting_detect_notify: None,
        }
    }

    #[test]
    fn test_effective_start_at_login_defaults_on_when_absent() {
        // No menubar.json at all -> autostart on by default.
        assert!(effective_start_at_login(None));
    }

    #[test]
    fn test_effective_start_at_login_defaults_on_when_field_missing() {
        // menubar.json exists but startAtLogin not set -> default on.
        let p = prefs_with_start(None);
        assert!(effective_start_at_login(Some(&p)));
    }

    #[test]
    fn test_effective_start_at_login_explicit_true() {
        let p = prefs_with_start(Some(true));
        assert!(effective_start_at_login(Some(&p)));
    }

    #[test]
    fn test_effective_start_at_login_explicit_false_opts_out() {
        // The one case that disables autostart: explicit opt-out.
        let p = prefs_with_start(Some(false));
        assert!(!effective_start_at_login(Some(&p)));
    }

    #[test]
    fn test_plist_path_format() {
        let path = plist_path().unwrap();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("Library/LaunchAgents"));
        assert!(path_str.ends_with("ai.indigo.hq-sync-menubar.plist"));
    }

    #[test]
    fn test_generate_plist_content() {
        let plist = generate_plist("/Applications/HQ Sync.app/Contents/MacOS/HQ Sync");

        assert!(plist.contains("<?xml version=\"1.0\""));
        assert!(plist.contains("<!DOCTYPE plist"));
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains(&format!("<string>{}</string>", BUNDLE_ID)));
        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(plist.contains(
            "<string>/Applications/HQ Sync.app/Contents/MacOS/HQ Sync</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn test_generate_plist_custom_path() {
        let custom = "/usr/local/bin/my-app";
        let plist = generate_plist(custom);
        assert!(plist.contains(&format!("<string>{}</string>", custom)));
    }

    #[test]
    fn test_resolve_app_path_returns_string() {
        // In test context we won't be inside a .app bundle,
        // so this should return the fallback path.
        let path = resolve_app_path();
        assert!(!path.is_empty());
        // In CI/test, expect fallback
        assert_eq!(path, FALLBACK_APP_PATH);
    }

    #[test]
    fn test_plist_write_and_remove() {
        let tmp = TempDir::new().unwrap();
        let plist_file = tmp.path().join("ai.indigo.hq-sync-menubar.plist");

        // Write
        let content = generate_plist(FALLBACK_APP_PATH);
        std::fs::write(&plist_file, &content).unwrap();
        assert!(plist_file.exists());

        // Verify content
        let read_back = std::fs::read_to_string(&plist_file).unwrap();
        assert!(read_back.contains(BUNDLE_ID));

        // Remove
        std::fs::remove_file(&plist_file).unwrap();
        assert!(!plist_file.exists());
    }

    #[test]
    fn test_plist_is_valid_xml() {
        let plist = generate_plist(FALLBACK_APP_PATH);
        // Basic XML validity checks
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("<plist version=\"1.0\">"));
        assert!(plist.contains("</plist>"));
        assert!(plist.contains("<dict>"));
        assert!(plist.contains("</dict>"));
        assert!(plist.contains("<array>"));
        assert!(plist.contains("</array>"));
    }
}
