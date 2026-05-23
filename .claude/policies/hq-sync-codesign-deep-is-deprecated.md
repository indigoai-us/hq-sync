---
id: hq-sync-codesign-deep-is-deprecated
title: Use inside-out codesigning for hq-sync bundles — never --deep
scope: repo
trigger: Any code-signing operation against the hq-sync `.app` bundle or any bundle that embeds a framework / dylib tree (e.g. `@recallai/desktop-sdk` with `GStreamer.framework`)
enforcement: hard
version: 1
created: 2026-05-23
updated: 2026-05-23
public: false
---

# hq-sync — codesign --deep is deprecated and broken for embedded frameworks

## Rule

NEVER use `codesign --deep` to sign the hq-sync `.app` bundle. Always go
inside-out via `scripts/sign-bundle.sh`. The `bundle:debug` and
`bundle:release` npm scripts already invoke it; ad-hoc command-line sign
attempts must follow the same pattern.

## Why

`codesign --deep` walks the bundle in a directory-traversal order that
doesn't respect dylib dependency edges. For bundles with a framework
tree (we ship the Recall Desktop SDK + GStreamer.framework with ~100
dylibs) that breaks load-command verification on inner dylibs and the
process SIGABRTs at first dlopen. We hit the symptom as a SIGKILL loop
on `libgstaudio-1.0.0.dylib` during the meeting-detect-notify project
(2026-05-21) and reverted to ad-hoc signing while iterating — but
ad-hoc signing breaks TCC bundle-identity attribution, so the user
loses permission grants on every rebuild.

Inside-out signing fixes both:

1. Every inner dylib gets signed in isolation — load-command bytes stay
   consistent because signing one dylib doesn't perturb another.
2. Each framework is sealed (signed-as-a-directory) after its contents,
   so the framework's own `_CodeSignature` reflects the final hashes.
3. The host `.app` is sealed last with all-leaf hashes locked in, so
   TCC's "code requirement" is stable across rebuilds (same identity +
   bundle ID + designated requirement → same TCC grants persist).

Apple's own [code-signing
docs](https://developer.apple.com/documentation/security/code_signing_services)
and the [Tauri distribution
guide](https://tauri.app/v1/guides/distribution/sign-macos) both
recommend this order. `--deep` is marked deprecated in the codesign
man page.

## Pattern

The canonical pipeline lives in `scripts/sign-bundle.sh`:

```
Phase 1: every dylib under embedded frameworks (~100 for our SDK)
Phase 2: framework mach-O (Versions/1.0/<Name>)
Phase 3: seal the framework directory
Phase 4: top-level dylibs in Frameworks/
Phase 5: SDK child binaries (desktop_sdk_macos_exe)
Phase 6: Contents/MacOS sidecars (bash wrapper, helper exes)
Phase 7: main app binary (hq-sync-menubar)
Phase 8: seal the .app bundle
```

Each phase calls `codesign --force --sign <IDENTITY>` (plus
`--options runtime` and `--entitlements` for distribution signs).
`--deep` is NEVER passed.

## Config

Distribution build (Developer ID Application cert with Team ID):

```bash
HARDENED_RUNTIME=1 \
TIMESTAMP=1 \
HQ_SIGN_ENTITLEMENTS=src-tauri/entitlements.plist \
bash scripts/sign-bundle.sh "path/to/HQ Sync.app" "Developer ID Application: …"
```

Local dev cert (self-signed "HQ Installer Dev", no Team ID):

```bash
bash scripts/sign-bundle.sh "path/to/HQ Sync.app"
```

Hardened runtime is OFF by default because self-signed certs have no
Team ID, and hardened runtime then refuses to load dylibs whose own
Team-ID-less signatures don't match the host process. This is fine for
local dev. Distribution builds MUST opt back in via
`HARDENED_RUNTIME=1` and either:

- Pair with a real Developer ID Application cert (provides a Team ID),
  or
- Add `com.apple.security.cs.disable-library-validation` to the
  entitlements file (acceptable for the Recall SDK which Apple won't
  sign for us, NOT acceptable for app code we control).

## Verification

After signing, the canonical health check:

```bash
codesign --verify --deep --strict --verbose=2 "path/to/HQ Sync.app"
codesign -dvv "path/to/HQ Sync.app" | grep -E "(Identifier|Authority|TeamIdentifier|Sealed)"
```

Both must succeed. The Sealed Resources file count should match
expectations (~639 files for the current SDK bundle).

## Trigger conditions to watch for

- `dyld: Library not loaded: @rpath/...` — load-command verification
  failed. Almost always a `--deep` sign or a partial inside-out (a
  dylib didn't get signed before its container was sealed).
- `code signature in '...' not valid for use in process: mapping
  process and mapped file (non-platform) have different Team IDs` —
  hardened runtime with mismatched Team IDs. Either drop
  `--options runtime` (dev cert) or add `disable-library-validation`
  entitlement.
- `Failed to match existing code requirement for subject <bundle-id>
  and service <kTCC...>` — TCC has a stale code-requirement cache.
  Reset with `tccutil reset All <bundle-id>` (bundle-scoped — NEVER
  `tccutil reset <Service>` without a bundle ID; that wipes every
  app's grants for that service).
