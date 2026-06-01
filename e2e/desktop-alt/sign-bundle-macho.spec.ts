import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Source-contract regression guard for scripts/sign-bundle.sh, the inside-out
// codesign pass used by the notarized Developer ID release
// (.github/workflows/release.yml → Sign step, HARDENED_RUNTIME=1 + TIMESTAMP=1
// + HQ_SIGN_ENTITLEMENTS).
//
// Regression for: the release dry-run (run 26725856981) PASSED the Sign step
// but FAILED the Notarize step. Apple's notary rejected a single unsigned
// Mach-O — the GStreamer umbrella binary at
//   .../GStreamer.framework/Versions/1.0/lib/GStreamer
// (a fat x86_64+arm64 binary). Root cause: Phase 1 signed only files matching
// `*.dylib`, and Phase 2 signed only the top-level Versions/1.0/GStreamer copy.
// The umbrella binary also ships at Versions/1.0/lib/GStreamer (a SECOND,
// distinct-inode, byte-identical copy with no .dylib extension), so nothing
// signed it.
//
// The trap: `codesign --verify --deep --strict` (Phase 9) PASSES with that
// binary unsigned — codesign only seals nested Mach-O as hashed resources and
// does not require each to carry its own signature. Only Apple's notary
// enforces per-Mach-O Developer-ID signing + secure timestamp, so the gap is
// invisible until ~8 min into the release at the Notarize step. The scripted
// E2E harness never runs codesign, so this can only be guarded at the source
// level — exactly like entitlements-plist.spec.ts.
//
// Rule enforced here: sign-bundle.sh must sign every Mach-O inside the
// framework by CONTENT (not by `.dylib` name), and must cover the umbrella
// binary at Versions/1.0/lib/GStreamer specifically.

const scriptPath = fileURLToPath(
  new URL('../../scripts/sign-bundle.sh', import.meta.url),
);
const script = readFileSync(scriptPath, 'utf8');

describe('scripts/sign-bundle.sh (inside-out notarization signing pass)', () => {
  it('signs framework Mach-O by content, not by a *.dylib name glob', () => {
    // The bug was `find "$GST_FRAMEWORK" -type f -name "*.dylib"` — a name glob
    // that skips the extension-less GStreamer umbrella binaries. The framework
    // scan must enumerate ALL regular files and classify each as Mach-O.
    expect(
      script.includes('find "$GST_FRAMEWORK" -type f -name "*.dylib"'),
      'sign-bundle.sh must NOT restrict the GStreamer.framework scan to *.dylib — that misses the extension-less umbrella Mach-O the notary rejects. Scan every file and classify by content.',
    ).toBe(false);

    // The framework scan must enumerate every regular file...
    expect(script).toContain('find "$GST_FRAMEWORK" -type f -print0');
    // ...and classify Mach-O by content (`file`), signing only those.
    expect(script).toMatch(/file -b "\$f"/);
    expect(script).toMatch(/\*Mach-O\*/);
  });

  it('covers the umbrella binary at Versions/1.0/lib/GStreamer (the path notary rejected)', () => {
    // The specific Mach-O that failed notarization. A content-based Phase 1
    // catches it, but pin the path so a future refactor cannot silently drop
    // the second umbrella copy again.
    expect(
      script.includes('Versions/1.0/lib/GStreamer'),
      'sign-bundle.sh must account for the GStreamer umbrella binary at Versions/1.0/lib/GStreamer — the exact unsigned Mach-O that failed notarization in run 26725856981.',
    ).toBe(true);
  });

  it('verifies the umbrella binaries are signed at sign time (fail fast, not at notary)', () => {
    // Belt-and-suspenders guard: assert sign-bundle.sh re-checks the umbrella
    // binaries with `codesign --verify` and aborts the Sign step if either is
    // unsigned — surfacing a glob/scan regression immediately instead of ~8 min
    // later at the Notarize step.
    expect(script).toMatch(/codesign --verify --strict "\$umbrella"/);
    expect(script).toMatch(/unsigned after Phase 1/);
  });
});
