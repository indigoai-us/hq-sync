//! Apply the staging-channel update by shelling out to
//! `personal/skills/replace-from-staging/replace-from-staging.sh` and
//! streaming its stdout/stderr to the frontend as Tauri events.
//!
//! Why a subprocess instead of a Rust reimplementation:
//!   * The skill script is the single source of truth for HQ's wipe +
//!     overlay semantics (used by `/personal:replace-from-staging`, and
//!     now by this command). Reimplementing the wipe in Rust would mean
//!     two places to keep in lock-step every time the preserve-list or
//!     rsync filter rules evolve.
//!   * Path set comes from `<HQ>/core/core.yaml#replace_from_staging.paths`
//!     (declarative manifest shipped with each release) and is forwarded
//!     to the script via `--paths` flags. `preserve_subpaths` ditto via
//!     `--preserve-subpath`. The menubar reads the manifest at invocation
//!     time so the UI's "what's about to be overwritten" list matches
//!     exactly what the script will do — no drift possible.
//!
//! Pre-flight contract enforced by this command (NOT the script):
//!   * The `staging_update_channel` MenubarPrefs flag must be on. We
//!     refuse if the user toggled it off between the UI render and
//!     button click.
//!   * The HQ root must look like an HQ root (the script also checks
//!     this — belt and suspenders so a bad invocation fails before the
//!     `git clone` cost is paid).
//!
//! Confirmation UX lives in the frontend's modal, not here. We always
//! pass `--yes` to the script because by the time we're spawning, the
//! user has already confirmed in the UI; an interactive prompt at the
//! Tauri-subprocess layer would just hang the process.

use std::path::PathBuf;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::logfile::log;
use crate::util::paths;

/// `core/core.yaml#replace_from_staging` shape. The menubar reads this so
/// the UI confirmation matches exactly what the script will do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceFromStagingManifest {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub preserve_subpaths: Vec<String>,
}

impl ReplaceFromStagingManifest {
    /// Empty manifest — used as the "absent" sentinel when an older
    /// hq-core release doesn't ship the key yet. The UI should treat
    /// an empty `paths` list as "feature not supported on this release"
    /// and disable the Update button rather than firing a no-op script.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Payload returned to the frontend by `apply_hq_core_staging` on success.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyHqCoreStagingResult {
    /// Script exit code (always 0 here — we only return Ok on a successful
    /// exit; non-zero exits propagate as Err).
    pub exit_code: i32,
    /// The `tag_name` (e.g. `v14.2.1-beta.3`) of the staging release that
    /// was applied — forwarded back to the frontend so it can render
    /// "Applied v14.2.1-beta.3" without needing to re-poll.
    pub tag: String,
    /// Echo of the manifest the menubar acted on, so the frontend can show
    /// the user exactly what landed (matches the confirmation modal).
    pub manifest: ReplaceFromStagingManifest,
}

/// Resolve the user's HQ folder using the same 4-tier resolver every other
/// path-aware command uses. Returns Err with a user-facing message when
/// resolution fails entirely.
fn resolve_hq_root() -> Result<PathBuf, String> {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    let hq_folder = paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );
    if !hq_folder.join(".git").exists() || !hq_folder.join("companies").exists() {
        return Err(format!(
            "HQ folder at {} doesn't look like an HQ root (missing .git/ or companies/). \
             Configure the HQ folder in Settings first.",
            hq_folder.display()
        ));
    }
    Ok(hq_folder)
}

/// Read `<HQ>/core/core.yaml#replace_from_staging` and return the manifest.
/// Returns an empty manifest when the key is missing — the frontend treats
/// that as "feature not supported by this hq-core release" and disables the
/// button.
fn read_manifest(hq_root: &std::path::Path) -> Result<ReplaceFromStagingManifest, String> {
    // Canonical (v14+) first, legacy root fallback.
    let canonical = hq_root.join("core").join("core.yaml");
    let legacy = hq_root.join("core.yaml");
    let core_yaml = if canonical.is_file() { canonical } else { legacy };
    if !core_yaml.is_file() {
        return Err(format!(
            "core.yaml not found at {} or {} — is this really an HQ root?",
            hq_root.join("core/core.yaml").display(),
            hq_root.join("core.yaml").display()
        ));
    }
    let bytes = std::fs::read(&core_yaml)
        .map_err(|e| format!("read {}: {e}", core_yaml.display()))?;
    let parsed: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|e| format!("parse {} as YAML: {e}", core_yaml.display()))?;
    let block = match parsed.get("replace_from_staging") {
        Some(b) => b,
        None => return Ok(ReplaceFromStagingManifest::default()),
    };
    let manifest: ReplaceFromStagingManifest = serde_yaml::from_value(block.clone())
        .map_err(|e| format!("parse replace_from_staging block: {e}"))?;
    Ok(manifest)
}

impl Default for ReplaceFromStagingManifest {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            preserve_subpaths: Vec::new(),
        }
    }
}

/// Locate the bundled replace-from-staging.sh inside the user's HQ folder.
///
/// We rely on the path the skill script itself encodes
/// (`personal/skills/replace-from-staging/replace-from-staging.sh`) instead
/// of falling back to an `npx`-style invocation, because the script's HQ-root
/// resolution literally goes `script_dir/../../.. -> HQ root`. If we ran a
/// copy bundled inside the Tauri app, the `..`-walk would land somewhere in
/// the app's resources dir, not the user's HQ.
fn locate_script(hq_root: &std::path::Path) -> Result<PathBuf, String> {
    let candidate = hq_root
        .join("personal")
        .join("skills")
        .join("replace-from-staging")
        .join("replace-from-staging.sh");
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(format!(
            "replace-from-staging.sh not found at {}. \
             This release needs hq-core ≥ the version that ships the \
             replace-from-staging skill in core/, OR the skill installed at \
             personal/skills/replace-from-staging/.",
            candidate.display()
        ))
    }
}

/// Pre-flight check: refuse if the staging channel toggle isn't on. The UI
/// already gates the button render on this, but a stale frontend or direct
/// devtools invocation could try to call us with the flag off.
fn is_staging_channel_enabled() -> bool {
    let path = match paths::menubar_json_path() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if !path.exists() {
        return false;
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    serde_json::from_str::<MenubarPrefs>(&contents)
        .ok()
        .and_then(|p| p.staging_update_channel)
        .unwrap_or(false)
}

/// Tauri command — read the manifest from the local HQ root without firing
/// the script. The Settings panel calls this to render the confirmation
/// modal's "what's about to be overwritten" list.
#[tauri::command]
pub fn read_replace_from_staging_manifest() -> Result<ReplaceFromStagingManifest, String> {
    let hq_root = resolve_hq_root()?;
    read_manifest(&hq_root)
}

/// Tauri command — invoke the replace-from-staging script in narrow-scope
/// mode using the manifest from `core/core.yaml`. Streams stdout/stderr
/// lines back as `hq-core-staging-apply:progress` events so the Settings UI
/// can render a live progress strip.
///
/// `tag` is the staging release tag the user confirmed against (e.g.
/// `v14.2.1-beta.3`); it gets forwarded to the script via `--ref` so the
/// shallow clone lands on the exact published artifact, not a moving main.
#[tauri::command]
pub async fn apply_hq_core_staging(
    app: AppHandle,
    tag: String,
) -> Result<ApplyHqCoreStagingResult, String> {
    // 1. Flag check.
    if !is_staging_channel_enabled() {
        return Err("Staging update channel is disabled. Toggle 'Staging channel' in Settings first.".to_string());
    }
    // 2. Validate the tag shape so a hostile caller can't pass `--ref` an
    //    arbitrary shell-metacharacter blob. We accept the conservative
    //    `v?<digit-dot-digit-dot-digit><suffix>` shape covering both stable
    //    (`vX.Y.Z`) and pre-release (`vX.Y.Z-beta.N`) tags.
    if !is_valid_tag(&tag) {
        return Err(format!("Refusing to invoke replace with malformed tag {tag:?}"));
    }
    // 3. HQ root resolution + manifest read happen BEFORE we spawn so any
    //    misconfiguration surfaces as a clean Err rather than a half-run
    //    script.
    let hq_root = resolve_hq_root()?;
    let manifest = read_manifest(&hq_root)?;
    if manifest.is_empty() {
        return Err(
            "core/core.yaml has no replace_from_staging.paths declaration. \
             This release predates the staging-channel update feature; \
             update hq-core first (use the production update flow)."
                .to_string(),
        );
    }
    let script = locate_script(&hq_root)?;

    let paths_csv = manifest.paths.join(",");
    let mut cmd = Command::new("bash");
    cmd.arg(&script)
        .arg("--ref")
        .arg(&tag)
        .arg("--paths")
        .arg(&paths_csv)
        .arg("--yes");
    for sub in &manifest.preserve_subpaths {
        cmd.arg("--preserve-subpath").arg(sub);
    }
    // Inherit nothing dangerous; we want the script's own HQ-root resolver
    // (relative to its on-disk location) to fire, not be steered by env.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    log(
        "hq-core-staging-apply",
        &format!("spawning {} --ref {tag} --paths {paths_csv}", script.display()),
    );
    let _ = app.emit(
        "hq-core-staging-apply:progress",
        &ProgressLine {
            stream: "info",
            line: format!("Starting replace-from-staging for {tag} …"),
        },
    );

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", script.display()))?;

    // Stream stdout AND stderr concurrently, tagging each line so the
    // frontend can color stderr differently if it wants. Using tokio
    // BufReader so we don't block the runtime thread waiting on the
    // (potentially long-running) script.
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout pipe".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "no stderr pipe".to_string())?;

    let app_for_stdout = app.clone();
    let stdout_task = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_for_stdout.emit(
                "hq-core-staging-apply:progress",
                &ProgressLine {
                    stream: "stdout",
                    line,
                },
            );
        }
    });
    let app_for_stderr = app.clone();
    let stderr_task = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_for_stderr.emit(
                "hq-core-staging-apply:progress",
                &ProgressLine {
                    stream: "stderr",
                    line,
                },
            );
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("wait on script: {e}"))?;
    // Drain the stream tasks so we don't drop their tail lines.
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let exit_code = status.code().unwrap_or(-1);
    if !status.success() {
        let msg = format!(
            "replace-from-staging.sh exited with code {exit_code}. \
             See the progress log above; nothing has been committed."
        );
        log("hq-core-staging-apply", &msg);
        let _ = app.emit(
            "hq-core-staging-apply:progress",
            &ProgressLine {
                stream: "error",
                line: msg.clone(),
            },
        );
        return Err(msg);
    }

    log(
        "hq-core-staging-apply",
        &format!("script completed cleanly for {tag}"),
    );
    let _ = app.emit(
        "hq-core-staging-apply:progress",
        &ProgressLine {
            stream: "info",
            line: format!("Done. Applied {tag}."),
        },
    );

    Ok(ApplyHqCoreStagingResult {
        exit_code,
        tag,
        manifest,
    })
}

/// Tag-shape validator. Permits a leading 'v', then a strict semver core
/// (`MAJOR.MINOR.PATCH`), then an optional `-<prerelease>` suffix made of
/// alphanumeric + `.` + `-` runs. Rejects anything that could embed a shell
/// metacharacter or path traversal segment.
fn is_valid_tag(tag: &str) -> bool {
    let s = tag.strip_prefix('v').unwrap_or(tag);
    let (core, suffix) = match s.find('-') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let core_parts: Vec<&str> = core.split('.').collect();
    if core_parts.len() != 3 {
        return false;
    }
    if !core_parts
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    if let Some(sfx) = suffix {
        if sfx.is_empty() {
            return false;
        }
        if !sfx
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Serialize)]
struct ProgressLine {
    /// `stdout` | `stderr` | `info` | `error` — the frontend may want to
    /// style each differently. `info` and `error` are this command's own
    /// bookends (spawn, done, exit-code fail); `stdout`/`stderr` are the
    /// raw script output.
    stream: &'static str,
    line: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_empty_when_paths_empty() {
        let m = ReplaceFromStagingManifest::default();
        assert!(m.is_empty());
    }

    #[test]
    fn manifest_not_empty_when_paths_populated() {
        let m = ReplaceFromStagingManifest {
            paths: vec!["core".to_string()],
            preserve_subpaths: vec![],
        };
        assert!(!m.is_empty());
    }

    #[test]
    fn manifest_deserializes_from_core_yaml_block() {
        let yaml = r#"
paths:
  - .agents
  - .codex
  - .claude
  - core
  - .obsidian
  - AGENTS.md
preserve_subpaths:
  - .claude/settings.local.json
"#;
        let m: ReplaceFromStagingManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.paths.len(), 6);
        assert_eq!(m.paths[0], ".agents");
        assert_eq!(m.paths[5], "AGENTS.md");
        assert_eq!(m.preserve_subpaths, vec![".claude/settings.local.json"]);
    }

    #[test]
    fn manifest_tolerates_missing_optional_fields() {
        // A future release might drop preserve_subpaths if it's empty by
        // convention; we must still parse cleanly.
        let yaml = r#"
paths:
  - core
"#;
        let m: ReplaceFromStagingManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.paths, vec!["core"]);
        assert!(m.preserve_subpaths.is_empty());
    }

    #[test]
    fn tag_validator_accepts_stable_and_prerelease_forms() {
        assert!(is_valid_tag("v14.2.1"));
        assert!(is_valid_tag("14.2.1"));
        assert!(is_valid_tag("v14.2.1-beta.3"));
        assert!(is_valid_tag("v14.2.1-alpha.1"));
        assert!(is_valid_tag("v14.2.1-rc.1"));
    }

    #[test]
    fn tag_validator_rejects_shell_metacharacters() {
        // Anything that could be used to pivot --ref into shell injection
        // or path traversal must be refused.
        assert!(!is_valid_tag("v14.2.1; rm -rf /"));
        assert!(!is_valid_tag("v14.2.1 && curl evil"));
        assert!(!is_valid_tag("v14.2.1/../"));
        assert!(!is_valid_tag("v14.2"));
        assert!(!is_valid_tag("v14"));
        assert!(!is_valid_tag(""));
        assert!(!is_valid_tag("v14.2.1-"));
        assert!(!is_valid_tag("v14.2.1-beta.3$(id)"));
    }
}
