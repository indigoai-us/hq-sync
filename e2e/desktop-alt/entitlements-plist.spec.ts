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
// under the hardened runtime, and (b) contain NO XML comments, because AMFI
// rejects them at sign time. Keep all rationale in scripts/sign-bundle.sh's
// header (a bash file AMFI never parses), never in the .plist itself.

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
