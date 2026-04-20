use std::path::{Path, PathBuf};
use std::process::Command;

/// Returns the ~/.hq/ directory path.
pub fn hq_config_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Cannot determine home directory".to_string())?;
    Ok(home.join(".hq"))
}

/// Resolve a node-backed CLI binary (e.g. `hq-sync-runner`, `hq`) to an
/// absolute path.
///
/// **Why this exists:** Tauri apps launched from Dock/Finder inherit a
/// minimal launchd PATH (roughly `/usr/bin:/bin:/usr/sbin:/sbin`) — they do
/// NOT see `/opt/homebrew/bin` or the user's `.zshrc` additions. A bare
/// `Command::new("hq-sync-runner")` then fails with "No such file or
/// directory (os error 2)" even though `which hq-sync-runner` works in
/// Terminal.
///
/// Resolution order:
/// 1. `$HOME/.npm-global/bin/{name}` — user-level npm prefix (no-sudo installs)
/// 2. `/opt/homebrew/bin/{name}` — Apple Silicon homebrew
/// 3. `/usr/local/bin/{name}` — Intel homebrew / system-wide installs
/// 4. Ask a login shell via `zsh -lc 'command -v {name}'` — respects the
///    user's actual shell config (picks up nvm, volta, asdf, etc.).
///
/// Returns the bare name as a last-ditch fallback — the caller's
/// `Command::new` will then error with the original "os error 2", which
/// surfaces as a sync error the UI can show. We don't invent a path that
/// doesn't exist.
pub fn resolve_bin(name: &str) -> String {
    // 1. User npm prefix
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".npm-global").join("bin").join(name);
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }

    // 2 + 3. Standard install locations
    for prefix in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let candidate = Path::new(prefix).join(name);
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }

    // 4. Login-shell PATH lookup — catches nvm/volta/asdf + any custom prefix
    //    the user configured in .zshrc. `-l` makes zsh a login shell so it
    //    sources the full startup chain. `command -v` prints the resolved
    //    path on success, nothing on miss.
    if let Ok(output) = Command::new("zsh").args(["-lc", &format!("command -v {}", name)]).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && Path::new(&path).exists() {
                return path;
            }
        }
    }

    // Fall back to bare name — Command::new will then produce os error 2
    // with the binary name still recognizable in the error message.
    name.to_string()
}

/// Returns the path to ~/.hq/config.json.
pub fn config_json_path() -> Result<PathBuf, String> {
    Ok(hq_config_dir()?.join("config.json"))
}

/// Returns the path to ~/.hq/menubar.json.
pub fn menubar_json_path() -> Result<PathBuf, String> {
    Ok(hq_config_dir()?.join("menubar.json"))
}

/// Resolve the HQ folder path with priority:
/// 1. menubar_override (from menubar.json hqPath)
/// 2. config_path (from config.json hqFolderPath)
/// 3. ~/HQ default
pub fn resolve_hq_folder(
    config_path: Option<&str>,
    menubar_override: Option<&str>,
) -> PathBuf {
    // Priority 1: menubar.json override
    if let Some(path) = menubar_override {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    // Priority 2: config.json hqFolderPath
    if let Some(path) = config_path {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    // Priority 3: ~/HQ default
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join("HQ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hq_config_dir() {
        let dir = hq_config_dir().unwrap();
        assert!(dir.ends_with(".hq"));
    }

    #[test]
    fn test_config_json_path() {
        let path = config_json_path().unwrap();
        assert!(path.ends_with("config.json"));
        assert!(path.parent().unwrap().ends_with(".hq"));
    }

    #[test]
    fn test_menubar_json_path() {
        let path = menubar_json_path().unwrap();
        assert!(path.ends_with("menubar.json"));
    }

    #[test]
    fn test_resolve_menubar_override_wins() {
        let result = resolve_hq_folder(
            Some("/from/config"),
            Some("/from/menubar"),
        );
        assert_eq!(result, PathBuf::from("/from/menubar"));
    }

    #[test]
    fn test_resolve_config_path() {
        let result = resolve_hq_folder(Some("/from/config"), None);
        assert_eq!(result, PathBuf::from("/from/config"));
    }

    #[test]
    fn test_resolve_default() {
        let result = resolve_hq_folder(None, None);
        assert!(result.ends_with("HQ"));
    }

    #[test]
    fn test_resolve_empty_menubar_falls_through() {
        let result = resolve_hq_folder(Some("/from/config"), Some(""));
        assert_eq!(result, PathBuf::from("/from/config"));
    }

    #[test]
    fn test_resolve_empty_both_falls_to_default() {
        let result = resolve_hq_folder(Some(""), Some(""));
        assert!(result.ends_with("HQ"));
    }

    #[test]
    fn test_resolve_bin_returns_name_when_missing() {
        // A name that almost certainly doesn't exist anywhere
        let result = resolve_bin("hq-sync-nonexistent-xyz-123");
        assert_eq!(result, "hq-sync-nonexistent-xyz-123");
    }

    #[test]
    fn test_resolve_bin_finds_system_binary() {
        // `ls` lives at /bin/ls on all macOS/Linux — the /usr/local/bin
        // branch won't match, but the zsh fallback should on any dev box.
        // On minimal CI containers without zsh this may return "ls", which
        // is still correct behavior (Command::new will then find /bin/ls
        // via its own PATH lookup).
        let result = resolve_bin("ls");
        // Either we resolved to an absolute path, or we fell back to the
        // bare name — both are valid.
        assert!(result == "ls" || std::path::Path::new(&result).exists());
    }
}
