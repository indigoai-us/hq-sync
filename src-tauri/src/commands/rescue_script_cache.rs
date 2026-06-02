//! Live-fetch fallback for the bundled `scripts/replace-rescue.sh`.
//!
//! ## Why this exists
//!
//! The rescue script ships inside `HQ Sync.app/Contents/Resources/_up_/scripts/`,
//! declared via `bundle.resources` in `tauri.conf.json`. In normal operation
//! `resolve_rescue_script` finds it via `BaseDirectory::Resource` and we're
//! done.
//!
//! In practice we've observed installs where the Tauri auto-updater swapped
//! the application executable but left `Resources/_up_/scripts/` holding a
//! pre-rename copy (the `replace-rescue.sh` rename landed in commit cebf307
//! between releases v0.1.106 and v0.1.107). The Rust binary then looks for
//! `replace-rescue.sh`, the bundle still has `replace-from-staging-rescue.sh`,
//! and the prod "Update to vX.Y.Z" CTA exits with `replace-rescue.sh not
//! found in resource dir`.
//!
//! Rather than try to repair Tauri's bundle-swap behavior (which we don't
//! control), this module guarantees the script is reachable by downloading
//! the matching version from `raw.githubusercontent.com` into a local
//! cache the first time it's needed. Subsequent invocations hit the cache
//! and skip the network round-trip.
//!
//! ## Cache layout
//!
//! ```text
//! $HOME/.hq/cache/hq-sync/scripts/replace-rescue-v{app_version}.sh
//! ```
//!
//! The cache key includes the app version so a future menubar upgrade that
//! ships a different rescue-script revision re-downloads cleanly. Old
//! cached versions stay on disk but become unreachable — see
//! [`prune_cache`] for an opt-in cleanup helper.
//!
//! ## Network
//!
//! `indigoai-us/hq-sync` is a public repo, so no auth header is required
//! and we lean on `raw.githubusercontent.com`'s CDN. The fetch tries the
//! tag matching `app_version` first (`v{version}/scripts/replace-rescue.sh`)
//! and falls back to `main` if the tag 404s — covers dev builds, alpha
//! versions, and any window where a release is built but its tag hasn't
//! pushed yet.

use std::path::{Path, PathBuf};

/// Public repo that owns the rescue script.
pub(crate) const SCRIPT_REPO: &str = "indigoai-us/hq-sync";

/// Script path inside the repo.
pub(crate) const SCRIPT_PATH: &str = "scripts/replace-rescue.sh";

/// Resolve the cache file for a given app version.
///
/// Pure function — derives a path under `$HOME/.hq/cache/hq-sync/scripts/`.
/// Does NOT touch the filesystem. Falls back to `/tmp` when the home dir
/// is missing (CI containers without `$HOME`) so callers always get a
/// usable path.
pub(crate) fn cached_rescue_script_path(home: Option<&Path>, app_version: &str) -> PathBuf {
    let base = home
        .map(|h| h.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(".hq")
        .join("cache")
        .join("hq-sync")
        .join("scripts")
        .join(format!("replace-rescue-v{app_version}.sh"))
}

/// Tagged URL for a given app version (preferred).
pub(crate) fn rescue_script_url_for_tag(app_version: &str) -> String {
    format!("https://raw.githubusercontent.com/{SCRIPT_REPO}/v{app_version}/{SCRIPT_PATH}")
}

/// Fallback URL on the default branch when the tag doesn't resolve.
pub(crate) fn rescue_script_url_main() -> String {
    format!("https://raw.githubusercontent.com/{SCRIPT_REPO}/main/{SCRIPT_PATH}")
}

/// Outcome of an `ensure_cached_rescue_script` call. Carried back to
/// callers (and logged) so the popover can show whether the script came
/// from bundle, cache, or a live download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CacheSource {
    /// File was already on disk at the expected cache path.
    CacheHit,
    /// File was just downloaded and written to the cache.
    Downloaded { url: String },
}

/// Ensure a rescue-script copy exists at the cache path.
///
/// `fetcher` is injected so unit tests can simulate cache-miss, network
/// failure, and tag-404-then-main-success without touching the network.
/// Production callers pass a real reqwest-backed closure.
///
/// On cache hit: returns the existing path; `fetcher` is not invoked.
///
/// On cache miss: tries the tagged URL first, then `main`. On the first
/// successful fetch, writes the body to disk, chmods +x, and returns the
/// path. If both fail, returns the last error.
pub(crate) async fn ensure_cached_rescue_script<F, Fut>(
    home: Option<&Path>,
    app_version: &str,
    fetcher: F,
) -> Result<(PathBuf, CacheSource), String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, String>>,
{
    let cache_path = cached_rescue_script_path(home, app_version);

    if cache_path.is_file() {
        return Ok((cache_path, CacheSource::CacheHit));
    }

    let parent = cache_path
        .parent()
        .ok_or_else(|| format!("no parent dir for cache path {cache_path:?}"))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir cache dir {parent:?}: {e}"))?;

    let tag_url = rescue_script_url_for_tag(app_version);
    let main_url = rescue_script_url_main();
    let attempted_urls = [tag_url.clone(), main_url.clone()];

    let mut last_err: Option<String> = None;
    for url in &attempted_urls {
        match fetcher(url.clone()).await {
            Ok(body) => {
                if body.is_empty() {
                    last_err = Some(format!("GET {url}: empty body"));
                    continue;
                }
                std::fs::write(&cache_path, &body)
                    .map_err(|e| format!("write cache {cache_path:?}: {e}"))?;
                set_executable(&cache_path)?;
                return Ok((cache_path, CacheSource::Downloaded { url: url.clone() }));
            }
            Err(e) => {
                last_err = Some(e);
            }
        }
    }

    Err(format!(
        "live-fetch rescue script failed (tried {attempted_urls:?}): {}",
        last_err.unwrap_or_else(|| "no error captured".to_string())
    ))
}

/// Mark the cached script executable. Unix-only; on other platforms this
/// is a no-op because the file is exec'd via `bash`/`sh` rather than
/// directly.
fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .map_err(|e| format!("stat {path:?}: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).map_err(|e| format!("chmod +x {path:?}: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path; // suppress unused warning on non-Unix targets
    }
    Ok(())
}

/// Best-effort cleanup of stale cached scripts. Caller chooses the
/// retention policy — this helper just walks the cache dir and removes
/// any `replace-rescue-v*.sh` entry that doesn't match `keep_version`.
/// Errors are logged at the call site, not propagated.
#[allow(dead_code)]
pub(crate) fn prune_cache(home: Option<&Path>, keep_version: &str) -> std::io::Result<()> {
    let dir = cached_rescue_script_path(home, keep_version)
        .parent()
        .map(|p| p.to_path_buf());
    let Some(dir) = dir else { return Ok(()) };
    if !dir.is_dir() {
        return Ok(());
    }
    let keep_file = format!("replace-rescue-v{keep_version}.sh");
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str == keep_file {
            continue;
        }
        if name_str.starts_with("replace-rescue-v") && name_str.ends_with(".sh") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn fake_home() -> TempDir {
        tempfile::tempdir().expect("create tempdir")
    }

    #[test]
    fn cached_path_under_home() {
        let home = PathBuf::from("/Users/somebody");
        let p = cached_rescue_script_path(Some(&home), "0.5.0");
        assert_eq!(
            p,
            PathBuf::from("/Users/somebody/.hq/cache/hq-sync/scripts/replace-rescue-v0.5.0.sh")
        );
    }

    #[test]
    fn cached_path_falls_back_to_tmp_when_no_home() {
        let p = cached_rescue_script_path(None, "0.5.0");
        assert_eq!(
            p,
            PathBuf::from("/tmp/.hq/cache/hq-sync/scripts/replace-rescue-v0.5.0.sh")
        );
    }

    #[test]
    fn cached_path_handles_beta_version_strings() {
        // Real release tags include suffixes like `-beta.3`. The cache
        // filename must survive those characters without surprises (no
        // shell-quoting needed; they live as plain filesystem chars).
        let home = PathBuf::from("/h");
        let p = cached_rescue_script_path(Some(&home), "0.4.4-beta.3");
        assert!(p.ends_with("replace-rescue-v0.4.4-beta.3.sh"), "got {p:?}");
    }

    #[test]
    fn tag_url_uses_v_prefix() {
        assert_eq!(
            rescue_script_url_for_tag("0.5.0"),
            "https://raw.githubusercontent.com/indigoai-us/hq-sync/v0.5.0/scripts/replace-rescue.sh"
        );
    }

    #[test]
    fn main_url_is_stable() {
        assert_eq!(
            rescue_script_url_main(),
            "https://raw.githubusercontent.com/indigoai-us/hq-sync/main/scripts/replace-rescue.sh"
        );
    }

    #[tokio::test]
    async fn cache_hit_returns_path_without_fetching() {
        let tmp = fake_home();
        let target = cached_rescue_script_path(Some(tmp.path()), "1.2.3");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"#!/usr/bin/env bash\necho cached\n").unwrap();

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fc = fetch_count.clone();
        let fetcher = move |_url: String| {
            let fc = fc.clone();
            async move {
                fc.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(b"should not be called".to_vec())
            }
        };

        let (path, source) = ensure_cached_rescue_script(Some(tmp.path()), "1.2.3", fetcher)
            .await
            .expect("cache hit");
        assert_eq!(path, target);
        assert_eq!(source, CacheSource::CacheHit);
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            0,
            "fetcher must not run on cache hit"
        );
    }

    #[tokio::test]
    async fn cache_miss_downloads_and_writes() {
        let tmp = fake_home();
        let body = b"#!/usr/bin/env bash\necho live-fetched\n";

        let fetcher = move |_url: String| async move { Ok::<_, String>(body.to_vec()) };

        let (path, source) = ensure_cached_rescue_script(Some(tmp.path()), "9.9.9", fetcher)
            .await
            .expect("download ok");
        assert!(path.is_file(), "cache file must exist after download");
        assert_eq!(std::fs::read(&path).unwrap(), body);
        match source {
            CacheSource::Downloaded { url } => {
                assert!(
                    url.contains("v9.9.9"),
                    "should pick tag URL first, got {url}"
                );
            }
            other => panic!("expected Downloaded, got {other:?}"),
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "cached script must be executable");
        }
    }

    #[tokio::test]
    async fn falls_back_to_main_when_tag_404s() {
        let tmp = fake_home();
        let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let c = calls.clone();

        let fetcher = move |url: String| {
            let c = c.clone();
            async move {
                c.lock().unwrap().push(url.clone());
                if url.contains("/v0.0.0-nope/") {
                    Err::<Vec<u8>, String>("404 Not Found".to_string())
                } else {
                    Ok(b"#!/usr/bin/env bash\necho main\n".to_vec())
                }
            }
        };

        let (_path, source) = ensure_cached_rescue_script(Some(tmp.path()), "0.0.0-nope", fetcher)
            .await
            .expect("falls back to main");
        match source {
            CacheSource::Downloaded { url } => {
                assert!(
                    url.ends_with("/main/scripts/replace-rescue.sh"),
                    "got {url}"
                );
            }
            other => panic!("expected Downloaded, got {other:?}"),
        }
        let seen = calls.lock().unwrap();
        assert_eq!(seen.len(), 2, "must try both URLs");
        assert!(seen[0].contains("/v0.0.0-nope/"));
        assert!(seen[1].ends_with("/main/scripts/replace-rescue.sh"));
    }

    #[tokio::test]
    async fn both_urls_failing_returns_combined_error() {
        let tmp = fake_home();
        let fetcher =
            |_url: String| async move { Err::<Vec<u8>, String>("connection refused".to_string()) };

        let err = ensure_cached_rescue_script(Some(tmp.path()), "1.0.0", fetcher)
            .await
            .expect_err("must fail when both URLs fail");
        assert!(err.contains("live-fetch rescue script failed"), "got {err}");
        assert!(err.contains("v1.0.0"), "must mention tag URL");
        assert!(err.contains("/main/"), "must mention main URL");
        assert!(
            err.contains("connection refused"),
            "must include underlying error"
        );
    }

    #[tokio::test]
    async fn empty_body_is_treated_as_failure() {
        let tmp = fake_home();
        let fetcher = |_url: String| async move { Ok::<_, String>(Vec::new()) };

        let err = ensure_cached_rescue_script(Some(tmp.path()), "1.0.0", fetcher)
            .await
            .expect_err("empty body must fail");
        assert!(err.contains("empty body"), "got {err}");
    }

    #[tokio::test]
    async fn prune_removes_other_versions_keeps_current() {
        let tmp = fake_home();
        let scripts_dir = cached_rescue_script_path(Some(tmp.path()), "x")
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::create_dir_all(&scripts_dir).unwrap();
        let keep = scripts_dir.join("replace-rescue-v2.0.0.sh");
        let stale1 = scripts_dir.join("replace-rescue-v1.0.0.sh");
        let stale2 = scripts_dir.join("replace-rescue-v1.5.0.sh");
        let unrelated = scripts_dir.join("notes.txt");
        std::fs::write(&keep, b"keep").unwrap();
        std::fs::write(&stale1, b"stale").unwrap();
        std::fs::write(&stale2, b"stale").unwrap();
        std::fs::write(&unrelated, b"unrelated").unwrap();

        prune_cache(Some(tmp.path()), "2.0.0").expect("prune ok");

        assert!(keep.is_file(), "must keep current version");
        assert!(!stale1.exists(), "must remove stale v1.0.0");
        assert!(!stale2.exists(), "must remove stale v1.5.0");
        assert!(unrelated.is_file(), "must leave unrelated files alone");
    }
}
