import { describe, expect, it } from 'vitest';
import {
  getConflictCardModel,
  getHomeProgressModel,
  getNeedsYouCount,
  type HomeConflict,
} from '../../src/desktop-alt/v4/home-model';
import { getV4TitleBarModel } from '../../src/desktop-alt/v4/model';
import { emptyWorkspaceStats } from '../../src/desktop-alt/lib/sync-model';
import { readRepoFile } from './harness';

/**
 * US-003 — V4 Home (healthy / syncing / error states).
 *
 * Source-contract + model harness, matching the existing desktop-alt spec
 * style. Story E2E scenarios:
 *  1. Given an unresolved conflict, when Home renders, then a needs-you card
 *     shows inline Keep mine / Take theirs actions.
 *  2. Given a running sync, when events stream, then per-company rows update
 *     and Cancel is available.
 */

function conflict(overrides: Partial<HomeConflict> = {}): HomeConflict {
  return {
    path: 'policies/slack-channel.md',
    canAutoResolve: false,
    status: 'pending',
    at: Date.now(),
    ...overrides,
  };
}

describe('desktop-alt V4 Home (US-003)', () => {
  it('an unresolved conflict renders a needs-you card with inline Keep mine / Take theirs', () => {
    const pending = [conflict()];
    expect(getNeedsYouCount(pending, null, false)).toBe(1);

    const card = getConflictCardModel(pending[0]);
    expect(card.actions.map((action) => action.label)).toEqual([
      'Keep mine',
      'Take theirs',
      'Compare',
    ]);

    // The card's resolution actions are wired to the real backend commands.
    const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
    expect(desktopApp).toContain("listen<{ path: string; localHash: string; remoteHash: string; canAutoResolve: boolean }>(");
    expect(desktopApp).toContain("await invoke('resolve_conflict', { path, strategy })");
    expect(desktopApp).toContain("invoke('open_in_editor', { path })");

    const homePage = readRepoFile('src/desktop-alt/pages/HomePage.svelte');
    expect(homePage).toContain('<NeedsYouCard');
    expect(homePage).toContain('getConflictCardModel(conflict)');
  });

  it('a running sync streams per-company fanout rows and the title bar offers Cancel', () => {
    // Mid-run snapshot: two companies done, one downloading, two queued.
    const model = getHomeProgressModel({
      filesProgressed: 187,
      totalFiles: 412,
      transferredBytes: 2_201_000,
      progress: { company: 'indigo', path: 'policies/indigo-hq-slack-channel.md', bytes: 1 },
      companies: [
        { uid: 'cmp_1', slug: 'corey-epstein' },
        { uid: 'cmp_2', slug: 'hpo' },
        { uid: 'cmp_3', slug: 'indigo' },
        { uid: 'cmp_4', slug: 'amass' },
        { uid: 'cmp_5', slug: 'keptwork' },
      ],
      statsBySlug: {
        'corey-epstein': { ...emptyWorkspaceStats(), completedFiles: 97 },
        hpo: { ...emptyWorkspaceStats(), completedFiles: 14 },
        indigo: { ...emptyWorkspaceStats(), plannedFiles: 301, progressedFiles: 76 },
      },
      workspaces: [],
    });

    expect(model.headline).toBe('187 of 412 files');
    expect(model.rows.map((row) => row.state)).toEqual(['done', 'done', 'active']);
    expect(model.rows[2].detail).toContain('downloading policies/indigo-hq-slack-channel.md');
    expect(model.queued?.count).toBe(2);

    // A later progress event for the active company updates its row in place.
    const updated = getHomeProgressModel({
      filesProgressed: 190,
      totalFiles: 412,
      transferredBytes: 2_400_000,
      progress: { company: 'indigo', path: 'docs/next.md', bytes: 1 },
      companies: [{ uid: 'cmp_3', slug: 'indigo' }],
      statsBySlug: {
        indigo: { ...emptyWorkspaceStats(), plannedFiles: 301, progressedFiles: 79 },
      },
      workspaces: [],
    });
    expect(updated.rows[0].detail).toBe('downloading docs/next.md · 79 of 301');

    // Title-bar contextual action while syncing is Cancel, wired to cancel_sync.
    const titleBar = getV4TitleBarModel({ syncState: 'syncing', watchedCount: 12 });
    expect(titleBar.action).toEqual({ id: 'cancel', label: 'Cancel' });
    const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
    expect(desktopApp).toContain("await invoke('cancel_sync')");
    expect(desktopApp).toContain('oncancel={handleCancelSync}');
  });

  it('file verb lanes are gray text, not colored (story AC)', () => {
    const digest = readRepoFile('src/desktop-alt/v4/ActivityDigest.svelte');
    const verbRule = digest.match(/\.v4-file-verb\s*\{[\s\S]*?\}/)?.[0] ?? '';
    expect(verbRule).toContain('color: var(--v4-text-2)');
    expect(verbRule).not.toMatch(/--v4-(ok|warn|error|unread)/);
  });

  it('sync-halted discipline: Home offers no override/force affordance', () => {
    for (const path of [
      'src/desktop-alt/pages/HomePage.svelte',
      'src/desktop-alt/v4/home-model.ts',
      'src/desktop-alt/v4/NeedsYouCard.svelte',
    ]) {
      const source = readRepoFile(path);
      expect(source).not.toMatch(/sync anyway|force sync|override/i);
    }
  });
});
