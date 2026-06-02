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
//! and falls back to `main` **only when the tag definitively 404s** —
//! covers dev builds, alpha versions, and any window where a release is
//! built but its tag hasn't pushed yet. A transient 5xx / 403 /
//! network failure on the tagged URL aborts the resolver with that
//! error rather than silently caching `main`'s (potentially
//! newer-than-binary) script under the current version key. See
//! [`FetchOutcome`] for the classification the fetcher MUST honor.

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

/// Outcome of one fetch attempt. The fetcher MUST distinguish
/// "tag doesn't exist" from any other error so the loop only falls back
/// to `main` when we're certain the tagged ref isn't there.
///
/// Conflating the two would let a transient 5xx / 403 rate-limit /
/// network glitch on the tagged URL cache `main` content under the
/// running version key — and subsequent rescue runs would silently
/// execute a script that's newer than (and potentially incompatible
/// with) the installed binary's contract.
#[derive(Debug)]
pub(crate) enum FetchOutcome {
    /// 2xx, non-empty body — use this content.
    Body(Vec<u8>),
    /// 404 (or equivalent definitive not-found). Caller may try the
    /// next URL.
    NotFound,
    /// Anything else: 5xx, 403, network timeout, DNS failure, empty
    /// body, parse error. Caller MUST NOT fall back — abort with this
    /// error to surface the real problem instead of masking it with a
    /// silent `main` write.
    TransientError(String),
}

/// Ensure a rescue-script copy exists at the cache path.
///
/// `fetcher` is injected so unit tests can simulate cache-miss,
/// definitive-404, transient errors, and tag-404-then-main-success
/// without touching the network. Production callers pass a
/// reqwest-backed closure that classifies HTTP status into the three
/// `FetchOutcome` variants.
///
/// On cache hit: returns the existing path; `fetcher` is not invoked.
///
/// On cache miss:
///   1. Try the tagged URL (`v{app_version}/scripts/...`).
///      * `Body(b)` → write + return.
///      * `NotFound` → continue to main URL (the documented fallback case).
///      * `TransientError(e)` → abort with `e`. **Do not** fall back —
///        the tag may exist; we just couldn't reach it.
///   2. If we fell through, try `main/scripts/...`.
///      * Same three-way classification.
///   3. If both legs ended in `NotFound`, return a combined error
///      explaining neither ref resolved.
pub(crate) async fn ensure_cached_rescue_script<F, Fut>(
    home: Option<&Path>,
    app_version: &str,
    fetcher: F,
) -> Result<(PathBuf, CacheSource), String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = FetchOutcome>,
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

    for url in [&tag_url, &main_url] {
        match fetcher(url.clone()).await {
            FetchOutcome::Body(body) => {
                if body.is_empty() {
                    // Empty 2xx body is treated as transient — never
                    // cache a zero-byte script — and never silently
                    // fall back, since `main` may have the same
                    // problem and we don't want to mask it.
                    return Err(format!("GET {url}: empty body"));
                }
                std::fs::write(&cache_path, &body)
                    .map_err(|e| format!("write cache {cache_path:?}: {e}"))?;
                set_executable(&cache_path)?;
                return Ok((cache_path, CacheSource::Downloaded { url: url.clone() }));
            }
            FetchOutcome::NotFound => {
                // Try the next URL in the list. This is the ONLY
                // path that authorises a fallback.
                continue;
            }
            FetchOutcome::TransientError(e) => {
                return Err(format!(
                    "live-fetch rescue script aborted at {url} (transient): {e}. \
                     Refusing to fall back to main — tag may exist."
                ));
            }
        }
    }

    Err(format!(
        "live-fetch rescue script: neither {tag_url} nor {main_url} resolved (both returned 404)"
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
                FetchOutcome::Body(b"should not be called".to_vec())
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

        let fetcher = move |_url: String| async move { FetchOutcome::Body(body.to_vec()) };

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
                    FetchOutcome::NotFound
                } else {
                    FetchOutcome::Body(b"#!/usr/bin/env bash\necho main\n".to_vec())
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
    async fn transient_error_on_tag_does_not_fall_back_to_main() {
        // Codex P2 (PR #151 r3341002540): a 5xx / 403 / network glitch
        // on the tagged URL must NOT silently cache `main` under the
        // current version — that would execute a newer-than-binary
        // script. Fallback is gated on a definitive 404.
        let tmp = fake_home();
        let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let c = calls.clone();

        let fetcher = move |url: String| {
            let c = c.clone();
            async move {
                c.lock().unwrap().push(url.clone());
                if url.contains("/v1.0.0/") {
                    FetchOutcome::TransientError("HTTP 503 Service Unavailable".to_string())
                } else {
                    // main URL would succeed if we (wrongly) fell through.
                    // The assertion below verifies we never get here.
                    FetchOutcome::Body(b"#!/usr/bin/env bash\necho main\n".to_vec())
                }
            }
        };

        let err = ensure_cached_rescue_script(Some(tmp.path()), "1.0.0", fetcher)
            .await
            .expect_err("transient error must abort, not fall back");
        assert!(err.contains("Refusing to fall back"), "got {err}");
        assert!(err.contains("HTTP 503"), "must surface underlying cause: {err}");
        let seen = calls.lock().unwrap();
        assert_eq!(seen.len(), 1, "must stop after the tag URL, not try main");
        assert!(seen[0].contains("/v1.0.0/"));

        let cache_path = cached_rescue_script_path(Some(tmp.path()), "1.0.0");
        assert!(
            !cache_path.exists(),
            "must NOT write a cache file on transient error"
        );
    }

    #[tokio::test]
    async fn both_urls_404_returns_combined_error() {
        let tmp = fake_home();
        let fetcher = |_url: String| async move { FetchOutcome::NotFound };

        let err = ensure_cached_rescue_script(Some(tmp.path()), "1.0.0", fetcher)
            .await
            .expect_err("must fail when both URLs 404");
        assert!(err.contains("neither"), "got {err}");
        assert!(err.contains("v1.0.0"), "must mention tag URL");
        assert!(err.contains("/main/"), "must mention main URL");
    }

    #[tokio::test]
    async fn empty_body_is_treated_as_transient_and_aborts() {
        let tmp = fake_home();
        let fetcher = |_url: String| async move { FetchOutcome::Body(Vec::new()) };

        let err = ensure_cached_rescue_script(Some(tmp.path()), "1.0.0", fetcher)
            .await
            .expect_err("empty body must fail");
        assert!(err.contains("empty body"), "got {err}");
        // Belt + suspenders: an empty 2xx on the tag URL must not silently
        // fall through to main either — same risk as a 5xx.
        let cache_path = cached_rescue_script_path(Some(tmp.path()), "1.0.0");
        assert!(
            !cache_path.exists(),
            "must NOT write a cache file on empty body"
        );
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
