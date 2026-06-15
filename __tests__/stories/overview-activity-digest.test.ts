import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// The company Overview surfaces a "Team activity" digest built from REAL data
// the desktop already fetches (`get_company_activity`, warmed via companyStore) —
// 7-day edits/files, members, vault size, edits-over-time, and top contributors.
// No backend change, no fabricated values: the same signal the Activity tab uses.

const read = (p: string) => readFileSync(resolve(process.cwd(), p), 'utf8');

describe('company Overview team-activity digest', () => {
  it('is rendered on the company Overview (CompanyBoardPanel)', () => {
    const panel = read('src/desktop-alt/panels/CompanyBoardPanel.svelte');
    expect(panel).toContain("import OverviewActivityDigest from '../components/OverviewActivityDigest.svelte'");
    // Rendered with the company slug + cloud-backed flag, in the overview body.
    expect(panel).toMatch(/<OverviewActivityDigest\s+\{slug\}\s+\{cloudBacked\}\s*\/>/);
  });

  it('pulls real activity data (no fabricated values) and reuses the shared cache', () => {
    const c = read('src/desktop-alt/components/OverviewActivityDigest.svelte');
    // Real source: the same command the Activity tab uses.
    expect(c).toContain("invoke<Partial<CompanyActivity>>('get_company_activity'");
    // Warmed/shared through companyStore so it does not double-fetch.
    expect(c).toContain('companyStore.activity(slug)');
    expect(c).toContain('companyStore.setActivity(slug, result)');
    // Surfaces the real fields.
    expect(c).toContain('activity.stats.edits7');
    expect(c).toContain('activity.stats.vaultSize');
    expect(c).toContain('activity.top');
    expect(c).toContain('activity.sparkline');
    // Honest empty state, not a faked filler.
    expect(c).toContain('No activity yet');
  });
});
