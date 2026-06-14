---
id: hq-sync-release-exit-code-1-not-a-failure
title: An hq-sync release.yml "exit code 1" is not a failure on its own — verify the real outcome
scope: repo
trigger: watching or judging the outcome of an hq-sync `.github/workflows/release.yml` run
when: release || workflow || gh && run
on: [PreToolUse, PostToolUse, UserPromptSubmit, AssistantIntent]
enforcement: soft
public: false
version: 1
created: 2026-06-13
updated: 2026-06-13
source: session-learning
applies_to: [github]
---

## Rule

NEVER treat `Process completed with exit code 1` in an hq-sync `release.yml` run as a failure on its own — it is commonly emitted by a `continue-on-error` step (Sentry sourcemap upload, Node 20 deprecation notice) and does not fail the job.

Verify the real outcome before reporting:

1. `gh run view <run-id> --json conclusion` — the job's `conclusion` is the truth signal, not an inline "exit code 1" line.
2. `gh run watch <run-id> --exit-status` returning `0` confirms success.
3. `gh release view <tag>` and confirm the asset list is complete: `HQ-Sync.app.tar.gz`, its `.sig`, the versioned DMG, the universal DMG alias (`HQ-Sync_universal.dmg`), and `latest.json`.

A run is only a real failure if `conclusion` is `failure`/`cancelled` AND/OR the release is missing its required assets.

## Rationale

Captured from an hq-sync release session. `continue-on-error: true` steps print a non-zero exit code into the log but do not fail the workflow. Reading the inline log line as a failure leads to false alarms and unnecessary retries of an already-successful, signed/notarized release. The job `conclusion` plus the published asset set are the authoritative signals. Complements `hq-sync-release-via-tag-workflow` (how releases are cut) and `hq-sync-version-triple-lockstep` (version parity).
