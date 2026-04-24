//! Integration tests for the `promote_company` + `list_all_companies` Tauri
//! commands (US-004b).
//!
//! Rather than spawning the real `hq-sync-runner` (which would require npx,
//! network, and a provisioned Cognito session), these tests drop a POSIX
//! shell stub into a tempdir and drive the command's `*_impl` function
//! directly against that stub. The stub emits canned ndjson in various
//! shapes — happy path, error, guard-trip — so the Rust side's event parsing
//! and handle registry are exercised end-to-end without leaving the test
//! process.
//!
//! The test file targets `promote_company_impl` (not the Tauri command
//! itself) because the `#[tauri::command]` wrapper spawns a background
//! thread and adds its own `promote:start` emission — we want full control
//! over event ordering in the assertions, so we call the impl directly.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hq_sync_menubar::commands::companies::{
    build_list_all_companies_spawn_args, build_promote_spawn_args, list_all_companies_impl,
    promote_company_impl, promote_company_impl_registered, promote_handle, CompanyInfo,
};
use hq_sync_menubar::commands::process::{try_register_handle, SpawnArgs};
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::{Listener, Manager as _};

// ─────────────────────────────────────────────────────────────────────────────
// Fixture helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Write `body` to `dir/name`, chmod +x, return the path.
fn make_exec_stub(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write stub");
    let mut perms = std::fs::metadata(&path).expect("stat stub").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod stub");
    path
}

/// Build a `SpawnArgs` that points at `script` with no extra args.
fn spawn_script(script: &Path) -> SpawnArgs {
    SpawnArgs {
        cmd: script.to_string_lossy().to_string(),
        args: vec![],
        cwd: None,
        env: None,
    }
}

/// Spin up a Tauri mock app whose `AppHandle` can be used to drive
/// `promote_company_impl`. Returns both the app (so the runtime stays
/// alive for the duration of the test — dropping it tears down listeners)
/// and a cloned `AppHandle`.
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

/// Wait up to `timeout` for `predicate` to return true. Polls at 10ms.
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
// list_all_companies_impl — against a POSIX stub
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn list_all_companies_parses_stub_canned_output() {
    let tmp = tempfile::tempdir().unwrap();
    // Canonical canned list from the PRD's e2e test: one local, one aws.
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        r#"#!/bin/sh
printf '[{"slug":"acme","name":"Acme","source":"local"},{"slug":"beta","name":"Beta","uid":"U-1","source":"aws"}]\n'
"#,
    );
    let result = list_all_companies_impl(&spawn_script(&stub)).expect("ok");
    assert_eq!(result.len(), 2);

    let acme = &result[0];
    assert_eq!(acme.slug, "acme");
    assert_eq!(acme.name, "Acme");
    assert_eq!(acme.source, "local");
    assert!(acme.uid.is_none(), "local entry must not carry uid");

    let beta = &result[1];
    assert_eq!(beta.slug, "beta");
    assert_eq!(beta.source, "aws");
    assert_eq!(beta.uid, Some("U-1".to_string()));
}

#[test]
fn list_all_companies_propagates_non_zero_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        "#!/bin/sh\nprintf 'discovery failed\\n' >&2\nexit 1\n",
    );
    let err = list_all_companies_impl(&spawn_script(&stub)).unwrap_err();
    assert!(err.contains("exited"), "expected exit error, got: {}", err);
    assert!(
        err.contains("discovery failed"),
        "stderr should surface in err: {}",
        err
    );
}

#[test]
fn list_all_companies_rejects_non_json_output() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        "#!/bin/sh\nprintf 'definitely not JSON\\n'\n",
    );
    let err = list_all_companies_impl(&spawn_script(&stub)).unwrap_err();
    assert!(err.contains("parse"), "expected parse err, got: {}", err);
}

// ─────────────────────────────────────────────────────────────────────────────
// promote_company_impl — happy path (success sequence)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn promote_company_emits_full_success_sequence() {
    let tmp = tempfile::tempdir().unwrap();
    // Unique slug per test keeps the (global) process registry from
    // colliding when cargo runs tests in parallel. Production code uses
    // the real slug; the test uses a scoped variant to stay isolated.
    let slug = "success-acme";
    // Canned success: start → progress(entity) → progress(bucket) →
    // progress(writeback) → complete. Emits one event per line, then exits 0.
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        &format!(
            r#"#!/bin/sh
printf '%s\n' '{{"type":"promote:start","slug":"{slug}"}}'
printf '%s\n' '{{"type":"promote:progress","slug":"{slug}","step":"entity"}}'
printf '%s\n' '{{"type":"promote:progress","slug":"{slug}","step":"bucket"}}'
printf '%s\n' '{{"type":"promote:progress","slug":"{slug}","step":"writeback"}}'
printf '%s\n' '{{"type":"promote:complete","slug":"{slug}","uid":"cmp_01","bucketName":"bucket-{slug}"}}'
exit 0
"#
        ),
    );

    let (app, handle) = mock_app_handle();

    // Collect every `promote:*` event in arrival order.
    let collected: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::new(Mutex::new(Vec::new()));
    for evt in [
        "promote:start",
        "promote:progress",
        "promote:complete",
        "promote:error",
    ] {
        let sink = collected.clone();
        let name = evt.to_string();
        app.listen(evt, move |event| {
            let payload: serde_json::Value =
                serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
            sink.lock().unwrap().push((name.clone(), payload));
        });
    }

    let result = promote_company_impl(handle, slug, spawn_script(&stub));
    assert!(result.is_ok(), "expected Ok, got: {:?}", result);

    // Tauri event delivery is async on the mock runtime. Give it a beat.
    assert!(
        wait_for(Duration::from_secs(2), || {
            collected.lock().unwrap().len() >= 5
        }),
        "timed out waiting for 5 events, got: {:?}",
        collected.lock().unwrap()
    );

    let seen = collected.lock().unwrap();
    let names: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "promote:start",
            "promote:progress",
            "promote:progress",
            "promote:progress",
            "promote:complete",
        ],
        "event ordering mismatch",
    );

    // Check the progress steps arrived in protocol order.
    let steps: Vec<String> = seen
        .iter()
        .filter(|(n, _)| n == "promote:progress")
        .map(|(_, v)| v["step"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(steps, vec!["entity", "bucket", "writeback"]);

    // promote:complete payload carries uid + bucketName.
    let complete = seen
        .iter()
        .find(|(n, _)| n == "promote:complete")
        .map(|(_, v)| v.clone())
        .expect("complete event present");
    assert_eq!(complete["slug"], slug);
    assert_eq!(complete["uid"], "cmp_01");
    assert_eq!(complete["bucketName"], format!("bucket-{}", slug));

    drop(app);
}

// ─────────────────────────────────────────────────────────────────────────────
// promote_company_impl — error path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn promote_company_surfaces_runner_error_event() {
    let tmp = tempfile::tempdir().unwrap();
    let slug = "error-acme"; // unique per-test slug to isolate registry state
                             // Canned error: start → progress(entity) → error → exit 1. The Rust
                             // side must emit promote:error and return Err — but NOT double-emit
                             // a synthetic "exited with code 1" on top of the real error.
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        &format!(
            r#"#!/bin/sh
printf '%s\n' '{{"type":"promote:start","slug":"{slug}"}}'
printf '%s\n' '{{"type":"promote:progress","slug":"{slug}","step":"entity"}}'
printf '%s\n' '{{"type":"promote:error","slug":"{slug}","message":"vault unreachable"}}'
exit 1
"#
        ),
    );

    let (app, handle) = mock_app_handle();

    let errors: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let errors_sink = errors.clone();
    app.listen("promote:error", move |event| {
        let payload: serde_json::Value =
            serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
        errors_sink.lock().unwrap().push(payload);
    });

    let result = promote_company_impl(handle, slug, spawn_script(&stub));
    assert!(
        result.is_err(),
        "expected Err on runner error, got: {:?}",
        result
    );

    assert!(
        wait_for(Duration::from_secs(2), || {
            errors.lock().unwrap().len() >= 1
        }),
        "timed out waiting for promote:error",
    );

    let errors = errors.lock().unwrap();
    assert_eq!(
        errors.len(),
        1,
        "exactly one promote:error should fire — got {}: {:?}",
        errors.len(),
        *errors
    );
    assert_eq!(errors[0]["slug"], slug);
    assert_eq!(errors[0]["message"], "vault unreachable");

    drop(app);
}

// ─────────────────────────────────────────────────────────────────────────────
// promote_company_impl — "already running" guard
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn promote_company_second_call_for_same_slug_errors_already_running() {
    let tmp = tempfile::tempdir().unwrap();
    let slug = "guarded";
    // Slow stub: `promote:start`, then sleep, then `promote:complete`.
    // The sleep window is when the main thread races in a second call.
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        r#"#!/bin/sh
printf '%s\n' '{"type":"promote:start","slug":"guarded"}'
sleep 2
printf '%s\n' '{"type":"promote:complete","slug":"guarded","uid":"cmp_g","bucketName":"b-g"}'
exit 0
"#,
    );

    let (app, handle) = mock_app_handle();

    // Use the emitted `promote:start` event as a ready signal — as soon as
    // the main thread sees it, the first call has definitely registered
    // the handle, so a second call MUST hit the guard.
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let ready_tx = Arc::new(Mutex::new(Some(ready_tx)));
    let ready_tx_clone = ready_tx.clone();
    app.listen("promote:start", move |_| {
        // Only signal once — subsequent starts shouldn't re-trigger.
        if let Some(tx) = ready_tx_clone.lock().unwrap().take() {
            let _ = tx.send(());
        }
    });

    // Kick off the first call on a background thread.
    let handle_bg = handle.clone();
    let stub_bg = stub.clone();
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = promote_company_impl(handle_bg, slug, spawn_script(&stub_bg));
        let _ = done_tx.send(res);
    });

    // Block until the first call has registered its handle (proxied via
    // `promote:start`). Without this signal we'd be racing against npx
    // startup latency.
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first promote call must emit promote:start");

    // Now the second call must hit the "already running" guard. We call
    // once — no polling, because the guard state is deterministic at
    // this point.
    let second = promote_company_impl(handle.clone(), slug, spawn_script(&stub));
    assert!(
        matches!(second, Err(ref e) if e == "already running"),
        "second promote call should have returned Err(\"already running\"), got: {:?}",
        second,
    );

    // Let the first call finish so we don't leak a subprocess.
    let first_result = done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("first promote call must complete");
    assert!(
        first_result.is_ok(),
        "first call should succeed, got: {:?}",
        first_result
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// promote_company_impl — concurrent "already running" guard (TOCTOU regression)
// ─────────────────────────────────────────────────────────────────────────────

/// Two concurrent `promote_company_impl` calls for the SAME slug must result
/// in exactly one Ok + one `Err("already running")` — and the stub must have
/// been invoked exactly once (no duplicate subprocess spawned).
///
/// Regression coverage for Codex P1 on PR #15: the Tauri command wrapper
/// previously registered the handle, then deregistered it, then re-registered
/// inside the background thread. Between the outer deregister and the inner
/// re-register, a second concurrent caller could pass the guard and spawn a
/// second runner. The fix is to hold the registration across the handoff
/// via `promote_company_impl_registered`, which inherits an existing
/// reservation instead of re-registering.
///
/// This test pounds on the `_impl` entry point (which does its own
/// atomic check-and-register) from two threads and asserts the guard
/// is race-free: exactly-once success, exactly-once "already running",
/// exactly-once stub spawn (counted via file append).
#[test]
fn promote_company_concurrent_calls_race_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let slug = "raceslug";
    let counter_path = tmp.path().join("spawn-count");
    let gate_path = tmp.path().join("gate");

    // The stub:
    //  1. Appends "x" to $COUNTER — our exactly-once spawn sentinel.
    //  2. Waits up to 3s for $GATE to exist — holds the handle registered
    //     while both racers attempt try_register_handle.
    //  3. Emits a full success sequence and exits 0.
    //
    // The COUNTER/GATE paths are passed via env (SpawnArgs.env) because
    // positional args would conflict with the runner protocol.
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        &format!(
            r#"#!/bin/sh
printf 'x' >> "$COUNTER"
# Hold the handle open long enough for the second racer to reach try_register_handle.
i=0
while [ ! -e "$GATE" ] && [ "$i" -lt 300 ]; do
  sleep 0.01
  i=$((i + 1))
done
printf '%s\n' '{{"type":"promote:start","slug":"{slug}"}}'
printf '%s\n' '{{"type":"promote:progress","slug":"{slug}","step":"entity"}}'
printf '%s\n' '{{"type":"promote:complete","slug":"{slug}","uid":"cmp_race","bucketName":"b-race"}}'
exit 0
"#
        ),
    );

    let mut env = std::collections::HashMap::new();
    env.insert(
        "COUNTER".to_string(),
        counter_path.to_string_lossy().to_string(),
    );
    env.insert("GATE".to_string(), gate_path.to_string_lossy().to_string());
    let spawn = SpawnArgs {
        cmd: stub.to_string_lossy().to_string(),
        args: vec![],
        cwd: None,
        env: Some(env),
    };

    let (app, handle) = mock_app_handle();

    // Launch two concurrent promote calls for the same slug. They race
    // through try_register_handle.
    let (res_tx, res_rx) = mpsc::channel::<Result<(), String>>();

    let handle_a = handle.clone();
    let spawn_a = SpawnArgs {
        cmd: spawn.cmd.clone(),
        args: spawn.args.clone(),
        cwd: spawn.cwd.clone(),
        env: spawn.env.clone(),
    };
    let res_tx_a = res_tx.clone();
    let t_a = std::thread::spawn(move || {
        let r = promote_company_impl(handle_a, slug, spawn_a);
        let _ = res_tx_a.send(r);
    });

    let handle_b = handle.clone();
    let spawn_b = SpawnArgs {
        cmd: spawn.cmd.clone(),
        args: spawn.args.clone(),
        cwd: spawn.cwd.clone(),
        env: spawn.env.clone(),
    };
    let res_tx_b = res_tx.clone();
    let t_b = std::thread::spawn(move || {
        let r = promote_company_impl(handle_b, slug, spawn_b);
        let _ = res_tx_b.send(r);
    });

    drop(res_tx);

    // Collect both results. One of them will return fast ("already running")
    // even while the other is still waiting at the GATE. We don't care about
    // order — only the multiset.
    //
    // We poll the counter file to see the first subprocess has started, then
    // release the gate so the winning call can complete.
    assert!(
        wait_for(Duration::from_secs(5), || {
            std::fs::read(&counter_path)
                .map(|b| !b.is_empty())
                .unwrap_or(false)
        }),
        "winning stub must have started (counter file populated)"
    );

    // Release the gate so the winning stub can finish.
    std::fs::write(&gate_path, b"go").expect("write gate");

    // Wait for both threads to finish.
    let r1 = res_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("first racer completed");
    let r2 = res_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("second racer completed");
    t_a.join().expect("thread a joined");
    t_b.join().expect("thread b joined");

    // Assert exactly one Ok + one Err("already running").
    let outcomes = [r1, r2];
    let ok_count = outcomes.iter().filter(|r| r.is_ok()).count();
    let already_running_count = outcomes
        .iter()
        .filter(|r| matches!(r, Err(e) if e == "already running"))
        .count();
    assert_eq!(
        ok_count, 1,
        "exactly one concurrent promote must succeed, got: {:?}",
        outcomes
    );
    assert_eq!(
        already_running_count, 1,
        "exactly one concurrent promote must return already-running, got: {:?}",
        outcomes
    );

    // Assert exactly ONE subprocess was spawned (counter file has one byte).
    let count = std::fs::read(&counter_path).expect("read counter");
    assert_eq!(
        count.len(),
        1,
        "exactly one stub spawn expected; counter={:?}",
        String::from_utf8_lossy(&count)
    );

    drop(app);
}

/// Codex P1 regression: the `promote_company` Tauri command reserves the
/// handle via `try_register_handle`, then hands off to the background thread
/// using `promote_company_impl_registered`. The handoff MUST hold the
/// registration continuously — any gap (like the old `deregister` + re-spawn
/// + inner `try_register_handle` pattern) would let a racing
/// `promote_company_impl` call slip through.
///
/// This test simulates the wrapper's flow step-by-step: reserve, then in a
/// background thread call `_registered`. While the background thread is
/// running, a direct `promote_company_impl` call from the foreground must
/// hit the guard. We assert the guard holds from reservation through worker
/// completion — no window.
#[test]
fn promote_company_wrapper_handoff_is_race_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let slug = "handoffslug";
    let gate_path = tmp.path().join("gate");

    // The worker stub blocks on $GATE so we can test the guard while it
    // holds the registration. Emits minimal ndjson on release.
    let stub = make_exec_stub(
        tmp.path(),
        "hq-sync-runner",
        &format!(
            r#"#!/bin/sh
i=0
while [ ! -e "$GATE" ] && [ "$i" -lt 500 ]; do
  sleep 0.01
  i=$((i + 1))
done
printf '%s\n' '{{"type":"promote:start","slug":"{slug}"}}'
printf '%s\n' '{{"type":"promote:complete","slug":"{slug}","uid":"cmp_h","bucketName":"b-h"}}'
exit 0
"#
        ),
    );

    let mut env = std::collections::HashMap::new();
    env.insert("GATE".to_string(), gate_path.to_string_lossy().to_string());
    let spawn = SpawnArgs {
        cmd: stub.to_string_lossy().to_string(),
        args: vec![],
        cwd: None,
        env: Some(env),
    };

    let (app, handle) = mock_app_handle();

    // Simulate the Tauri command wrapper's outer reservation. This MUST
    // succeed on the first try for a fresh slug.
    let reserved = try_register_handle(&promote_handle(slug));
    assert!(reserved, "outer wrapper must successfully reserve handle");

    // Hand the reservation off to the worker via `_registered`. The worker
    // inherits the reservation — does NOT call try_register_handle again.
    let spawn_bg = SpawnArgs {
        cmd: spawn.cmd.clone(),
        args: spawn.args.clone(),
        cwd: spawn.cwd.clone(),
        env: spawn.env.clone(),
    };
    let handle_bg = handle.clone();
    let slug_bg = slug.to_string();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let r = promote_company_impl_registered(handle_bg, &slug_bg, spawn_bg);
        let _ = done_tx.send(r);
    });

    // Give the worker a chance to enter run_process_impl + register PID.
    // We don't need perfect sync — the guard is held continuously by the
    // reservation, so ANY time between the outer reserve and worker exit
    // must reject a racing caller.
    std::thread::sleep(Duration::from_millis(100));

    // A concurrent `promote_company_impl` call for the same slug MUST fail
    // with "already running". This is the property that was broken: in the
    // old code the wrapper deregistered before spawning the worker, so this
    // racer could slip through.
    let racer = promote_company_impl(handle.clone(), slug, spawn_script(&stub));
    assert!(
        matches!(racer, Err(ref e) if e == "already running"),
        "racing caller must hit already-running guard, got: {:?}",
        racer
    );

    // Also verify a direct try_register_handle (the inner guard) rejects —
    // proving the reservation is held, not merely that the impl-level guard
    // happened to run first.
    assert!(
        !try_register_handle(&promote_handle(slug)),
        "registry must still hold the reservation while worker runs"
    );

    // Release the gate so the worker finishes cleanly.
    std::fs::write(&gate_path, b"go").expect("write gate");

    let worker_result = done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("worker completes");
    worker.join().expect("worker joined");
    assert!(
        worker_result.is_ok(),
        "worker should succeed, got: {:?}",
        worker_result
    );

    // After worker exits, the registration is released — a fresh call for
    // the same slug should succeed again. (Sanity that we don't leak.)
    assert!(
        try_register_handle(&promote_handle(slug)),
        "handle must be released once worker exits"
    );
    // Clean up the handle we just re-registered so parallel tests aren't
    // affected. Use the public impl rather than exposing deregister.
    hq_sync_menubar::commands::process::deregister_process(&promote_handle(slug));

    drop(app);
}

// ─────────────────────────────────────────────────────────────────────────────
// handle / spawn-args sanity
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn promote_handle_is_slug_scoped() {
    assert_eq!(promote_handle("acme"), "hq-promote-acme");
    assert_ne!(promote_handle("acme"), promote_handle("beta"));
}

#[test]
fn spawn_args_include_expected_flags() {
    let list = build_list_all_companies_spawn_args("/tmp/HQ");
    assert!(list.args.iter().any(|a| a == "--list-all-companies"));
    assert!(list.args.iter().any(|a| a == "--hq-root"));

    let promote = build_promote_spawn_args("acme", "/tmp/HQ");
    assert!(promote.args.iter().any(|a| a == "--promote"));
    assert!(promote.args.iter().any(|a| a == "acme"));
}

#[test]
fn company_info_deserializes_mixed_sources() {
    let json = r#"[
        {"slug":"a","name":"A","source":"local"},
        {"slug":"b","name":"B","uid":"U","source":"aws"},
        {"slug":"c","name":"C","uid":"V","source":"both"}
    ]"#;
    let rows: Vec<CompanyInfo> = serde_json::from_str(json).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows[0].uid.is_none() && rows[0].source == "local");
    assert!(rows[1].uid.is_some() && rows[1].source == "aws");
    assert!(rows[2].uid.is_some() && rows[2].source == "both");
}
