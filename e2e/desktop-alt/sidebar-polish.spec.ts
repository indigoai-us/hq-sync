import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-005 — UI polish: balance item spacing + fixed-height company names with fade.
 *
 * Source-contract coverage for the two PRD e2eTests (sidebar render behavior).
 * Named sidebar-polish.spec.ts to avoid the unrelated pre-existing
 * __tests__/stories/US-005.test.ts (a DIFFERENT project's V4-Home story —
 * story-ID collision, not this PRD's US-005).
 *
 * Asserts the CSS contract that guarantees the rendered behavior:
 *  1. A very long company name keeps the row at the standard fixed row height
 *     (no growth) — single nowrap line, clipped with a right-edge mask fade
 *     instead of an ellipsis cutoff.
 *  2. The companies list scrolls on overflow while the nav above and the
 *     footer below stay fixed.
 *  3. Spacing is normalized to a shared token scale across the V4 sidebars and
 *     the desktop-alt list rows; the active-row + status-dot are not regressed.
 */

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

const tokens = readRepoFile('src/desktop-alt/v4/tokens.css');
const sidebar = readRepoFile('src/desktop-alt/v4/V4Sidebar.svelte');
const secondary = readRepoFile('src/desktop-alt/v4/V4SecondarySidebar.svelte');
const desktopAltCss = readRepoFile('src/desktop-alt/styles/desktop-alt.css');

describe('US-005: balanced spacing + fixed-height company names with fade', () => {
  it('declares a shared spacing scale in the V4 tokens (documented values)', () => {
    // 4px-based scale + canonical row height + inter-row gap, declared once.
    expect(tokens).toContain('--v4-space-1: 4px');
    expect(tokens).toContain('--v4-space-2: 8px');
    expect(tokens).toContain('--v4-space-3: 12px');
    expect(tokens).toContain('--v4-space-4: 16px');
    expect(tokens).toContain('--v4-space-5: 20px');
    expect(tokens).toContain('--v4-space-6: 24px');
    expect(tokens).toContain('--v4-row-h: 28px');
    expect(tokens).toContain('--v4-row-gap: 2px');
  });

  it('e2e-1: a long company name cannot grow the row — fixed height, nowrap, fade not ellipsis', () => {
    const css = normalize(sidebar);

    // The company row IS a .v4-row, whose height resolves from the shared token.
    expect(css).toMatch(/\.v4-row\s*\{[^}]*height:\s*var\(--v4-row-h\)/);

    // The name span is a single fixed line that never wraps/grows.
    expect(css).toMatch(/\.v4-company-name\s*\{[^}]*white-space:\s*nowrap/);
    expect(css).toMatch(/\.v4-company-name\s*\{[^}]*overflow:\s*hidden/);
    expect(css).toMatch(/\.v4-company-name\s*\{[^}]*min-width:\s*0/);

    // Clipped with a right-edge mask fade (both standard + WebKit prefix),
    // NOT a hard ellipsis cutoff.
    expect(css).toMatch(/\.v4-company-name\s*\{[^}]*-webkit-mask-image:\s*linear-gradient\(to right,/);
    expect(css).toMatch(/\.v4-company-name\s*\{[^}]*[^-]mask-image:\s*linear-gradient\(to right,/);
    expect(css).not.toMatch(/\.v4-company-name\s*\{[^}]*text-overflow:\s*ellipsis/);
  });

  it('e2e-1: the status dot stays fixed-size so name length never shifts alignment', () => {
    const css = normalize(sidebar);
    // Dot is non-shrinking; name takes the remaining width.
    expect(css).toMatch(/\.v4-dot\s*\{[^}]*flex:\s*0 0 6px/);
    expect(css).toMatch(/\.v4-company-name\s*\{[^}]*flex:\s*1 1 auto/);
  });

  it('e2e-2: the companies list scrolls on overflow while nav + footer stay fixed', () => {
    const css = normalize(sidebar);

    // Only the companies list scrolls and grows to fill.
    expect(css).toMatch(/\.v4-company-nav\s*\{[^}]*flex:\s*1 1 auto/);
    expect(css).toMatch(/\.v4-company-nav\s*\{[^}]*overflow-y:\s*auto/);
    expect(css).toMatch(/\.v4-company-nav\s*\{[^}]*min-height:\s*0/);

    // Thin-scrollbar styling preserved.
    expect(css).toMatch(/\.v4-company-nav\s*\{[^}]*scrollbar-width:\s*thin/);
    expect(css).toContain('.v4-company-nav::-webkit-scrollbar');
    expect(css).toContain('.v4-company-nav::-webkit-scrollbar-thumb');

    // The top nav does NOT grow/scroll (stays fixed above the list).
    expect(css).toMatch(/\.v4-nav\s*\{[^}]*flex:\s*0 0 auto/);
    // Footer is a non-scrolling fixed block pinned by the spacer above it.
    expect(css).toMatch(/\.v4-spacer\s*\{[^}]*flex:\s*0 0/);
  });

  it('keeps the active-row highlight (no regression)', () => {
    const css = normalize(sidebar);
    expect(css).toMatch(/\.v4-row\.active\s*\{[^}]*background:\s*var\(--v4-active-row\)/);
  });

  it('applies the shared row-height + gap tokens across the secondary sidebar and list rows', () => {
    const secondaryCss = normalize(secondary);
    expect(secondaryCss).toMatch(/\.v4-row\s*\{[^}]*height:\s*var\(--v4-row-h\)/);
    expect(secondaryCss).toMatch(/\.v4-menu\s*\{[^}]*gap:\s*var\(--v4-row-gap\)/);

    // Primary sidebar nav uses the same gap token.
    expect(normalize(sidebar)).toMatch(/\.v4-nav\s*\{[^}]*gap:\s*var\(--v4-row-gap\)/);

    // desktop-alt list rows resolve from the same row-height token.
    expect(normalize(desktopAltCss)).toMatch(/\.empty-row\s*\{[^}]*height:\s*var\(--v4-row-h/);
  });
});
