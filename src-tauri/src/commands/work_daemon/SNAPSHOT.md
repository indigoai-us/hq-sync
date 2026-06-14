# Work-delivery daemon — PRESERVATION SNAPSHOT (not shipped)

This branch (`feat/work-mesh-daemon-snapshot`, cut from `origin/main`) exists to
**preserve at-risk code, nothing more.** As of 2026-06-14 the work-delivery
daemon (Agent Work Mesh US-006 / US-010 / US-011) existed only as **uncommitted,
untracked files** sitting in the `feature/hq-sync-conflict-versioning` working
tree — on no branch, in no ref, recoverable by nothing but the filesystem. A
`git clean` or a reset would have destroyed ~86 KB of real, substantial code.
This snapshot captures it so it can't be lost. It has **never been committed,
shipped, deployed, or run in production** — the backend's claim/lease/
directed-delivery/reaper machinery it consumes is live, but this consumer is not.

## What's here

- `mod.rs` (34 KB) — Tauri work-daemon core: IoT-receive + spawn, extracted from
  `dm_mqtt.rs`; `setup_work_daemon()` entry point.
- `headless.rs` (29 KB) — US-010 headless build (`run_headless()`): no Tauri,
  `claude -p` subprocess spawn, REST heartbeat, own tokio runtime. Linux/Outpost.
- `relay.rs` (22 KB) — US-011 session-relay (live-steer).
- `../../examples/agent-daemon.rs` — headless entry-point example.
- `WIRING.patch`, `mod.rs.wired-reference`, `main.rs.wired-reference` — the
  uncommitted working-tree state of the two files that wired the module in.

## To actually build/run it (it is NOT wired on origin/main)

The ONLY daemon-relevant wiring is two lines (the reference files + patch also
contain UNRELATED Recall-SDK permission WIP from the other session — ignore it):

1. `src-tauri/src/commands/mod.rs`: add `pub mod work_daemon;`
2. `src-tauri/src/main.rs` (in the setup block): add
   `#[cfg(target_os = "macos")] commands::work_daemon::setup_work_daemon();`

Dependencies reuse what's already in `Cargo.toml` (`reqwest`, `tokio`,
`aws-sdk-*`, the existing `dm_mqtt` SigV4/WSS path) — no Cargo change needed.
Feature-gated behind `WORK_MESH_ENABLED=true`. Verify with `cargo test` from
`src-tauri/` (hq-sync has no PR CI per `indigo-hq-app-release`).

## Important

The originals were **copied, not moved** — they remain in the
`feature/hq-sync-conflict-versioning` working tree, untouched. This branch is a
safety net, not a decision to ship the daemon. Whether to ship vs shelve it is
the open architectural fork from the 2026-06-13 Work Mesh evaluation
(`companies/indigo/projects/agent-work-mesh/execution-log.md`).
