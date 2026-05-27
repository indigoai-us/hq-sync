---
id: hq-sync-version-triple-lockstep
title: Bump the hq-sync version triple together
scope: repo
trigger: Cutting a release of hq-sync (any change to `package.json`, `src-tauri/tauri.conf.json`, or `src-tauri/Cargo.toml` version fields)
enforcement: soft
version: 2
created: 2026-05-20
updated: 2026-05-26
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

### Pre-release tags (beta / alpha)

The triple may carry a SemVer pre-release suffix when the tag does:

- Stable tag `vX.Y.Z` ⇔ triple `X.Y.Z`
- Beta tag `vX.Y.Z-beta.N` ⇔ triple `X.Y.Z-beta.N`
- Alpha tag `vX.Y.Z-alpha.N` ⇔ triple `X.Y.Z-alpha.N`

The release workflow (`.github/workflows/release.yml`) classifies the tag by suffix and rejects any other pre-release pattern (`-rc.N`, `-pre`, `-dev`, etc.). See `hq-sync-release-channels-client-gating` for how the auto-updater routes these tags into per-channel update streams.

## Why

- The Tauri auto-updater pins `latest.json` to the bundle version. A mismatch between `tauri.conf.json` and the tag produces an installer the updater will silently refuse to apply (no error surfaced to the user).
- Sentry source maps are uploaded keyed off the frontend `package.json` version. If it drifts, crash reports from the field point at the wrong source maps and become unreadable.
- The Rust crate version is what `cargo` reports in panic backtraces — drift makes it impossible to map a backtrace to a release.
- For pre-release tags, the version triple feeds `semver::Version::parse` in `util/release_channel.rs::parse_channel_from_tag` on the client side. The client uses pre-release SemVer ordering to rank channel candidates (`X.Y.Z-alpha.1 < X.Y.Z-beta.1 < X.Y.Z`). If `tauri.conf.json` reports the stable `X.Y.Z` while the tag is `vX.Y.Z-beta.1`, the Tauri updater advertises a newer version than what was actually published and a stable user will silently never get the next release.

## How to comply

Before pushing a release tag, verify in one shell:

```sh
node -p "require('./package.json').version"
node -p "require('./src-tauri/tauri.conf.json').version"
grep '^version' src-tauri/Cargo.toml | head -1
```

All three must print the same `X.Y.Z[-channel.N]`, and the tag you are about to push must be `vX.Y.Z[-channel.N]`.

## Exceptions

None at this scope. If a future migration intentionally desynchronises the crate version from the bundle version (e.g. the Rust crate stops being shipped to users), update this policy first.
