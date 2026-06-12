import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

// The V4 safety flows (US-012) are handled INLINE on Home rather than by
// dedicated pages: conflicts + core drift surface as NEEDS YOU cards
// (NeedsYouCard, built from home-model) whose actions are wired to the Tauri
// commands in DesktopApp. The earlier dedicated ConflictResolutionPage /
// DriftDetailPage / CoreUpdateCard / SyncHaltedCard components shipped orphaned
// (never mounted) and were removed — this spec asserts the real, mounted path.
describe('desktop-alt V4 safety flows (US-012)', () => {
  const homeModel = readRepoFile('src/desktop-alt/v4/home-model.ts');
  const needsYouCard = readRepoFile('src/desktop-alt/v4/NeedsYouCard.svelte');
  const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
  const syncModel = readRepoFile('src/desktop-alt/lib/sync-model.ts');
  const homePage = readRepoFile('src/desktop-alt/pages/HomePage.svelte');

  it('conflicts surface as a NeedsYou card with keep-local / keep-remote / compare actions', () => {
    // Card model (home-model) carries the action ids + labels from the spec.
    expect(homeModel).toContain("id: 'keep-local', label: 'Keep mine'");
    expect(homeModel).toContain("id: 'keep-remote', label: 'Take theirs'");
    expect(homeModel).toContain("id: 'compare', label: 'Compare'");
    // The card is actually rendered on Home.
    expect(homePage).toContain('NeedsYouCard');
    expect(needsYouCard).toContain('data-testid="needs-you-card"');
  });

  it('conflict actions are wired to resolve_conflict (keep-local/keep-remote) + open_in_editor', () => {
    expect(desktopApp).toContain(
      "async function handleResolveConflict(path: string, strategy: 'keep-local' | 'keep-remote')",
    );
    expect(desktopApp).toContain("await invoke('resolve_conflict', { path, strategy })");
    expect(desktopApp).toContain("invoke('open_in_editor', { path })");
  });

  it('core drift surfaces as a NeedsYou card restored via restore_from_upstream', () => {
    expect(homeModel).toContain("id: 'restore'");
    expect(homeModel).toContain("label: 'Keep edit'");
    expect(homeModel).toContain("id: 'view-diff'");
    expect(homeModel).toContain('drifted from v');
    expect(desktopApp).toContain("await invoke('restore_from_upstream', {");
  });

  it('a conflict aborts the sync — abort-only, with NO force / override / "sync anyway" affordance', () => {
    // Hard policy hq-sync-bulk-asymmetry-breaker-means-abort: the breaker is
    // abort-only. The sync stops and surfaces an attention item; there is no UI
    // path to force/override/continue past it.
    expect(syncModel).toContain('Sync stopped because a conflict needs attention.');
    for (const surface of [homeModel, needsYouCard, desktopApp, syncModel, homePage]) {
      expect(surface).not.toMatch(/sync anyway|force[ -]?sync|override|continue anyway/i);
    }
  });
});
