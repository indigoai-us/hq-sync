//! First-run / first-update onboarding classification + persisted flags.
//!
//! Three launch kinds, classified once at app `.setup()` time and cached in
//! Tauri managed state so the verdict is stable for the whole process:
//!
//!   - **FirstRun**       brand-new install — never run this app before.
//!   - **ExistingUpdate** a legacy user who updated to a build that has the
//!                        new onboarding flags (and so hasn't seen the
//!                        auto-sync notice yet).
//!   - **Normal**         everything after the first-run sequence completes.
//!
//! ## Why classification must run BEFORE `ensure_machine_id`
//!
//! Both FirstRun and ExistingUpdate lack the new `firstRunCompleted` flag, so
//! that flag alone can't tell them apart. The tiebreaker is `machineId`:
//! `config::ensure_machine_id` writes it to `menubar.json` on the *first ever*
//! launch, so an existing user already has it while a brand-new install does
//! not (the installer writes `hqPath` but never `machineId`). We therefore
//! snapshot the classification at the very top of `.setup()` — before
//! `ensure_machine_id` runs and populates `machineId` for everyone — and stash
//! the result in managed state.
//!
//! All writes use the same untyped-merge + atomic-rename algorithm as
//! `config::ensure_machine_id`: read `menubar.json` as an untyped `Map`, mutate
//! only the target keys, atomic-rename back. The typed `MenubarPrefs` is
//! deliberately NOT used for writes here — a typed round-trip would silently
//! drop unknown / future top-level keys.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};
use tauri::{AppHandle, Manager, State};

use crate::util::paths;

/// How this launch was classified. Cached in managed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchKind {
    /// Brand-new install — never run before.
    FirstRun,
    /// Legacy user updating to an onboarding-aware build.
    ExistingUpdate,
    /// First-run sequence already completed on a prior launch.
    Normal,
}

/// Managed-state wrapper so the launch verdict survives the rest of the
/// process even after `machineId` gets written this launch.
pub struct LaunchKindState(pub LaunchKind);

/// Pure classifier over an already-parsed `menubar.json` object. Kept
/// filesystem-free so it's directly unit-testable.
fn classify_from_map(obj: &Map<String, Value>) -> LaunchKind {
    let first_run_done = obj
        .get("firstRunCompleted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if first_run_done {
        return LaunchKind::Normal;
    }
    let had_machine_id = obj
        .get("machineId")
        .and_then(Value::as_str)
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if had_machine_id {
        LaunchKind::ExistingUpdate
    } else {
        LaunchKind::FirstRun
    }
}

/// True when `autoSyncNoticeShown` is explicitly `true`.
fn notice_shown_in_map(obj: &Map<String, Value>) -> bool {
    obj.get("autoSyncNoticeShown")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Read `menubar.json` at `path` as an untyped object. Missing / malformed /
/// non-object files degrade to an empty map (same leniency as
/// `ensure_machine_id`).
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

/// Untyped-merge `updates` into the `menubar.json` at `path` and atomic-rename
/// it back. Unknown / future top-level keys pass through unchanged. Mirrors the
/// `config::ensure_machine_id` write algorithm exactly.
fn merge_menubar_flags(path: &Path, updates: &[(&str, Value)]) -> Result<(), String> {
    let mut obj = read_menubar_obj(path);
    for (k, v) in updates {
        obj.insert((*k).to_string(), v.clone());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let body =
        serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Classify this launch and stash the verdict in managed state. MUST be called
/// at the top of `.setup()`, before `config::ensure_machine_id` populates
/// `machineId`.
pub fn classify_launch(app: &AppHandle) {
    let kind = match paths::menubar_json_path() {
        Ok(path) => classify_from_map(&read_menubar_obj(&path)),
        // No resolvable home dir → treat as a fresh, safe default. A
        // brand-new machine is the conservative assumption here.
        Err(_) => LaunchKind::FirstRun,
    };
    app.manage(LaunchKindState(kind));
}

/// True only on a brand-new install's first launch.
#[tauri::command]
pub fn is_first_run(state: State<'_, LaunchKindState>) -> bool {
    state.0 == LaunchKind::FirstRun
}

/// True when a legacy user updated to this build, hasn't seen the auto-sync
/// notice yet, AND still has auto-sync on. A user who explicitly turned
/// auto-sync off (`realtimeSync: false`) made a deliberate choice and gets no
/// "auto-sync is on" notice — notify-only, respect opt-outs.
#[tauri::command]
pub fn should_show_auto_sync_notice(state: State<'_, LaunchKindState>) -> bool {
    if state.0 != LaunchKind::ExistingUpdate {
        return false;
    }
    let notice_shown = paths::menubar_json_path()
        .map(|p| notice_shown_in_map(&read_menubar_obj(&p)))
        .unwrap_or(false);
    if notice_shown {
        return false;
    }
    // Respect an explicit opt-out (default-on when the field is absent).
    crate::commands::daemon::is_realtime_sync_enabled()
}

/// Mark the brand-new-install onboarding as finished. Persists
/// `firstRunCompleted` + `autoSyncNoticeShown` (new users got the carousel, so
/// skip the separate notice) and makes "sync is on" explicit by writing
/// `realtimeSync` + `personalSyncEnabled` true.
#[tauri::command]
pub fn mark_first_run_complete() -> Result<(), String> {
    let path = paths::menubar_json_path()?;
    merge_menubar_flags(
        &path,
        &[
            ("firstRunCompleted", Value::Bool(true)),
            ("autoSyncNoticeShown", Value::Bool(true)),
            ("realtimeSync", Value::Bool(true)),
            ("personalSyncEnabled", Value::Bool(true)),
        ],
    )
}

/// Mark the one-time auto-sync notice as shown for an updating user. Also sets
/// `firstRunCompleted` so the next launch classifies as `Normal`. Deliberately
/// does NOT touch `realtimeSync` — opt-outs are respected.
#[tauri::command]
pub fn mark_auto_sync_notice_shown() -> Result<(), String> {
    let path = paths::menubar_json_path()?;
    merge_menubar_flags(
        &path,
        &[
            ("autoSyncNoticeShown", Value::Bool(true)),
            ("firstRunCompleted", Value::Bool(true)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn classify_fresh_install_is_first_run() {
        // No machineId, no firstRunCompleted.
        assert_eq!(classify_from_map(&Map::new()), LaunchKind::FirstRun);
        // Installer wrote hqPath but app never ran (no machineId).
        let obj = map(json!({ "hqPath": "/Users/x/HQ" }));
        assert_eq!(classify_from_map(&obj), LaunchKind::FirstRun);
    }

    #[test]
    fn classify_existing_user_update() {
        // machineId present (app ran before), but no firstRunCompleted yet.
        let obj = map(json!({ "machineId": "abc-123" }));
        assert_eq!(classify_from_map(&obj), LaunchKind::ExistingUpdate);
    }

    #[test]
    fn classify_empty_machine_id_is_first_run() {
        let obj = map(json!({ "machineId": "" }));
        assert_eq!(classify_from_map(&obj), LaunchKind::FirstRun);
    }

    #[test]
    fn classify_completed_is_normal() {
        // Once firstRunCompleted is set, always Normal regardless of machineId.
        let obj = map(json!({ "machineId": "abc", "firstRunCompleted": true }));
        assert_eq!(classify_from_map(&obj), LaunchKind::Normal);
        let obj2 = map(json!({ "firstRunCompleted": true }));
        assert_eq!(classify_from_map(&obj2), LaunchKind::Normal);
    }

    #[test]
    fn notice_shown_reads_flag() {
        assert!(!notice_shown_in_map(&Map::new()));
        assert!(notice_shown_in_map(&map(json!({ "autoSyncNoticeShown": true }))));
        assert!(!notice_shown_in_map(&map(json!({ "autoSyncNoticeShown": false }))));
    }

    #[test]
    fn merge_preserves_unknown_keys_and_sets_flags() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("menubar.json");
        // Seed with a machineId + an unrelated future key.
        fs::write(
            &path,
            r#"{"machineId":"keep-me","futureKey":{"nested":1}}"#,
        )
        .unwrap();

        merge_menubar_flags(
            &path,
            &[
                ("firstRunCompleted", Value::Bool(true)),
                ("autoSyncNoticeShown", Value::Bool(true)),
            ],
        )
        .unwrap();

        let obj = read_menubar_obj(&path);
        // Flags written.
        assert_eq!(obj.get("firstRunCompleted"), Some(&Value::Bool(true)));
        assert_eq!(obj.get("autoSyncNoticeShown"), Some(&Value::Bool(true)));
        // Unknown keys preserved untouched.
        assert_eq!(obj.get("machineId"), Some(&Value::String("keep-me".into())));
        assert_eq!(obj.get("futureKey"), Some(&json!({ "nested": 1 })));
    }

    #[test]
    fn merge_creates_file_when_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("menubar.json");
        assert!(!path.exists());
        merge_menubar_flags(&path, &[("autoSyncNoticeShown", Value::Bool(true))]).unwrap();
        assert!(notice_shown_in_map(&read_menubar_obj(&path)));
    }

    #[test]
    fn merge_then_classify_roundtrip_is_normal() {
        // After mark_first_run_complete-style write, classification flips to
        // Normal even on a fresh map.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("menubar.json");
        merge_menubar_flags(
            &path,
            &[
                ("firstRunCompleted", Value::Bool(true)),
                ("autoSyncNoticeShown", Value::Bool(true)),
                ("realtimeSync", Value::Bool(true)),
                ("personalSyncEnabled", Value::Bool(true)),
            ],
        )
        .unwrap();
        assert_eq!(classify_from_map(&read_menubar_obj(&path)), LaunchKind::Normal);
    }
}
