//! Conflict resolution commands — resolve file conflicts and open in editor.

use std::process::Command;
use std::time::Duration;

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::util::paths;

/// CLI command timeout (10 seconds).
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Valid resolution strategies.
const VALID_STRATEGIES: &[&str] = &["keep-local", "keep-remote"];

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (same pattern as sync.rs / status.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the HQ folder path by reading config.json and menubar.json directly.
fn resolve_hq_folder_path() -> Result<String, String> {
    let config_path = paths::config_json_path()?;
    let menubar_path = paths::menubar_json_path()?;

    let menubar_prefs: Option<MenubarPrefs> = if menubar_path.exists() {
        std::fs::read_to_string(&menubar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    let config: Option<HqConfig> = if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.json: {}", e))?;
        Some(
            serde_json::from_str(&contents)
                .map_err(|e| format!("Failed to parse config.json: {}", e))?,
        )
    } else {
        None
    };

    let hq_folder = paths::resolve_hq_folder(
        config
            .as_ref()
            .and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs
            .as_ref()
            .and_then(|p| p.hq_path.as_deref()),
    );

    Ok(hq_folder.to_string_lossy().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Strategy validation (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Validate that a strategy string is one of the accepted values.
pub fn validate_strategy(strategy: &str) -> Result<(), String> {
    if VALID_STRATEGIES.contains(&strategy) {
        Ok(())
    } else {
        Err(format!(
            "Unknown strategy '{}'. Must be one of: {}",
            strategy,
            VALID_STRATEGIES.join(", ")
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI args builder (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Build the CLI args for `hq sync resolve`.
pub fn build_resolve_args(strategy: &str, path: &str, hq_folder: &str) -> Vec<String> {
    vec![
        "sync".to_string(),
        "resolve".to_string(),
        "--strategy".to_string(),
        strategy.to_string(),
        "--path".to_string(),
        path.to_string(),
        "--hq-path".to_string(),
        hq_folder.to_string(),
    ]
}

/// Build the full file path from HQ folder and relative path.
/// Returns an error if the resolved path escapes the HQ folder (path traversal).
pub fn build_full_path(hq_folder: &str, relative_path: &str) -> Result<String, String> {
    let mut full = std::path::PathBuf::from(hq_folder);
    full.push(relative_path);
    let full_str = full.to_string_lossy().to_string();

    // Canonicalize both paths to resolve .. and symlinks, then verify containment
    let hq_canon = std::path::PathBuf::from(hq_folder)
        .canonicalize()
        .map_err(|e| format!("Invalid HQ folder: {}", e))?;
    let full_canon = full
        .canonicalize()
        .map_err(|e| format!("Invalid path '{}': {}", full_str, e))?;

    if !full_canon.starts_with(&hq_canon) {
        return Err(format!(
            "Path '{}' escapes HQ folder",
            relative_path
        ));
    }

    Ok(full_canon.to_string_lossy().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve a file conflict using the specified strategy.
///
/// - `strategy` must be `"keep-local"` or `"keep-remote"`.
/// - Runs `hq sync resolve --strategy {strategy} --path {path} --hq-path {hq_folder}`.
/// - Times out after 10 seconds; the child process is killed if it exceeds this.
#[tauri::command]
pub fn resolve_conflict(path: String, strategy: String) -> Result<(), String> {
    validate_strategy(&strategy)?;

    let hq_folder = resolve_hq_folder_path()?;
    let args = build_resolve_args(&strategy, &path, &hq_folder);

    #[cfg(debug_assertions)]
    eprintln!("[conflicts] resolving {} with strategy {}", path, strategy);

    let mut child = Command::new(paths::resolve_bin("hq"))
        .args(&args)
        .env("HQ_ROOT", &hq_folder)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn hq CLI: {}", e))?;

    // Wait with timeout — kill the process if it takes too long
    let start = std::time::Instant::now();
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= RESOLVE_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait(); // reap zombie
                    return Err("hq sync resolve timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("Failed to wait for hq CLI: {}", e)),
        }
    };

    if !exit_status.success() {
        let mut stderr_buf = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            use std::io::Read;
            let _ = stderr.read_to_string(&mut stderr_buf);
        }
        return Err(format!(
            "hq sync resolve exited with code {}: {}",
            exit_status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            stderr_buf.trim()
        ));
    }

    Ok(())
}

/// Open a file in the system default editor.
///
/// Resolves the HQ folder path, constructs the full path as `{hq_folder}/{path}`,
/// and uses macOS `open` command to launch the default application.
#[tauri::command]
pub fn open_in_editor(path: String) -> Result<(), String> {
    let hq_folder = resolve_hq_folder_path()?;
    let full_path = build_full_path(&hq_folder, &path)?;

    #[cfg(debug_assertions)]
    eprintln!("[conflicts] opening in editor: {}", full_path);

    let output = Command::new("open")
        .arg(&full_path)
        .output()
        .map_err(|e| format!("Failed to run open command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "open command failed with code {}: {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            stderr.trim()
        ));
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Strategy validation ─────────────────────────────────────────────

    #[test]
    fn test_validate_strategy_keep_local() {
        assert!(validate_strategy("keep-local").is_ok());
    }

    #[test]
    fn test_validate_strategy_keep_remote() {
        assert!(validate_strategy("keep-remote").is_ok());
    }

    #[test]
    fn test_validate_strategy_unknown_rejected() {
        let result = validate_strategy("merge");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown strategy 'merge'"));
    }

    #[test]
    fn test_validate_strategy_empty_rejected() {
        let result = validate_strategy("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown strategy ''"));
    }

    #[test]
    fn test_validate_strategy_case_sensitive() {
        let result = validate_strategy("Keep-Local");
        assert!(result.is_err());
    }

    // ── CLI args builder ────────────────────────────────────────────────

    #[test]
    fn test_build_resolve_args_keep_local() {
        let args = build_resolve_args("keep-local", "docs/readme.md", "/Users/test/HQ");
        assert_eq!(
            args,
            vec![
                "sync",
                "resolve",
                "--strategy",
                "keep-local",
                "--path",
                "docs/readme.md",
                "--hq-path",
                "/Users/test/HQ",
            ]
        );
    }

    #[test]
    fn test_build_resolve_args_keep_remote() {
        let args = build_resolve_args("keep-remote", "file.txt", "/tmp/hq");
        assert_eq!(
            args,
            vec![
                "sync",
                "resolve",
                "--strategy",
                "keep-remote",
                "--path",
                "file.txt",
                "--hq-path",
                "/tmp/hq",
            ]
        );
    }

    // ── Path construction + traversal protection ──────────────────────

    #[test]
    fn test_build_full_path_valid() {
        let dir = tempfile::tempdir().unwrap();
        let hq = dir.path().to_str().unwrap();
        let sub = dir.path().join("docs");
        std::fs::create_dir(&sub).unwrap();
        let file = sub.join("readme.md");
        std::fs::write(&file, "").unwrap();

        let result = build_full_path(hq, "docs/readme.md");
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with("docs/readme.md"));
    }

    #[test]
    fn test_build_full_path_traversal_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let hq = dir.path().to_str().unwrap();

        let result = build_full_path(hq, "../../etc/passwd");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("escapes HQ folder") || err.contains("Invalid path"));
    }

    #[test]
    fn test_build_full_path_nonexistent_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let hq = dir.path().to_str().unwrap();

        let result = build_full_path(hq, "nonexistent/file.txt");
        assert!(result.is_err());
    }

    // ── Timeout constant ────────────────────────────────────────────────

    #[test]
    fn test_resolve_timeout_value() {
        assert_eq!(RESOLVE_TIMEOUT, Duration::from_secs(10));
    }

    // ── Valid strategies constant ────────────────────────────────────────

    #[test]
    fn test_valid_strategies_list() {
        assert_eq!(VALID_STRATEGIES.len(), 2);
        assert!(VALID_STRATEGIES.contains(&"keep-local"));
        assert!(VALID_STRATEGIES.contains(&"keep-remote"));
    }
}
