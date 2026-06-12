import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const notesPath = resolve(process.cwd(), 'docs/design/v4/IMPLEMENTATION-NOTES.md');
const notes = readFileSync(notesPath, 'utf8');
const safetyFlows = readFileSync(
  resolve(process.cwd(), 'e2e/desktop-alt/safety-flows.spec.ts'),
  'utf8',
);
const secretsSpec = readFileSync(
  resolve(process.cwd(), 'e2e/desktop-alt/secrets-never-leak.spec.ts'),
  'utf8',
);
const desktopStyle = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/styles/desktop-alt.css'),
  'utf8',
);

describe('US-017: full-suite verification release guard', () => {
  it('documents the visual QA map and intentional deviations for the V4 PNG set', () => {
    expect(existsSync(notesPath)).toBe(true);
    for (const reference of [
      'chrome-master.png',
      'home-healthy.png',
      'home-syncing.png',
      'home-error.png',
      'companies.png',
      'company-overview.png',
      'company-goals.png',
      'company-projects.png',
      'company-tasks.png',
      'story-detail.png',
      'project-detail.png',
      'company-activity.png',
      'company-deployments.png',
      'company-secrets.png',
      'messages-*.png',
      'conflict-resolution.png',
      'drift-detail.png',
      'core-update.png',
      'sync-halted.png',
      'settings.png',
      'first-run.png',
      'library.png',
      'marketplace.png',
      'creator-profile-moderation.png',
      'meetings.png',
      'meeting-permissions.png',
      'banners-palette.png',
      'system-states.png',
    ]) {
      expect(notes).toContain(reference);
    }
    expect(notes).toContain('Intentional Deviations');
    expect(notes).toContain('tauri-driver');
  });

  it('names the required full-suite commands in the release notes', () => {
    for (const command of [
      'npm run typecheck',
      'npm run lint',
      'npm test',
      'npm run test:e2e:desktop-alt',
    ]) {
      expect(notes).toContain(command);
    }
  });

  it('keeps the critical safety and secrets specs wired', () => {
    expect(secretsSpec).toContain('desktop-alt secrets never leak');
    expect(secretsSpec).toContain('metadata only');
    // V4 safety flows are handled inline (NeedsYouCard) rather than by dedicated
    // pages; the spec asserts the real conflict/drift wiring and the abort-only
    // guard (hard policy hq-sync-bulk-asymmetry-breaker-means-abort).
    expect(safetyFlows).toContain('resolve_conflict');
    expect(safetyFlows).toContain('restore_from_upstream');
    expect(safetyFlows).toContain('hq-sync-bulk-asymmetry-breaker-means-abort');
    expect(safetyFlows).toContain('Sync stopped because a conflict needs attention.');
  });

  it('keeps V4 styling isolated to desktop-alt while reusing popover tokens', () => {
    expect(desktopStyle).toContain("@import '../../styles/popover.css'");
    expect(desktopStyle).toContain('--desktop-titlebar-height');
    expect(notes).toContain('Menubar/popover behavior remains outside the V4 desktop-alt route changes');
  });
});
