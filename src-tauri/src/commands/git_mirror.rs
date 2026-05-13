//! After-sync git mirror.
//!
//! When the HQ folder root is itself a git repo, commit any local changes
//! and push to the tracked upstream (if any) so the user's HQ doubles as a
//! versioned snapshot. Triggered fire-and-forget from the AllComplete arms
//! of both the manual sync (`commands/sync.rs`) and the auto-sync watcher
//! (`commands/daemon.rs`).
//!
//! All output goes to the persistent diagnostic log under the `git-mirror`
//! tag — never to the popover. The HQ sync itself is authoritative; a git
//! mirror failure must never block sync.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::Mutex;

use chrono::SecondsFormat;

use crate::util::logfile::log;

const LOG_TAG: &str = "git-mirror";

/// Guards against overlapping mirror runs. The auto-sync watcher fires
/// AllComplete every 10 minutes; on a slow network a single push could run
/// longer than that. `try_lock` lets the second pass skip rather than
/// race a still-running `git push`, and the guard auto-releases on scope
/// exit so a panic mid-run never strands the lock.
static MIRROR_LOCK: Mutex<()> = Mutex::new(());

/// Spawn the mirror on a background thread so the AllComplete handler
/// returns immediately and the sync stdout reader keeps draining.
pub fn spawn_mirror_after_sync(hq_folder: &str) {
    let hq_folder = hq_folder.to_string();
    std::thread::spawn(move || {
        mirror_after_sync(&hq_folder);
    });
}

/// Synchronous entry point. Returns immediately if `<hq_folder>/.git` is
/// absent or if a previous mirror is still running. Never panics, never
/// propagates errors — everything ends up in the log under `git-mirror`.
pub fn mirror_after_sync(hq_folder: &str) {
    if !Path::new(hq_folder).join(".git").exists() {
        return;
    }
    let _guard = match MIRROR_LOCK.try_lock() {
        Ok(g) => g,
        Err(_) => {
            log(
                LOG_TAG,
                &format!("{hq_folder}: previous mirror still in flight, skipping"),
            );
            return;
        }
    };
    if let Err(e) = run_mirror(hq_folder) {
        log(LOG_TAG, &format!("{hq_folder}: {e}"));
    }
}

fn run_mirror(hq_folder: &str) -> Result<(), String> {
    run_git(hq_folder, &["add", "-A"])?;

    // `diff --cached --quiet` exits 0 when index == HEAD, 1 when staged
    // changes exist. Anything else is unexpected (signal, missing HEAD on
    // a brand-new repo, etc.) and gets logged but isn't fatal.
    let staged = git_output(hq_folder, &["diff", "--cached", "--quiet"])?;
    match staged.status.code() {
        Some(0) => {
            log(LOG_TAG, &format!("{hq_folder}: nothing to commit"));
            return Ok(());
        }
        Some(1) => {} // staged changes — proceed to commit
        Some(code) => {
            return Err(format!(
                "git diff --cached unexpected exit {code}: {}",
                String::from_utf8_lossy(&staged.stderr).trim()
            ));
        }
        None => return Err("git diff --cached killed by signal".to_string()),
    }

    // ISO-8601 to the second; sortable in `git log` without quoting issues.
    let now_iso = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let msg = format!("hq-sync: {now_iso}");
    run_git(hq_folder, &["commit", "-m", &msg])?;
    log(LOG_TAG, &format!("{hq_folder}: committed \"{msg}\""));

    // No upstream → skip push. Covers detached HEAD, never-pushed branches,
    // and one-off forks. User runs `git push -u` once; later syncs push.
    let upstream = git_output(hq_folder, &["rev-parse", "--abbrev-ref", "@{u}"])?;
    if upstream.status.success() {
        run_git(hq_folder, &["push"])?;
        log(LOG_TAG, &format!("{hq_folder}: push ok"));
    } else {
        log(
            LOG_TAG,
            &format!("{hq_folder}: no upstream, skipping push"),
        );
    }

    Ok(())
}

fn run_git(cwd: &str, args: &[&str]) -> Result<(), String> {
    let out = git_output(cwd, args)?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed (exit {}): {}",
            args.join(" "),
            out.status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

fn git_output(cwd: &str, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|e| format!("spawn git: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git available in test env")
    }

    fn init_repo(dir: &Path) {
        assert!(git(dir, &["init", "-q", "-b", "main"]).status.success());
        // Test env may have no global git identity; pin one locally so
        // `git commit` doesn't bail with "Please tell me who you are".
        assert!(git(dir, &["config", "user.email", "test@example.com"])
            .status
            .success());
        assert!(git(dir, &["config", "user.name", "hq-sync-test"])
            .status
            .success());
        // Disable any inherited commit hooks/templates — keep the test
        // environment hermetic regardless of the dev's global ~/.gitconfig.
        assert!(git(dir, &["config", "commit.gpgsign", "false"])
            .status
            .success());
    }

    fn rev_count(dir: &Path) -> usize {
        let out = git(dir, &["rev-list", "--count", "HEAD"]);
        if !out.status.success() {
            return 0;
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }

    /// Most tests bypass `mirror_after_sync` and call `run_mirror` directly
    /// so the process-wide `MIRROR_LOCK` doesn't make parallel cargo-test
    /// threads race each other. The single test that does exercise the
    /// outer entry point only hits the no-`.git` early-return, which doesn't
    /// touch the lock.

    #[test]
    fn no_git_dir_is_noop() {
        let tmp = TempDir::new().unwrap();
        // Should not panic, should not create anything.
        mirror_after_sync(tmp.path().to_str().unwrap());
        assert!(!tmp.path().join(".git").exists());
    }

    #[test]
    fn no_changes_means_no_commit() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        // Seed an initial commit so HEAD exists.
        fs::write(tmp.path().join("README"), "seed").unwrap();
        assert!(git(tmp.path(), &["add", "-A"]).status.success());
        assert!(git(tmp.path(), &["commit", "-q", "-m", "seed"])
            .status
            .success());

        let before = rev_count(tmp.path());
        run_mirror(tmp.path().to_str().unwrap()).expect("mirror ok");
        let after = rev_count(tmp.path());
        assert_eq!(before, after, "no-change mirror must not add commits");
    }

    #[test]
    fn untracked_file_is_committed() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("README"), "seed").unwrap();
        assert!(git(tmp.path(), &["add", "-A"]).status.success());
        assert!(git(tmp.path(), &["commit", "-q", "-m", "seed"])
            .status
            .success());
        let before = rev_count(tmp.path());

        fs::write(tmp.path().join("new-file.txt"), "hello").unwrap();
        run_mirror(tmp.path().to_str().unwrap()).expect("mirror ok");

        let after = rev_count(tmp.path());
        assert_eq!(after, before + 1, "expected exactly one new commit");

        let log_out = git(tmp.path(), &["log", "-1", "--pretty=%s"]);
        let subject = String::from_utf8_lossy(&log_out.stdout);
        assert!(
            subject.starts_with("hq-sync: "),
            "expected `hq-sync: <iso>` subject, got: {subject}"
        );
    }

    #[test]
    fn modified_tracked_file_is_committed() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        let f = tmp.path().join("README");
        fs::write(&f, "seed").unwrap();
        assert!(git(tmp.path(), &["add", "-A"]).status.success());
        assert!(git(tmp.path(), &["commit", "-q", "-m", "seed"])
            .status
            .success());
        let before = rev_count(tmp.path());

        fs::write(&f, "edited").unwrap();
        run_mirror(tmp.path().to_str().unwrap()).expect("mirror ok");

        assert_eq!(rev_count(tmp.path()), before + 1);
    }

    #[test]
    fn no_upstream_means_commit_without_push() {
        // Pin the contract explicitly: with no remote configured, the
        // mirror still commits locally and reports success.
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("README"), "seed").unwrap();
        assert!(git(tmp.path(), &["add", "-A"]).status.success());
        assert!(git(tmp.path(), &["commit", "-q", "-m", "seed"])
            .status
            .success());
        let before = rev_count(tmp.path());

        // No `git remote add`, no upstream branch.
        fs::write(tmp.path().join("x"), "y").unwrap();
        run_mirror(tmp.path().to_str().unwrap()).expect("mirror ok");
        assert_eq!(rev_count(tmp.path()), before + 1);
    }

    #[test]
    fn pushes_to_configured_upstream() {
        let work = TempDir::new().unwrap();
        let remote = TempDir::new().unwrap();
        // Bare repo acts as the remote so `git push` has somewhere to land.
        assert!(Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(remote.path())
            .output()
            .expect("git available")
            .status
            .success());

        init_repo(work.path());
        let remote_url = remote.path().to_str().unwrap();
        assert!(git(work.path(), &["remote", "add", "origin", remote_url])
            .status
            .success());
        fs::write(work.path().join("README"), "seed").unwrap();
        assert!(git(work.path(), &["add", "-A"]).status.success());
        assert!(git(work.path(), &["commit", "-q", "-m", "seed"])
            .status
            .success());
        assert!(git(work.path(), &["push", "-q", "-u", "origin", "main"])
            .status
            .success());

        fs::write(work.path().join("new"), "data").unwrap();
        run_mirror(work.path().to_str().unwrap()).expect("mirror ok");

        // Remote (bare repo) should now have the same HEAD as local.
        let local_head = String::from_utf8(
            git(work.path(), &["rev-parse", "HEAD"]).stdout,
        )
        .unwrap();
        let remote_head = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(remote.path())
                .args(["rev-parse", "main"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        assert_eq!(local_head.trim(), remote_head.trim());
    }
}
