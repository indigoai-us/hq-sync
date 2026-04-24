use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

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

/// Resolve the bin directory of the Node the user's login shell picks.
///
/// Asks `zsh -lc 'command -v node'` so the user's startup chain (`.zshrc`,
/// nvm/volta/asdf activation, etc.) runs and PATH points at the *currently
/// active* Node — the same one they had when they ran `npm i -g @tobilu/qmd`
/// and linked its native modules against a specific ABI. Returns `None` when
/// zsh isn't available or no Node resolves.
///
/// This replaces the old `child_path` behaviour of blindly prepending every
/// `~/.nvm/versions/node/*/bin` in directory-listing order. With multiple
/// installed Node versions, that order is alphabetical (non-deterministic
/// from the user's perspective), so the child could inherit a PATH where
/// `env node` hit a version that didn't match the one `qmd` / `hq-sync-runner`
/// was compiled against — surfacing as
/// `NODE_MODULE_VERSION N vs M` ABI mismatches. See the bug report on
/// hq-sync#14 / hq-installer#34 codex review.
pub fn active_node_bin_dir() -> Option<String> {
    let out = Command::new("zsh")
        .args(["-lc", "command -v node"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let node_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if node_path.is_empty() {
        return None;
    }
    let p = Path::new(&node_path);
    if !p.exists() {
        return None;
    }
    p.parent().map(|d| d.to_string_lossy().to_string())
}

/// Build a PATH value suitable for handing to a spawned child process.
///
/// **Why this exists:** even after we resolve a launcher binary to an absolute
/// path via `resolve_bin`, the *child itself* still inherits the parent's
/// PATH. Node-backed CLIs use `#!/usr/bin/env node` shebangs — `env` does a
/// PATH lookup for `node`. Under the minimal launchd PATH a Dock-launched
/// Tauri app inherits, that lookup fails and the child exits with 127
/// ("command not found"). Same applies to anything the script itself spawns.
///
/// **Ordering strategy:** put the user's *active* Node (the one their login
/// shell would resolve) first, then standard install locations, then
/// whatever the parent process had. This ensures native-module packages
/// (qmd → better-sqlite3, hq-sync-runner) see the exact Node they were
/// compiled against — no ABI roulette from mixing nvm versions.
///
/// Order: active nvm/volta/asdf node → `~/.npm-global/bin` →
/// `/opt/homebrew/bin` → `/usr/local/bin` → system defaults → parent PATH.
///
/// The result is memoised in a `OnceLock` because the active Node resolution
/// spawns a login shell (~100ms). For the app lifetime that's fine: the
/// user's default Node doesn't change between invocations of the same
/// running app.
pub fn child_path() -> String {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(compute_child_path).clone()
}

fn compute_child_path() -> String {
    let mut parts: Vec<String> = Vec::new();

    // Invariant: `parts` never contains duplicates. `env node` uses
    // first-match semantics so duplicates don't change behaviour, but
    // trimming them keeps the PATH readable in logs and tests.
    let push_if_new = |v: String, parts: &mut Vec<String>| {
        if !v.is_empty() && !parts.iter().any(|x| x == &v) {
            parts.push(v);
        }
    };

    // 1. Active Node (resolves nvm/volta/asdf through the user's shell).
    if let Some(node_bin) = active_node_bin_dir() {
        push_if_new(node_bin, &mut parts);
    }

    // 2. User-level npm prefix (no-sudo installs).
    if let Some(home) = dirs::home_dir() {
        let npm_global = home.join(".npm-global").join("bin");
        if npm_global.exists() {
            push_if_new(npm_global.to_string_lossy().to_string(), &mut parts);
        }
    }

    // 3. Standard install locations + system dirs.
    for p in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        push_if_new(p.to_string(), &mut parts);
    }

    // 4. Preserve anything the parent already had.
    if let Ok(existing) = std::env::var("PATH") {
        for p in existing.split(':') {
            push_if_new(p.to_string(), &mut parts);
        }
    }

    parts.join(":")
}

/// Returns the path to ~/.hq/config.json.
pub fn config_json_path() -> Result<PathBuf, String> {
    Ok(hq_config_dir()?.join("config.json"))
}

/// Returns the path to ~/.hq/menubar.json.
pub fn menubar_json_path() -> Result<PathBuf, String> {
    Ok(hq_config_dir()?.join("menubar.json"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Embeddings handoff paths (US-002)
// ─────────────────────────────────────────────────────────────────────────────

/// Journal path at `{hq_folder}/.hq-embeddings-journal.json`. Kept next to
/// the sync journal so the support snippet is the same shape:
/// `cat ~/HQ/.hq-embeddings-journal.json`.
pub fn embeddings_journal_path(hq_folder: &Path) -> PathBuf {
    hq_folder.join(".hq-embeddings-journal.json")
}

/// Candidate pending-marker paths, in check order:
///   1. `{hq_folder}/.hq-embeddings-pending.json` — preferred, written by the
///      installer when `installPath` resolves to an absolute canonical path.
///   2. `~/.hq/embeddings-pending.json` — fallback written by the installer
///      when it couldn't resolve `installPath`.
///
/// Sync's auto-trigger needs to check both; success cleans both. Never
/// returns the `~` path if the home directory can't be resolved.
pub fn embeddings_pending_paths(hq_folder: &Path) -> Vec<PathBuf> {
    let mut out = vec![hq_folder.join(".hq-embeddings-pending.json")];
    if let Ok(hq_dir) = hq_config_dir() {
        out.push(hq_dir.join("embeddings-pending.json"));
    }
    out
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
    fn test_child_path_includes_homebrew() {
        let path = child_path();
        assert!(path.contains("/opt/homebrew/bin"));
        assert!(path.contains("/usr/local/bin"));
        assert!(path.contains("/usr/bin"));
    }

    #[test]
    fn test_child_path_preserves_existing() {
        // Whatever PATH the test runner has, child_path should include its entries.
        if let Ok(existing) = std::env::var("PATH") {
            if let Some(first) = existing.split(':').next() {
                if !first.is_empty() {
                    let path = child_path();
                    assert!(path.contains(first), "child_path dropped existing entry {}", first);
                }
            }
        }
    }

    #[test]
    fn test_child_path_puts_active_node_first_when_resolvable() {
        // Regression test for the ABI mismatch bug: previously child_path()
        // iterated `~/.nvm/versions/node/*/bin` in directory-listing order
        // and prepended every version; `env node` then bound to whichever
        // sorted first alphabetically, which was often the wrong one.
        //
        // The new implementation asks the user's login shell for the
        // *single* active Node (`zsh -lc 'command -v node'`) and puts its
        // bin dir first. We verify that invariant here: if the active Node
        // resolves, it must be the FIRST entry in child_path() so `env
        // node` finds it before any other Node binaries the parent PATH
        // may happen to contain.
        //
        // When zsh isn't available or no node resolves, `active_node_bin_dir`
        // returns None and this test is a no-op — the behavioural fix is
        // still in place, it just has nothing to assert on this host.
        let path = child_path();
        if let Some(active) = active_node_bin_dir() {
            let first = path.split(':').next().unwrap_or("");
            assert_eq!(
                first, active,
                "active node bin dir must be first in child_path() — got:\n  first:  {}\n  active: {}\n  full:   {}",
                first, active, path
            );
        }
    }

    #[test]
    fn test_child_path_dedupes_entries() {
        // The forward-path construction must not emit the same directory
        // twice — this matters when the active-Node resolver returns the
        // same dir as one of the standard system locations (e.g. Homebrew
        // `/opt/homebrew/bin/node` on a machine without nvm). Duplicates
        // don't change `env node` behaviour but they bloat logs and make
        // the PATH harder to audit.
        let path = child_path();
        let mut seen: Vec<&str> = Vec::new();
        for entry in path.split(':') {
            assert!(
                !seen.contains(&entry),
                "child_path contains duplicate entry `{}`:\n  {}",
                entry,
                path
            );
            seen.push(entry);
        }
    }

    #[test]
    fn test_active_node_bin_dir_returns_existing_directory_when_some() {
        // Whatever active_node_bin_dir() returns (including None), we don't
        // want to hand the child process a path that doesn't exist on disk
        // — that would turn an ABI mismatch into a silent "command not
        // found" with the same user symptoms and worse diagnostics.
        if let Some(dir) = active_node_bin_dir() {
            assert!(
                std::path::Path::new(&dir).exists(),
                "active_node_bin_dir returned a non-existent path: {}",
                dir
            );
            // The returned dir must contain a `node` executable — otherwise
            // we just polluted PATH without actually fixing shebang lookup.
            let node_bin = std::path::Path::new(&dir).join("node");
            assert!(
                node_bin.exists(),
                "active_node_bin_dir returned {} but {}/node does not exist",
                dir,
                dir
            );
        }
    }

    // ── Embeddings handoff paths (US-002) ────────────────────────────────────

    #[test]
    fn test_embeddings_journal_path_joins_dotfile() {
        let p = embeddings_journal_path(Path::new("/tmp/hq"));
        assert_eq!(p, PathBuf::from("/tmp/hq/.hq-embeddings-journal.json"));
    }

    #[test]
    fn test_embeddings_pending_paths_first_is_hq_folder_primary() {
        let paths = embeddings_pending_paths(Path::new("/tmp/hq"));
        assert_eq!(
            paths.first().unwrap(),
            &PathBuf::from("/tmp/hq/.hq-embeddings-pending.json")
        );
    }

    #[test]
    fn test_embeddings_pending_paths_fallback_in_home_dotdir_when_resolvable() {
        let paths = embeddings_pending_paths(Path::new("/tmp/hq"));
        // When home dir resolves (the standard case on macOS/Linux),
        // the fallback path is included as the second entry and lives under
        // `~/.hq/`. If hq_config_dir fails (unusual), only the primary is
        // returned — still safe, just less thorough.
        if paths.len() >= 2 {
            assert!(
                paths[1].ends_with("embeddings-pending.json"),
                "fallback path should end with embeddings-pending.json: {:?}",
                paths[1]
            );
            assert!(
                paths[1]
                    .parent()
                    .map(|p| p.ends_with(".hq"))
                    .unwrap_or(false),
                "fallback parent dir should be `.hq`: {:?}",
                paths[1]
            );
        }
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
