import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('desktop-alt V4 safety flows (US-012)', () => {
  const conflict = readRepoFile('src/desktop-alt/pages/ConflictResolutionPage.svelte');
  const drift = readRepoFile('src/desktop-alt/pages/DriftDetailPage.svelte');
  const update = readRepoFile('src/desktop-alt/v4/CoreUpdateCard.svelte');
  const halted = readRepoFile('src/desktop-alt/v4/SyncHaltedCard.svelte');

  it('conflict resolution is side-by-side and wired to resolve_conflict', () => {
    expect(conflict).toContain('class="compare-grid"');
    expect(conflict).toContain('class="version-pane selected"');
    expect(conflict).toContain('class="changed-region"');
    expect(conflict).toContain('localOwner');
    expect(conflict).toContain('remoteOwner');
    expect(conflict).toContain("await invoke('resolve_conflict', { path: currentConflict.path, strategy })");
    expect(conflict).toContain("resolveCurrent('keep-local')");
    expect(conflict).toContain("resolveCurrent('keep-remote')");
    expect(conflict).toContain("invoke('open_in_editor', { path: currentConflict.path })");
    expect(conflict).toContain('Decide later');
    expect(conflict).toContain('progressLabel');
    expect(conflict).toContain('Both versions are retained until you choose one.');
  });

  it('drift detail renders all row types and restores via restore_from_upstream', () => {
    expect(drift).toContain("await invoke<CoreState | null>('check_core_state')");
    expect(drift).toContain('MODIFIED');
    expect(drift).toContain('MISSING');
    expect(drift).toContain('ADDED');
    expect(drift).toContain("await invoke('restore_from_upstream', {");
    expect(drift).toContain('expectedUpstreamSha: entry.gitShaUpstream');
    expect(drift).toContain('targetRepo: report.targetRepo');
    expect(drift).toContain('targetRef: report.targetRef');
    expect(drift).toContain('Keep edit');
    expect(drift).toContain('Keep missing');
    expect(drift).toContain('Keep file');
  });

  it('core update card has available, in-progress, and failed states wired to install_hq_core_update', () => {
    expect(update).toContain("'available' | 'in-progress' | 'failed'");
    expect(update).toContain("await invoke<string>('install_hq_core_update')");
    expect(update).toContain('logTail');
    expect(update).toContain('Update log tail');
    expect(update).toContain('Install update');
    expect(update).toContain('Updating…');
    expect(update).toContain('Try again');
  });

  it('sync halted is abort-only with no force or override affordance', () => {
    expect(halted).toContain('SYNC HALTED');
    expect(halted).toContain('Abort sync');
    expect(halted).toContain('onabort');
    expect(halted).not.toMatch(/force|override|continue anyway/i);
  });
});
