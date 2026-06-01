import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Source-contract regression guard for src-tauri/Entitlements.plist, the
// hardened-runtime entitlements file passed to `codesign --entitlements` during
// the notarized Developer ID release (scripts/sign-bundle.sh, via
// HQ_SIGN_ENTITLEMENTS in .github/workflows/release.yml).
//
// Regression for: the release dry-run failed at the very first dylib in the
// inside-out signing pass with
//   "Failed to parse entitlements: AMFIUnserializeXML: syntax error near line 9".
// Apple's entitlements deserializer (AMFIUnserializeXML) is a RESTRICTED plist
// parser — unlike `plutil -lint` (which happily accepts the file) it rejects
// XML comments. The plist had a large <!-- ... --> rationale block, so every
// `codesign --entitlements` call aborted the signing step. The scripted E2E
// harness never invokes codesign, so a malformed entitlements file passes every
// other gate and only explodes in the release pipeline — exactly the blind spot
// tauri-conf.spec.ts guards for tauri.conf.json.
//
// Rule enforced here: the entitlements plist must (a) carry the
// disable-library-validation entitlement the Team-ID-less GStreamer dylibs need
// under the hardened runtime, (b) carry com.apple.security.device.audio-input so
// the hardened-runtime app can actually be granted Microphone access — without it
// AVCaptureDevice.authorizationStatus returns .denied and macOS never prompts,
// while screen capture (which needs no entitlement) still works; that exact
// asymmetry shipped in v0.4.4-beta.2 — (c) carry com.apple.security.cs.allow-jit
// AND com.apple.security.cs.allow-unsigned-executable-memory so the Recall SDK's
// bundled GStreamer ORC JIT can allocate write+exec memory under the hardened
// runtime — without them the SDK server SIGABRT-loops ("Failed to create write
// and exec mmap regions") the instant a recording starts, so recording:started
// fires but stop never confirms and the UI hangs forever in "Stopping…" (shipped
// through v0.4.4-beta.3) — and (d) contain NO XML comments, because AMFI rejects
// them at sign time. Keep all rationale in scripts/sign-bundle.sh's header (a bash
// file AMFI never parses), never in the .plist itself.

const plistPath = fileURLToPath(
  new URL('../../src-tauri/Entitlements.plist', import.meta.url),
);
const plist = readFileSync(plistPath, 'utf8');

describe('src-tauri/Entitlements.plist (hardened-runtime signing entitlements)', () => {
  it('declares disable-library-validation = true (Team-ID-less SDK dylibs)', () => {
    // The <key> must be immediately followed by <true/> (allowing whitespace),
    // not <false/> — library validation MUST be disabled or the SDK's GStreamer
    // dylibs SIGABRT-loop under the hardened runtime.
    expect(plist).toMatch(
      /<key>com\.apple\.security\.cs\.disable-library-validation<\/key>\s*<true\/>/,
    );
  });

  it('declares cs.allow-jit = true (Recall SDK GStreamer/ORC JIT under hardened runtime)', () => {
    // The Recall SDK bundles GStreamer, whose ORC runtime compiler JITs media
    // code. Under the hardened runtime, allocating executable memory needs
    // com.apple.security.cs.allow-jit — without it the SDK server SIGABRT-loops
    // ("Failed to create write and exec mmap regions") the moment a recording
    // starts, so recording:started fires but stop never confirms and the UI
    // hangs in "Stopping…" forever. The <key> must be immediately followed by
    // <true/> (allowing whitespace).
    expect(plist).toMatch(
      /<key>com\.apple\.security\.cs\.allow-jit<\/key>\s*<true\/>/,
    );
  });

  it('declares cs.allow-unsigned-executable-memory = true (ORC legacy RWX JIT path)', () => {
    // allow-jit covers ORC's MAP_JIT path; older ORC builds map a single
    // write+exec region instead (the legacy path), which is gated by
    // com.apple.security.cs.allow-unsigned-executable-memory. The crash wording
    // ("write and exec" in one region) is the legacy path, so BOTH keys are
    // required to fully unblock the SDK media stack. The <key> must be
    // immediately followed by <true/> (allowing whitespace).
    expect(plist).toMatch(
      /<key>com\.apple\.security\.cs\.allow-unsigned-executable-memory<\/key>\s*<true\/>/,
    );
  });

  it('declares device.audio-input = true (hardened-runtime Microphone access)', () => {
    // Under the hardened runtime, an app cannot be granted Microphone access
    // without com.apple.security.device.audio-input: AVCaptureDevice
    // authorizationStatus returns .denied (2) and requestAccess never prompts.
    // The v0.4.4-beta.2 build omitted this key, so Screen Recording (no
    // entitlement required) granted fine but the Meeting Permissions wizard
    // read Microphone as "Not granted" forever. The <key> must be immediately
    // followed by <true/> (allowing whitespace).
    expect(plist).toMatch(
      /<key>com\.apple\.security\.device\.audio-input<\/key>\s*<true\/>/,
    );
  });

  it('contains NO XML comments (AMFIUnserializeXML rejects them at codesign time)', () => {
    // This is the actual regression: an XML comment anywhere in the file makes
    // `codesign --entitlements` fail with "AMFIUnserializeXML: syntax error".
    // plutil -lint does NOT catch this, so assert on the raw text.
    expect(
      plist.includes('<!--'),
      'Entitlements.plist must not contain XML comments — Apple\'s AMFI entitlements parser rejects them and codesign --entitlements fails. Move rationale to scripts/sign-bundle.sh.',
    ).toBe(false);
  });

  it('is a well-formed entitlements dict (plist + dict wrappers present)', () => {
    expect(plist).toContain('<plist version="1.0">');
    expect(plist).toMatch(/<dict>[\s\S]*<\/dict>/);
  });
});
