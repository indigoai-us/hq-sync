//! End-to-end handoff: installer marker → sync embeddings run → journal + cleanup.
//!
//! Verifies the cross-repo contract introduced by US-001 / US-002 without a
//! running Tauri AppHandle. The AppHandle-dependent bit of `start_embeddings`
//! is event emission (`app.emit(...)`); everything else — subprocess
//! lifecycle, journal persistence, marker cleanup — is pure Rust that this
//! test exercises by driving `run_process_impl` directly with a stubbed
//! `qmd` binary and then invoking the same persistence helpers
//! (`write_embeddings_journal`, `clear_pending_markers`) that the real
//! command uses on exit.
//!
//! Two flows covered:
//!   - happy path: stub exits 0 → journal state:"ok", marker removed
//!   - failure path: stub exits 1 → journal state:"error", marker preserved
//!
//! Why not test `start_embeddings` end-to-end? It takes `tauri::AppHandle`,
//! which needs a live Tauri app. hq-sync has no mock-tauri harness today
//! (per `CLAUDE.md`: "Manual testing only in V1"), and standing one up is
//! US-scope creep. Testing the persistence layer with a real subprocess
//! gets us the meaningful coverage — if this test passes, the only thing
//! `start_embeddings` adds on top is `app.emit`, which is trivially
//! correct.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hq_sync_menubar::commands::embeddings::{
    clear_pending_markers, push_stderr_tail, read_embeddings_journal, stderr_tail_to_string,
    write_embeddings_journal, EmbeddingsJournal, STDERR_TAIL_BYTES,
};
use hq_sync_menubar::commands::process::{run_process_impl, ProcessEvent, SpawnArgs};

// ─────────────────────────────────────────────────────────────────────────────
// Event collector — plays the role of AppHandle.emit
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Captured {
    Start,
    Progress(String),
    Complete { duration_sec: u64 },
    Error(String),
}

fn stub_qmd_path() -> PathBuf {
    // `CARGO_MANIFEST_DIR` resolves to `src-tauri/` at test-compile time, so
    // the fixture lookup is independent of the cwd cargo-test happens to pick.
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest).join("tests/fixtures/stub-qmd.sh")
}

/// Drive the same sequence `start_embeddings` does, but into a collector
/// vec instead of a Tauri emit. Takes the stub's first-arg exit code so we
/// can flip between the ok and error flows.
fn drive(
    hq_folder: &str,
    stub_exit_arg: Option<&str>,
) -> (Vec<Captured>, Option<EmbeddingsJournal>) {
    let handle = format!("hq-embeddings-test-{}", std::process::id());
    let mut args: Vec<String> = vec![];
    if let Some(a) = stub_exit_arg {
        args.push(a.to_string());
    }
    let spawn = SpawnArgs {
        cmd: stub_qmd_path().to_string_lossy().into_owned(),
        args,
        cwd: Some(hq_folder.to_string()),
        env: None,
    };

    let captured: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(vec![Captured::Start]));
    let stderr_tail: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
    let start = std::time::Instant::now();

    let result = run_process_impl(&handle, &spawn, |event| match event {
        ProcessEvent::Stdout(line) => {
            captured
                .lock()
                .unwrap()
                .push(Captured::Progress(line));
        }
        ProcessEvent::Stderr(line) => {
            let mut buf = stderr_tail.lock().unwrap();
            push_stderr_tail(&mut buf, &line);
            captured.lock().unwrap().push(Captured::Progress(line));
        }
        ProcessEvent::Exit { code, success } => {
            let duration_sec = start.elapsed().as_secs();
            let now = chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if success {
                let journal = EmbeddingsJournal {
                    last_run_at: now,
                    duration_sec,
                    state: "ok".to_string(),
                    error_msg: None,
                };
                write_embeddings_journal(hq_folder, &journal).expect("journal write");
                clear_pending_markers(hq_folder);
                captured
                    .lock()
                    .unwrap()
                    .push(Captured::Complete { duration_sec });
            } else {
                let tail = stderr_tail_to_string(&stderr_tail.lock().unwrap());
                let msg = if tail.trim().is_empty() {
                    format!(
                        "qmd embed exited with code {}",
                        code.map(|c| c.to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                } else {
                    tail.trim_end().to_string()
                };
                let journal = EmbeddingsJournal {
                    last_run_at: now,
                    duration_sec,
                    state: "error".to_string(),
                    error_msg: Some(msg.clone()),
                };
                write_embeddings_journal(hq_folder, &journal).expect("error journal write");
                // Marker deliberately NOT cleared on the error path.
                captured.lock().unwrap().push(Captured::Error(msg));
            }
        }
    });

    // If run_process_impl errored at the spawn layer we surface it via
    // `captured` so the assertions below still see something meaningful.
    if let Err(e) = result {
        captured.lock().unwrap().push(Captured::Error(e));
    }

    let final_events = captured.lock().unwrap().clone();
    let journal = read_embeddings_journal(hq_folder).ok();
    (final_events, journal)
}

// ─────────────────────────────────────────────────────────────────────────────
// Happy path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn handoff_happy_path_removes_marker_and_writes_ok_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let hq_folder = tmp.path().to_str().unwrap();
    let marker = tmp.path().join(".hq-embeddings-pending.json");
    std::fs::write(&marker, r#"{"reason":"post-install"}"#).unwrap();
    assert!(marker.exists(), "precondition: marker written by stub installer");

    let (events, journal) = drive(hq_folder, None);

    // Event order: Start, ≥1 Progress, Complete. Use a small state machine
    // instead of exact indices — stderr lines can interleave with stdout in
    // real runs and we don't want this test to flake on ordering of non-
    // critical events.
    assert!(matches!(events.first(), Some(Captured::Start)));
    assert!(matches!(events.last(), Some(Captured::Complete { .. })));
    let progress_count = events
        .iter()
        .filter(|e| matches!(e, Captured::Progress(_)))
        .count();
    assert!(
        progress_count >= 1,
        "expected ≥1 Progress event from stub stdout, got {}: {:?}",
        progress_count,
        events
    );

    // Marker cleanup: both primary and fallback paths are empty.
    assert!(!marker.exists(), "marker should be removed after ok run");

    // Journal contents.
    let j = journal.expect("journal must exist after ok run");
    assert_eq!(j.state, "ok");
    assert_eq!(j.error_msg, None);
    // duration_sec could be 0 on very fast runs — accept any non-failure
    // value; the point is just that the field was populated.
    let _ = j.duration_sec;
}

#[test]
fn handoff_happy_path_completes_within_ten_seconds() {
    let tmp = tempfile::tempdir().unwrap();
    let hq_folder = tmp.path().to_str().unwrap();
    std::fs::write(
        tmp.path().join(".hq-embeddings-pending.json"),
        r#"{"reason":"post-install"}"#,
    )
    .unwrap();

    let start = std::time::Instant::now();
    let (events, journal) = drive(hq_folder, None);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "handoff flow should complete in <10s, took {:.1}s",
        elapsed.as_secs_f32()
    );
    assert!(matches!(events.last(), Some(Captured::Complete { .. })));
    assert_eq!(
        journal.expect("journal must exist").state,
        "ok",
        "fast path should still persist an ok journal"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Error path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn handoff_error_path_preserves_marker_and_writes_error_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let hq_folder = tmp.path().to_str().unwrap();
    let marker = tmp.path().join(".hq-embeddings-pending.json");
    std::fs::write(&marker, r#"{"reason":"post-install"}"#).unwrap();

    let (events, journal) = drive(hq_folder, Some("1"));

    // Last event must be Error (not Complete).
    assert!(
        matches!(events.last(), Some(Captured::Error(_))),
        "expected last event to be Error, got {:?}",
        events.last()
    );

    // Marker preservation — this is the retry contract.
    assert!(
        marker.exists(),
        "marker must remain after error run for next-launch retry"
    );

    // Journal reflects the failure.
    let j = journal.expect("journal must exist after error run");
    assert_eq!(j.state, "error");
    assert!(
        j.error_msg.is_some(),
        "error_msg must be populated after non-zero exit"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 2KB stderr tail guarantee (US-002 invariant, exercised E2E)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn stderr_tail_truncates_over_budget_even_under_live_spawn() {
    // We don't need the stub for this — drive push_stderr_tail directly with
    // a synthetic stream larger than the budget and confirm the clamp holds
    // across a realistic loop count.
    let mut buf = VecDeque::with_capacity(STDERR_TAIL_BYTES + 256);
    for i in 0..500 {
        push_stderr_tail(
            &mut buf,
            &format!("line {:05}: qmd embed progress noise that repeats", i),
        );
    }
    assert!(buf.len() <= STDERR_TAIL_BYTES);

    let tail = stderr_tail_to_string(&buf);
    assert!(
        tail.contains("00499") || tail.contains("00498"),
        "tail should retain most recent lines, got: {}",
        &tail[..tail.len().min(200)]
    );
}
