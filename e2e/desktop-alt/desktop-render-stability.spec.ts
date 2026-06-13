import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * Regression — the desktop window must never hard-reload itself or tear down its
 * chrome to refresh the workspace list.
 *
 * Context: a "Fix desktop local data surfaces" patch (commit 22e4832) added
 * `window.location.reload()` inside loadWorkspaces() — which fires on the initial
 * cache→live swap, on every window focus, and on every sync:all-complete — plus
 * `{#key renderWorkspaceCount}` remounts of the title bar, sidebar, and status
 * bar. The reload mid-paint is what blanked/froze the desktop. The chrome is
 * already reactive (V4Sidebar derives its model from the `companies` prop;
 * V4TitleBar / DesktopStatusBar are pure $props consumers), so reassigning
 * renderCompanies refreshes everything without a reload or a remount.
 */
describe('desktop render stability', () => {
  const app = readRepoFile('src/desktop-alt/DesktopApp.svelte');

  it('never reloads the document to refresh the workspace list', () => {
    expect(app).not.toContain('window.location.reload');
    expect(app).not.toContain('WORKSPACE_RELOAD_KEY');
    expect(app).not.toContain('workspaceSignature');
  });

  it('does not tear down the chrome on a workspace-count change', () => {
    expect(app).not.toContain('{#key renderWorkspaceCount}');
  });

  it('keeps the dev-only render audit out of production builds', () => {
    const at = app.indexOf('function queueDesktopRenderAudit');
    expect(at).toBeGreaterThan(-1);
    const fn = app.slice(at, at + 500);
    expect(fn).toContain('import.meta.env.DEV');
  });
});
