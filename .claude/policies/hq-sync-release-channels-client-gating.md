---
id: hq-sync-release-channels-client-gating
title: Release-channel gating is client-side and identity-aware
scope: repo
trigger: Modifying the Tauri auto-updater path, the release-channel resolver, or the publish workflow
enforcement: hard
version: 1
created: 2026-05-26
updated: 2026-05-26
public: false
source: user-decision
---

## Rule

HQ Sync ships three release channels — `stable`, `beta`, `alpha` — and the only users ever notified about non-stable releases are `@getindigo.ai` accounts. Concretely:

1. The Settings UI MUST only render the channel picker when the backend's `available_channels` Tauri command returns more than one channel. That command MUST gate on `util::feature_gate::is_indigo_user`.
2. The Rust-side updater MUST re-apply the indigo gate at every check via `util::release_channel::effective_channel(stored_pref, is_indigo)`. A non-indigo user with a hand-edited `release_channel: "beta"` in `~/.hq/menubar.json` MUST receive Stable updates.
3. The `available_channels` command for non-indigo users MUST return `["stable"]` exactly. Adding any other entry — even gated by feature flags — defeats the layered defense in (2).
4. The channel-to-tag mapping is by **GitHub tag suffix**, not by GitHub's per-release `prerelease` flag:
   - `vX.Y.Z` → Stable
   - `vX.Y.Z-beta.N` → Beta
   - `vX.Y.Z-alpha.N` → Alpha
   - Any other pre-release suffix (`-rc.N`, `-pre`, `-dev`, raw `-beta` with no number) is rejected by `parse_channel_from_tag` and ignored by the resolver.
5. The release workflow (`.github/workflows/release.yml`) MUST set `prerelease: true` for beta/alpha tags so the GitHub UI labels them correctly AND the `/releases/latest/download/` redirect skips them (which is what keeps stable users — who poll that static endpoint as a fallback — from accidentally picking up a beta).
6. Indigo users with no stored preference default to **Beta** (auto-opt-in for dogfooding). The default is applied at `effective_channel`'s boundary; `get_settings` does NOT write the default back, so a true "no preference" state survives in `menubar.json` and the default re-resolves correctly if a user later switches Cognito identities.

## Why

- Non-stable builds are by definition higher-risk. Surfacing them to non-Indigo customers violates the implicit contract of an auto-updater (`Check for Updates` must mean "the next stable I should be on", not "any artifact that exists").
- A single point of gating (e.g. UI-only) is insufficient: HQ Sync's `menubar.json` is plain JSON on disk, end-user editable, and synced across machines. The defense-in-depth pattern (UI gate + Rust-side coercion + tag-suffix filter + CI `prerelease` flag) means no single bypass leaks alpha/beta artifacts to a stable user.
- Routing by tag suffix rather than the server's `prerelease` flag means the client correctness does not depend on the CI workflow being patched first — making the client change shippable independently and rollback-safe.
- Using GitHub's existing `/releases/latest/download/` redirect for stable means we don't need to host a separate channel-manifests endpoint or maintain a `latest-stable.json` artifact. Beta/alpha resolution hits the GitHub Releases API for the freshest matching tag and constructs the per-release manifest URL.

## How to comply

When touching any of:

- `src-tauri/src/updater.rs` (resolver, background loop, install)
- `src-tauri/src/util/release_channel.rs` (channel enum, tag parsing, endpoint resolver)
- `src-tauri/src/util/feature_gate.rs` (indigo gate)
- `src-tauri/src/commands/settings.rs` (preference round-trip)
- `src/components/Settings.svelte` (picker UI)
- `.github/workflows/release.yml` (tag classifier, prerelease flag)

…verify each of the six conditions in **Rule** still holds. The unit tests in `util::release_channel::tests` cover (1)–(4) at the resolver layer; (5) is enforced by the workflow's `classify` step and the policy is the lockstep authority that prevents re-introducing `prerelease: false` for non-stable tags.

When adding a new channel, both the resolver enum AND the workflow classifier MUST learn it in the same PR — drift between them is the canonical way to silently strand a channel.

## Exceptions

None. If a future product requirement asks for "beta releases visible to everyone" (e.g. a public beta program), update Rule (1) and (3) first and refactor the gate from `is_indigo_user` to a more general allowlist — never bypass the layering.
