import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Source-contract regression guard for the Recall SDK sidecar bundle.
 *
 * `bundle.resources` in src-tauri/tauri.conf.json hand-lists which sidecar files
 * get copied into the .app. When `recording-tracker.mjs` was split out of
 * `bridge.mjs` it was NOT added to that list, so the shipped bundle imported a
 * file that wasn't there → the recall sidecar died on every launch with
 * `ERR_MODULE_NOT_FOUND` (meeting recording silently broken in 0.6.4/0.6.5).
 *
 * This test fails the build if `bridge.mjs` imports a relative `.mjs` (non-test)
 * that isn't bundled — so a future refactor that adds another sidecar module
 * can't ship the same broken bundle.
 */

const repoUrl = (rel: string) => fileURLToPath(new URL(`../../${rel}`, import.meta.url));

const conf = JSON.parse(readFileSync(repoUrl('src-tauri/tauri.conf.json'), 'utf8'));
const bridgeSrc = readFileSync(repoUrl('sidecar/recall-sdk-bridge/bridge.mjs'), 'utf8');

/** Source paths the bundler copies into the .app (the keys of bundle.resources). */
const resourceSources: string[] = Object.keys(conf.bundle?.resources ?? {});

/** Relative `./foo.mjs` specifiers imported by bridge.mjs (the bundle entrypoint). */
function relativeMjsImports(source: string): string[] {
  const out = new Set<string>();
  // Matches both `import … from './x.mjs'` and `import('./x.mjs')`, single/double quotes.
  const re = /(?:from|import)\s*\(?\s*['"](\.\/[^'"]+\.mjs)['"]/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) {
    const spec = m[1].replace(/^\.\//, '');
    if (!spec.includes('.test.')) out.add(spec);
  }
  return [...out];
}

describe('recall-sdk-bridge bundle resources', () => {
  it('bundles every relative .mjs that bridge.mjs imports', () => {
    const imports = relativeMjsImports(bridgeSrc);
    // Sanity: the refactor that motivated this guard means there is at least one.
    expect(imports.length).toBeGreaterThan(0);

    for (const spec of imports) {
      const bundled = resourceSources.some((src) =>
        src.endsWith(`sidecar/recall-sdk-bridge/${spec}`),
      );
      expect(
        bundled,
        `bridge.mjs imports "./${spec}" but it is not in tauri.conf.json bundle.resources — ` +
          `the shipped .app will crash with ERR_MODULE_NOT_FOUND. Add ` +
          `"../sidecar/recall-sdk-bridge/${spec}": "recall-sdk-bridge/${spec}".`,
      ).toBe(true);
    }
  });

  it('explicitly bundles recording-tracker.mjs (the regression)', () => {
    const bundled = resourceSources.some((src) =>
      src.endsWith('sidecar/recall-sdk-bridge/recording-tracker.mjs'),
    );
    expect(bundled).toBe(true);
  });
});
