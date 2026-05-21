---
id: hq-sync-version-triple-lockstep
title: Bump the hq-sync version triple together
scope: repo
trigger: Cutting a release of hq-sync (any change to `package.json`, `src-tauri/tauri.conf.json`, or `src-tauri/Cargo.toml` version fields)
enforcement: soft
version: 1
created: 2026-05-20
updated: 2026-05-20
public: false
source: discover
learned_from: discover/hq-sync@5aca1cd
---

## Rule

Three versions must always agree before a release tag is cut:

1. `package.json` `version` (frontend ESM bundle)
2. `src-tauri/tauri.conf.json` `version` (Tauri bundle, also embedded in the binary)
3. `src-tauri/Cargo.toml` `[package].version` for crate `hq-sync-menubar` (Rust backend)

The git tag must match the Tauri bundle version: `git tag vX.Y.Z` ⇔ `tauri.conf.json` reports `X.Y.Z`.

## Why

- The Tauri auto-updater pins `latest.json` to the bundle version. A mismatch between `tauri.conf.json` and the tag produces an installer the updater will silently refuse to apply (no error surfaced to the user).
- Sentry source maps are uploaded keyed off the frontend `package.json` version. If it drifts, crash reports from the field point at the wrong source maps and become unreadable.
- The Rust crate version is what `cargo` reports in panic backtraces — drift makes it impossible to map a backtrace to a release.

## How to comply

Before pushing a release tag, verify in one shell:

```sh
node -p "require('./package.json').version"
node -p "require('./src-tauri/tauri.conf.json').version"
grep '^version' src-tauri/Cargo.toml | head -1
```

All three must print the same `X.Y.Z`, and the tag you are about to push must be `vX.Y.Z`.

## Exceptions

None at this scope. If a future migration intentionally desynchronises the crate version from the bundle version (e.g. the Rust crate stops being shipped to users), update this policy first.
