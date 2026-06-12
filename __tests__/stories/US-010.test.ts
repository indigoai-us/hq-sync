import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const companyPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompanyPage.svelte'),
  'utf8',
);
const activityPanel = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/panels/ActivityPanel.svelte'),
  'utf8',
);
const statTile = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/StatTile.svelte'),
  'utf8',
);
const sparkline = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/Sparkline.svelte'),
  'utf8',
);
const tauriMain = readFileSync(resolve(process.cwd(), 'src-tauri/src/main.rs'), 'utf8');

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-010: Activity panel reads company activity via Tauri command', () => {
  it('wires the activity tab to ActivityPanel with the selected company slug', () => {
    const page = normalize(companyPage);
    const panel = normalize(activityPanel);

    expect(page).toContain("import ActivityPanel from '../panels/ActivityPanel.svelte'");
    expect(page).toContain('<ActivityPanel slug={company.slug} {cloudBacked} />');
    expect(page).not.toContain('Activity panel - wired in US-010');
    expect(panel).toContain("void invoke<Partial<CompanyActivity>>('get_company_activity', { slug })");
    expect(panel).toContain('return () => { cancelled = true; };');
    expect(panel).toContain('function retry() { reloadToken += 1; }');
    expect(panel).toContain("console.error('get_company_activity failed:', err)");
    expect(tauriMain).toContain('commands::desktop_alt::get_company_activity');
  });

  it('renders four stat tiles with the required 14-day labels and defaults', () => {
    const panel = normalize(activityPanel);

    expect(panel).toContain('<StatTile label="New files · 14d" value={activity.stats.files7} {loading} />');
    expect(panel).toContain('<StatTile label="Edits · 14d" value={activity.stats.edits7} {loading} />');
    expect(panel).toContain('<StatTile label="Members" value={activity.stats.members} {loading} />');
    expect(panel).toContain('<StatTile label="Vault size" value={activity.stats.vaultSize || \'0\'} {loading} />');
    expect(panel).toContain('files7: 0');
    expect(panel).toContain('edits7: 0');
    expect(panel).toContain('members: 0');
    expect(statTile).toContain('aria-busy={loading}');
    expect(statTile).toContain('.label-skeleton');
    expect(statTile).toContain('.value-skeleton');
  });

  it('implements the prototype sparkline math and CSS bar height animation', () => {
    const panel = normalize(activityPanel);
    const spark = normalize(sparkline);

    expect(spark).toContain('const max = $derived(Math.max(1, ...data))');
    expect(spark).toContain('const stepX = $derived(data.length > 1 ? width / (data.length - 1) : width)');
    expect(spark).toContain('`${(index * stepX).toFixed(1)},${(height - (value / max) * (height - 2) - 1).toFixed(1)}`');
    expect(spark).toContain('<polyline points={points} fill="none" stroke="currentColor" stroke-width="1" opacity="0.85" />');
    expect(panel).toContain('<Sparkline data={activity.sparkline} width={120} height={20} />');
    expect(panel).toContain('{#each activity.sparkline as value, index (index)}');
    expect(panel).toContain('style={`height: ${barHeight(value)}`}');
    expect(panel).toContain('transition: height 300ms ease');
    expect(panel).toContain('{#each Array(14) as _, index (index)}');
  });

  it('renders empty states and per-section skeleton loaders while loading', () => {
    const panel = normalize(activityPanel);

    expect(panel).toContain('<div class="chart-skeleton" aria-label="Loading edits over time">');
    expect(panel).toContain('<div class="contributor-skeleton" aria-label="Loading top contributors">');
    expect(panel).toContain('<div class="recent-skeleton" aria-label="Loading recent files">');
    expect((activityPanel.match(/No activity yet/g) ?? []).length).toBeGreaterThanOrEqual(3);
    expect(panel).toContain('{#if loading}');
    expect(panel).toContain('{:else if activity.sparkline.length > 0}');
    expect(panel).toContain('{:else if activity.top.length > 0}');
    expect(panel).toContain('{:else if recentGroups.length > 0}');
  });

  it('renders top contributor bars and the V4 actor-grouped recent feed from backend data', () => {
    const panel = normalize(activityPanel);

    expect(panel).toContain('const contributorMax = $derived(Math.max(1, ...activity.top.map((contributor) => contributor.edits)))');
    expect(panel).toContain('return `${(edits / contributorMax) * 100}%`;');
    expect(panel).toContain('<span class="contributor-fill" style={`width: ${contributorWidth(contributor.edits)}`} ></span>');
    expect(panel).toContain('{#each activity.top as contributor, index (`${contributor.who}:${index}`)}');
    expect(panel).toContain("let activityDirection = $state<ActivityDirection>('all')");
    expect(panel).toContain('const filteredRecent = $derived(activity.recent.filter((entry) => matchesDirection(entry, activityDirection)))');
    expect(panel).toContain('const recentGroups = $derived(groupRecentActivity(filteredRecent))');
    expect(panel).toContain('class:is-active={activityDirection === \'outgoing\'}');
    expect(panel).toContain('{#each recentGroups as group (`actor:${group.who}`)}');
    expect(panel).toContain('<header class="actor-header">');
    expect(panel).toContain('<span class="avatar" title={group.who}>{initialFor(group.who)}</span>');
    expect(panel).toContain('{#each group.entries as entry, index (`${entry.file}:${entry.when}:${index}`)}');
    expect(panel).toContain('<span class="verb-lane" title={entry.what}>{verbLane(entry.what)}</span>');
    expect(panel).toContain('<strong title={entry.file}>{entry.file}</strong>');
    expect(panel).toContain('<span>{entry.what}</span>');
    expect(panel).toContain('<time class="date-chip">{dateChip(entry.when)}</time>');
    // US-012: the per-recent-row "Open" affordance is now a live Claude Code
    // drill-in (replacing the old `disabled` placeholder button), gated to
    // entries that actually name a file.
    expect(panel).toContain("import OpenFileInClaudeCode from '../components/OpenFileInClaudeCode.svelte'");
    expect(panel).toContain("{#if entry.file && entry.file !== 'Untitled file'}");
    expect(panel).toContain('<OpenFileInClaudeCode file={entry.file} folder={hqFolderPath} label="Open" />');
  });
});
