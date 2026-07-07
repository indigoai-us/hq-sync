//! One-shot hq-sync -> hq-desktop-app bundle handoff.
//!
//! Normal hq-sync builds do not call `setup_migration`; `main.rs` wires it only
//! behind the default-off `migrate-to-hq-desktop` Cargo feature. Keeping this
//! module compiled in default builds lets the pure parsing/path helpers stay
//! covered without making every hq-sync release self-migrate.

#![cfg_attr(not(feature = "migrate-to-hq-desktop"), allow(dead_code))]

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use serde_json::{Map, Value};
use tauri::AppHandle;

use crate::util::logfile::log;
use crate::util::paths;

const MANIFEST_URL: &str =
    "https://github.com/indigoai-us/hq-desktop-app/releases/latest/download/latest.json";
const HQ_DESKTOP_PUBKEY: &str = "untrusted comment: minisign public key: 702D18216BAB970A\nRWQKl6trIRgtcNT5cTMccdAITa5hpwuJCtyTsZO6vAVug6D+fjxmUGtU";
const MIGRATION_MARKER: &str = "migratedToDesktopApp";
const LOCK_FILE_NAME: &str = "migrate-to-hq-desktop.lock";
const INITIAL_DELAY: Duration = Duration::from_secs(3);
/// Timeout for the small `latest.json` manifest fetch.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Fail fast if a connection can't even be established, regardless of the
/// (deliberately generous) total timeout on the big bundle download below.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Total timeout for the ~90 MB app-bundle download. `reqwest`'s `.timeout()`
/// caps the WHOLE request (headers + body), so this must comfortably exceed the
/// download time on a slow link — a 15s cap failed the download at ~6 MB/s.
/// 10 minutes covers roughly a 0.15 MB/s (~1.2 Mbps) connection; retries add headroom.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const REQUEST_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesktopUpdateTarget {
    pub url: String,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
struct LatestManifest {
    platforms: HashMap<String, ManifestPlatform>,
}

#[derive(Debug, Deserialize)]
struct ManifestPlatform {
    url: String,
    signature: String,
}

struct MigrationLockGuard {
    path: PathBuf,
}

impl Drop for MigrationLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Public setup hook for the migration build. It runs once, a few seconds
/// after launch, and logs every failure instead of surfacing UI or aborting
/// the app.
pub fn setup_migration(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        if let Err(e) = run_migration_handoff(handle).await {
            log("desktop-migration", &format!("handoff failed: {e}"));
        }
    });
}

async fn run_migration_handoff(app: AppHandle) -> Result<(), String> {
    let menubar_path = paths::menubar_json_path()?;
    if !should_migrate_from_map(&read_menubar_obj(&menubar_path)) {
        log("desktop-migration", "marker present; skipping handoff");
        return Ok(());
    }

    let lock_path = paths::hq_config_dir()?.join(LOCK_FILE_NAME);
    let Some(lock) = MigrationLockGuard::try_acquire(&lock_path)? else {
        log(
            "desktop-migration",
            "another migration handoff is already active; skipping",
        );
        return Ok(());
    };

    if !should_migrate_from_map(&read_menubar_obj(&menubar_path)) {
        log(
            "desktop-migration",
            "marker present after lock; skipping handoff",
        );
        return Ok(());
    }

    let current_app = match resolve_enclosing_app_path(
        std::env::current_exe().map_err(|e| format!("resolve current executable: {e}"))?,
    ) {
        Some(path) => path,
        None => {
            log(
                "desktop-migration",
                "current executable is not inside a .app bundle; skipping handoff",
            );
            return Ok(());
        }
    };

    let parent = match current_app.parent() {
        Some(parent) => parent.to_path_buf(),
        None => {
            log(
                "desktop-migration",
                "current .app bundle has no parent directory; skipping handoff",
            );
            return Ok(());
        }
    };

    let work_dir = match tempfile::Builder::new()
        .prefix(".hq-desktop-migration-")
        .tempdir_in(&parent)
    {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            log(
                "desktop-migration",
                &format!(
                    "app parent is not writable ({}); skipping handoff",
                    parent.display()
                ),
            );
            return Ok(());
        }
        Err(e) => {
            return Err(format!(
                "create migration tempdir in {}: {e}",
                parent.display()
            ))
        }
    };

    let manifest_body = fetch_text_with_retries(MANIFEST_URL).await?;
    let target = select_darwin_platform_from_latest_json(&manifest_body, &current_target_triple())?;

    let archive_bytes = fetch_bytes_with_retries(&target.url).await?;
    verify_tauri_signature(&archive_bytes, &target.signature, HQ_DESKTOP_PUBKEY)?;

    let archive_path = work_dir.path().join("hq-desktop-app.app.tar.gz");
    fs::write(&archive_path, &archive_bytes)
        .map_err(|e| format!("write downloaded archive: {e}"))?;

    let staged_app = extract_and_stage_app(&archive_path, work_dir.path(), &parent)?;

    // Pre-swap guard: never swap in a bundle that isn't structurally launchable.
    // This closes the "minisign-valid but malformed .app" path — a broken bundle
    // is rejected HERE, before any irreversible filesystem op, so the user keeps
    // their working app and the handoff simply retries on the next launch.
    if let Err(e) = validate_bundle_launchable(&staged_app) {
        let _ = fs::remove_dir_all(&staged_app);
        return Err(e);
    }

    if let Err(e) = atomic_swap_staged_app_into_place(&current_app, &staged_app) {
        let _ = fs::remove_dir_all(&staged_app);
        return Err(e);
    }

    if let Err(marker_err) = crate::commands::first_run::merge_menubar_flags(
        &menubar_path,
        &[(MIGRATION_MARKER, Value::Bool(true))],
    ) {
        let rollback = rollback_swapped_app(&current_app, &staged_app);
        return Err(format!(
            "record migration marker: {marker_err}; rollback={}",
            rollback.map(|_| "ok".to_string()).unwrap_or_else(|e| e)
        ));
    }

    // Keep the swapped-out OLD bundle as a rollback backup for a short window
    // after relaunch, then remove it from a detached process that survives our
    // exit. `app.restart()` below cannot report whether the new bundle actually
    // comes up, so retaining the old bundle briefly means a launch that goes
    // wrong is still recoverable from disk; a healthy launch cleans it shortly.
    schedule_backup_cleanup(&staged_app);
    let _ = std::process::Command::new("touch")
        .arg(&current_app)
        .status();

    log(
        "desktop-migration",
        &format!(
            "swapped {} to hq-desktop-app; restarting",
            current_app.display()
        ),
    );
    drop(lock);
    app.restart();
}

impl MigrationLockGuard {
    fn try_acquire(path: &Path) -> Result<Option<Self>, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create lock parent: {e}"))?;
        }

        match create_lock_file(path) {
            Ok(()) => Ok(Some(Self {
                path: path.to_path_buf(),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_owner_is_alive(path) {
                    return Ok(None);
                }
                let _ = fs::remove_file(path);
                create_lock_file(path)
                    .map_err(|e| format!("create migration lock after stale cleanup: {e}"))?;
                Ok(Some(Self {
                    path: path.to_path_buf(),
                }))
            }
            Err(e) => Err(format!("create migration lock: {e}")),
        }
    }
}

fn create_lock_file(path: &Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    writeln!(file, "{}", std::process::id())?;
    file.sync_all().ok();
    Ok(())
}

fn lock_owner_is_alive(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return true;
    };
    let Ok(pid) = contents.trim().parse::<i32>() else {
        return true;
    };
    if pid <= 0 {
        return true;
    }
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

async fn fetch_text_with_retries(url: &str) -> Result<String, String> {
    let response = fetch_response_with_retries(url, REQUEST_TIMEOUT).await?;
    response
        .text()
        .await
        .map_err(|e| format!("read text response from {url}: {e}"))
}

async fn fetch_bytes_with_retries(url: &str) -> Result<Vec<u8>, String> {
    let response = fetch_response_with_retries(url, DOWNLOAD_TIMEOUT).await?;
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("read bytes response from {url}: {e}"))
}

async fn fetch_response_with_retries(
    url: &str,
    timeout: Duration,
) -> Result<reqwest::Response, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("build reqwest client: {e}"))?;

    let mut last_error = String::new();
    for attempt in 1..=REQUEST_ATTEMPTS {
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                last_error = format!("GET {url} returned HTTP {}", response.status());
            }
            Err(e) => {
                last_error = format!("GET {url}: {e}");
            }
        }

        if attempt < REQUEST_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
        }
    }

    Err(last_error)
}

pub(crate) fn select_darwin_platform_from_latest_json(
    body: &str,
    target_triple: &str,
) -> Result<DesktopUpdateTarget, String> {
    let manifest: LatestManifest =
        serde_json::from_str(body).map_err(|e| format!("parse latest.json: {e}"))?;
    let arch = darwin_arch_from_target_triple(target_triple)
        .ok_or_else(|| format!("unsupported macOS target triple: {target_triple}"))?;

    for key in [format!("darwin-{arch}"), "darwin-universal".to_string()] {
        if let Some(platform) = manifest.platforms.get(&key) {
            return Ok(DesktopUpdateTarget {
                url: platform.url.clone(),
                signature: platform.signature.clone(),
            });
        }
    }

    Err(format!(
        "latest.json has no darwin-{arch} or darwin-universal platform"
    ))
}

fn darwin_arch_from_target_triple(target_triple: &str) -> Option<&'static str> {
    if target_triple.starts_with("aarch64-") || target_triple == "aarch64" {
        Some("aarch64")
    } else if target_triple.starts_with("x86_64-") || target_triple == "x86_64" {
        Some("x86_64")
    } else {
        None
    }
}

fn current_target_triple() -> String {
    format!("{}-apple-darwin", std::env::consts::ARCH)
}

pub(crate) fn resolve_enclosing_app_path(exe: PathBuf) -> Option<PathBuf> {
    exe.ancestors()
        .find(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("app"))
                .unwrap_or(false)
        })
        .map(Path::to_path_buf)
}

pub(crate) fn should_migrate_from_map(obj: &Map<String, Value>) -> bool {
    !obj.contains_key(MIGRATION_MARKER)
}

fn read_menubar_obj(path: &Path) -> Map<String, Value> {
    if !path.exists() {
        return Map::new();
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn verify_tauri_signature(
    data: &[u8],
    release_signature: &str,
    pub_key: &str,
) -> Result<(), String> {
    let public_key =
        PublicKey::decode(pub_key).map_err(|e| format!("decode minisign public key: {e}"))?;
    let signature_file = base64::engine::general_purpose::STANDARD
        .decode(release_signature.trim())
        .map_err(|e| format!("base64-decode release signature: {e}"))?;
    let signature_text = std::str::from_utf8(&signature_file)
        .map_err(|e| format!("release signature is not UTF-8: {e}"))?;
    let signature =
        Signature::decode(signature_text).map_err(|e| format!("decode minisign signature: {e}"))?;
    public_key
        .verify(data, &signature, true)
        .map_err(|e| format!("verify minisign signature: {e}"))
}

fn extract_and_stage_app(
    archive_path: &Path,
    work_dir: &Path,
    app_parent: &Path,
) -> Result<PathBuf, String> {
    let extract_root = work_dir.join("extracted");
    fs::create_dir_all(&extract_root).map_err(|e| format!("create extract dir: {e}"))?;
    extract_app_archive(archive_path, &extract_root)?;

    let extracted_app = find_top_level_app(&extract_root)?;
    let staged_app = unique_staging_path(app_parent);
    fs::rename(&extracted_app, &staged_app).map_err(|e| {
        format!(
            "stage extracted app {} -> {}: {e}",
            extracted_app.display(),
            staged_app.display()
        )
    })?;
    Ok(staged_app)
}

fn extract_app_archive(archive_path: &Path, extract_root: &Path) -> Result<(), String> {
    let archive_file =
        File::open(archive_path).map_err(|e| format!("open verified archive: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(extract_root)
        .map_err(|e| format!("extract verified archive: {e}"))
}

fn find_top_level_app(extract_root: &Path) -> Result<PathBuf, String> {
    let mut apps = fs::read_dir(extract_root)
        .map_err(|e| format!("read extract dir: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("app"))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    apps.sort();
    match apps.len() {
        1 => Ok(apps.remove(0)),
        0 => Err("verified archive did not contain a top-level .app bundle".to_string()),
        _ => Err("verified archive contained multiple top-level .app bundles".to_string()),
    }
}

fn unique_staging_path(parent: &Path) -> PathBuf {
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    parent.join(format!(
        ".hq-desktop-migration-{}-{stamp}.app",
        std::process::id()
    ))
}

fn atomic_swap_staged_app_into_place(current_app: &Path, staged_app: &Path) -> Result<(), String> {
    atomic_swap_paths(staged_app, current_app).map_err(|e| {
        format!(
            "atomic swap {} <-> {}: {e}",
            staged_app.display(),
            current_app.display()
        )
    })
}

fn rollback_swapped_app(current_app: &Path, staged_app: &Path) -> Result<(), String> {
    atomic_swap_paths(staged_app, current_app)
        .map_err(|e| {
            format!(
                "rollback atomic swap {} <-> {}: {e}",
                staged_app.display(),
                current_app.display()
            )
        })
        .and_then(|_| {
            fs::remove_dir_all(staged_app).map_err(|e| {
                format!(
                    "remove rolled-back staged bundle {}: {e}",
                    staged_app.display()
                )
            })
        })
}

/// Structural sanity check that the staged bundle can actually launch before we
/// swap it into place — the pre-swap guard against a (minisign-valid but)
/// malformed archive. Cheap checks only: a readable `Info.plist` and a runnable
/// main executable under `Contents/MacOS`. Combined with the minisign integrity
/// check, the notarized source release, and the VM-verified rollout, this closes
/// the "swapped in a bundle that won't launch" path.
#[cfg(target_os = "macos")]
fn validate_bundle_launchable(app: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let contents = app.join("Contents");
    if !contents.join("Info.plist").is_file() {
        return Err(format!(
            "staged bundle is missing Contents/Info.plist: {}",
            app.display()
        ));
    }

    let macos = contents.join("MacOS");
    let has_executable = fs::read_dir(&macos)
        .map_err(|e| format!("read {}: {e}", macos.display()))?
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .metadata()
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        });
    if !has_executable {
        return Err(format!(
            "staged bundle has no runnable executable under Contents/MacOS: {}",
            app.display()
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn validate_bundle_launchable(_app: &Path) -> Result<(), String> {
    Ok(())
}

/// Remove the retained old-bundle backup shortly after relaunch, from a detached
/// process that outlives our own exit. The path is passed as an argv slot (not
/// interpolated into the shell script) so a directory containing spaces or
/// quotes is handled safely.
#[cfg(unix)]
fn schedule_backup_cleanup(backup: &Path) {
    let _ = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 60; /bin/rm -rf \"$1\"")
        .arg("hq-migration-cleanup")
        .arg(backup)
        .spawn();
}

#[cfg(not(unix))]
fn schedule_backup_cleanup(backup: &Path) {
    let _ = fs::remove_dir_all(backup);
}

#[cfg(target_os = "macos")]
fn atomic_swap_paths(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    let result = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn atomic_swap_paths(_a: &Path, _b: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "bundle swap is only supported on macOS",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn selects_arch_specific_darwin_platform() {
        let body = json!({
            "version": "1.2.3",
            "platforms": {
                "darwin-aarch64": {
                    "url": "https://example.com/arm.app.tar.gz",
                    "signature": "arm-sig"
                },
                "darwin-universal": {
                    "url": "https://example.com/universal.app.tar.gz",
                    "signature": "universal-sig"
                }
            }
        })
        .to_string();

        let selected =
            select_darwin_platform_from_latest_json(&body, "aarch64-apple-darwin").unwrap();
        assert_eq!(
            selected,
            DesktopUpdateTarget {
                url: "https://example.com/arm.app.tar.gz".to_string(),
                signature: "arm-sig".to_string(),
            }
        );
    }

    #[test]
    fn selects_universal_fallback_when_arch_missing() {
        let body = json!({
            "version": "1.2.3",
            "platforms": {
                "darwin-universal": {
                    "url": "https://example.com/universal.app.tar.gz",
                    "signature": "universal-sig"
                }
            }
        })
        .to_string();

        let selected =
            select_darwin_platform_from_latest_json(&body, "x86_64-apple-darwin").unwrap();
        assert_eq!(selected.url, "https://example.com/universal.app.tar.gz");
        assert_eq!(selected.signature, "universal-sig");
    }

    #[test]
    fn errors_when_no_darwin_platform_matches() {
        let body = json!({
            "version": "1.2.3",
            "platforms": {
                "windows-x86_64": {
                    "url": "https://example.com/windows.zip",
                    "signature": "sig"
                }
            }
        })
        .to_string();

        let err =
            select_darwin_platform_from_latest_json(&body, "aarch64-apple-darwin").unwrap_err();
        assert!(err.contains("darwin-aarch64"));
    }

    #[test]
    fn resolves_enclosing_app_bundle_from_executable() {
        let exe = PathBuf::from("/Applications/HQ Sync.app/Contents/MacOS/hq-sync");
        assert_eq!(
            resolve_enclosing_app_path(exe),
            Some(PathBuf::from("/Applications/HQ Sync.app"))
        );
    }

    #[test]
    fn resolve_enclosing_app_returns_none_for_dev_binary() {
        let exe = PathBuf::from("/Users/dev/hq-sync/src-tauri/target/debug/hq-sync");
        assert_eq!(resolve_enclosing_app_path(exe), None);
    }

    #[test]
    fn should_migrate_when_marker_absent() {
        assert!(should_migrate_from_map(&map(json!({
            "machineId": "abc"
        }))));
    }

    #[test]
    fn should_not_migrate_when_marker_present() {
        assert!(!should_migrate_from_map(&map(json!({
            "migratedToDesktopApp": false
        }))));
        assert!(!should_migrate_from_map(&map(json!({
            "migratedToDesktopApp": true
        }))));
    }

    #[test]
    fn verify_tauri_signature_rejects_known_bad_payload() {
        let public_key = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        let signature = "untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        let encoded = STANDARD.encode(signature.as_bytes());

        let err = verify_tauri_signature(b"not test", &encoded, public_key).unwrap_err();
        assert!(err.contains("verify minisign signature"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn validate_bundle_launchable_accepts_well_formed_and_rejects_broken() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("HQ.app");
        let macos = app.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(app.join("Contents/Info.plist"), b"<plist/>").unwrap();

        let exe = macos.join("HQ");
        fs::write(&exe, b"#!/bin/sh\n").unwrap();

        // Present but non-executable main binary -> rejected.
        assert!(validate_bundle_launchable(&app).is_err());

        // Executable bit set -> accepted.
        let mut perms = fs::metadata(&exe).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&exe, perms).unwrap();
        assert!(validate_bundle_launchable(&app).is_ok());

        // Missing Info.plist -> rejected.
        fs::remove_file(app.join("Contents/Info.plist")).unwrap();
        assert!(validate_bundle_launchable(&app).is_err());
    }
}
