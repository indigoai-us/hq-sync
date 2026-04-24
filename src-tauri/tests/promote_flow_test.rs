//! US-006 cross-repo integration test: the hq-sync side of the
//! installer-dedupe + sync-promote story.
//!
//! This test drives the end-to-end promote flow against a checked-in
//! POSIX stub (`tests/fixtures/stub-sync-runner.sh`) rather than spawning
//! the real `hq-sync-runner` via npx. The stub emits the same ndjson +
//! JSON-list shapes the real runner does, so every byte the Rust side
//! sees here is protocol-identical to production.
//!
//! ## Why this test exists alongside `promote_command_test.rs`
//!
//! The US-004b tests in `promote_command_test.rs` inject *inline* stubs
//! written on the fly into a tempdir — that's the right shape for probing
//! edge cases (error paths, "already running" guard, malformed JSON),
//! because each test can craft a minimal stub that isolates one branch.
//!
//! This file has a different job: prove the happy-path *story* end to
//! end using the same checked-in fixture the PRD describes, so a future
//! reader can `cat tests/fixtures/stub-sync-runner.sh` and see exactly
//! what "a well-behaved runner looks like to hq-sync". It also honors
//! the PRD's "mirror the installer-embeddings-to-sync US-006 pattern"
//! instruction — checked-in fixture, not inline.

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hq_sync_menubar::commands::companies::{
    list_all_companies_impl, promote_company_impl, CompanyInfo,
};
use hq_sync_menubar::commands::process::SpawnArgs;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Listener;

// ─────────────────────────────────────────────────────────────────────────────
// Fixture path resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Absolute path to the checked-in stub. `CARGO_MANIFEST_DIR` is set by
/// Cargo to the crate root (`src-tauri/`), so this resolves to
/// `src-tauri/tests/fixtures/stub-sync-runner.sh` regardless of where
/// `cargo test` is invoked from.
fn stub_runner_path() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set by cargo test");
    PathBuf::from(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join("stub-sync-runner.sh")
}

/// Defense-in-depth: git on macOS preserves mode bits, but a fresh clone
/// onto a filesystem that doesn't (e.g. an exFAT share) would drop +x.
/// Re-apply 0o755 at test start so the stub is always executable.
fn ensure_stub_executable(path: &std::path::Path) {
    let mut perms = std::fs::metadata(path)
        .expect("stub fixture must exist")
        .permissions();
    if perms.mode() & 0o111 == 0 {
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod stub fixture");
    }
}

/// Build a SpawnArgs pointing at the checked-in stub with the given flags.
fn spawn_stub(args: Vec<String>) -> SpawnArgs {
    let path = stub_runner_path();
    ensure_stub_executable(&path);
    SpawnArgs {
        cmd: path.to_string_lossy().to_string(),
        args,
        cwd: None,
        env: None,
    }
}

fn mock_app_handle() -> (
    tauri::App<tauri::test::MockRuntime>,
    tauri::AppHandle<tauri::test::MockRuntime>,
) {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("build mock app");
    let handle = app.handle().clone();
    (app, handle)
}

fn wait_for<F: Fn() -> bool>(timeout: Duration, predicate: F) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

// ─────────────────────────────────────────────────────────────────────────────
// The full-story test
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn list_then_promote_full_flow_against_checked_in_stub() {
    // Unique slug per test run keeps the (process-global) promote handle
    // registry clean when cargo schedules this alongside other tests in
    // `promote_command_test.rs`.
    let slug = "story-acme";

    // ─── Step A: list_all_companies against the stub ────────────────────
    //
    // Proves the stub's `--list-all-companies` payload parses correctly
    // into `CompanyInfo` — same round-trip the frontend's CompanyRow UI
    // (US-005) depends on.
    let listing = list_all_companies_impl(&spawn_stub(vec![
        "--list-all-companies".to_string(),
        "--hq-root".to_string(),
        "/tmp/test-hq".to_string(),
    ]))
    .expect("list_all_companies should succeed against stub");

    assert_eq!(listing.len(), 2, "stub emits exactly two companies");

    // Find the local entry and assert its shape — this is the one the
    // UI would render with a "Promote" button.
    let local: &CompanyInfo = listing
        .iter()
        .find(|c| c.source == "local")
        .expect("stub must include a local-source company");
    assert_eq!(local.slug, "acme", "local entry slug");
    assert_eq!(local.name, "Acme", "local entry name");
    assert!(
        local.uid.is_none(),
        "local entry must not have a uid — that's what promote provisions",
    );

    let aws: &CompanyInfo = listing
        .iter()
        .find(|c| c.source == "aws")
        .expect("stub must include an aws-source company");
    assert_eq!(aws.slug, "beta");
    assert_eq!(aws.uid, Some("U-1".to_string()));

    // ─── Step B: promote_company against the stub ───────────────────────
    //
    // The stub emits start → progress(entity) → progress(bucket) →
    // progress(writeback) → complete. Observe the event sequence exactly
    // — ordering + payloads both matter to the UI (US-005's row swaps
    // from "Promote" to "Promoting…" to "✓ Promoted" as these arrive).
    let (app, handle) = mock_app_handle();

    // Subscribe to every promote:* channel before kicking off the call,
    // so we don't miss early emissions. MockRuntime delivers events on
    // a background task; the wait_for below is the sync point.
    let events: Arc<Mutex<Vec<(String, serde_json::Value)>>> =
        Arc::new(Mutex::new(Vec::new()));
    for channel in [
        "promote:start",
        "promote:progress",
        "promote:complete",
        "promote:error",
    ] {
        let sink = events.clone();
        let name = channel.to_string();
        app.listen(channel, move |evt| {
            let payload: serde_json::Value =
                serde_json::from_str(evt.payload()).unwrap_or(serde_json::Value::Null);
            sink.lock().unwrap().push((name.clone(), payload));
        });
    }

    let promote_result = promote_company_impl(
        handle,
        slug,
        spawn_stub(vec![
            "--promote".to_string(),
            slug.to_string(),
            "--hq-root".to_string(),
            "/tmp/test-hq".to_string(),
        ]),
    );
    assert!(
        promote_result.is_ok(),
        "happy-path promote must return Ok, got: {:?}",
        promote_result,
    );

    // Five events: start + 3 progress + complete. No error.
    assert!(
        wait_for(Duration::from_secs(2), || {
            events.lock().unwrap().len() >= 5
        }),
        "timed out waiting for 5 promote events — got: {:?}",
        events.lock().unwrap(),
    );

    let seen = events.lock().unwrap();
    let channels: Vec<&str> = seen.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(
        channels,
        vec![
            "promote:start",
            "promote:progress",
            "promote:progress",
            "promote:progress",
            "promote:complete",
        ],
        "event channel ordering must match the stub's ndjson stream",
    );
    assert!(
        !channels.contains(&"promote:error"),
        "happy path must emit zero promote:error events",
    );

    // Progress steps in protocol order — matches the three promote
    // phases documented in hq-cloud's promoteLocalCompany.
    let steps: Vec<String> = seen
        .iter()
        .filter(|(c, _)| c == "promote:progress")
        .map(|(_, v)| v["step"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        steps,
        vec!["entity", "bucket", "writeback"],
        "progress steps must arrive in entity → bucket → writeback order",
    );

    // Complete payload carries the canonical uid + bucketName shape.
    // This is the payload the UI consumes to flip the row's state to
    // "Promoted" and (per AC 2c) chains into `invoke('start_sync')`.
    // We don't actually invoke start_sync here — that would spawn the
    // real hq-sync, which is out of scope for this test. Instead we
    // assert the complete payload is shaped the way start_sync's
    // caller expects: slug-scoped, carrying uid + bucketName.
    let complete = seen
        .iter()
        .find(|(c, _)| c == "promote:complete")
        .map(|(_, v)| v.clone())
        .expect("promote:complete event must be present");
    assert_eq!(complete["slug"], slug);
    assert_eq!(complete["uid"], format!("cmp_{slug}"));
    assert_eq!(complete["bucketName"], format!("bucket-{slug}"));

    // ─── Step C (AC 2c): start_sync chaining precondition ────────────────
    //
    // The acceptance criterion says: "assert that `invoke('start_sync')`
    // WOULD be chained (test stops short of real sync)". The frontend's
    // chaining logic keys off the payload shape above — specifically, a
    // promote:complete with a non-empty `slug` + `uid`. Encode that
    // contract as assertions here so a future change to the payload
    // shape would trip this test before breaking the UI integration.
    let complete_slug = complete["slug"].as_str().unwrap_or("");
    let complete_uid = complete["uid"].as_str().unwrap_or("");
    assert!(
        !complete_slug.is_empty() && !complete_uid.is_empty(),
        "promote:complete must carry a non-empty slug + uid for start_sync to chain off",
    );

    drop(app);
}
