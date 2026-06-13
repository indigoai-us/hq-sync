import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-011 — Deployments panel actions.
 *
 * Source-contract harness (same style as board-surface.spec.ts). Asserts that
 * DeploymentRow.svelte wires the three actionable behaviours over the
 * deployment data shape that `get_company_deployments` already returns
 * (sub, url, state, lastDeploy, size, ver, pwd — no logs/commit fields exist,
 * so none are invented):
 *
 *   1. Open URL — the live URL opens in the default browser via the
 *      `@tauri-apps/plugin-shell` `open()` (same pattern US-001 used), with the
 *      deployment URL.
 *   2. Detail drill-in — an expand affordance surfaces status + the other
 *      detail fields the data already carries (last deploy, size, version,
 *      access), gated behind an `expanded` state.
 *   3. Redeploy path — there is NO parameterless redeploy/trigger endpoint in
 *      hq-deploy (the only deploy route, `POST /api/apps/:id/deploy`, requires
 *      an artifact upload the desktop app does not hold), so the row shows a
 *      calm "Managed via hq-deploy" note and adds NO redeploy button/command.
 *      This test asserts that absence.
 *   4. Affordances — hover/cursor/focus-visible ring on the drill-in controls.
 *   5. Tokens — no hardcoded hex in the component styles.
 */

describe('desktop-alt Deployments panel actions (US-011)', () => {
  it('opens the live deployment URL in the browser via plugin-shell (US-001 pattern)', () => {
    const row = readRepoFile('src/desktop-alt/components/DeploymentRow.svelte');

    // Same import + call shape US-001 established in CompanyPage.svelte.
    expect(row).toContain("import { open } from '@tauri-apps/plugin-shell'");
    // open() is called with the deployment's URL (https-prefixed host).
    expect(row).toContain('await open(`https://${deployment.url}`)');
    // The open button is wired to the open handler.
    expect(row).toContain('onclick={openDeployment}');
  });

  it('surfaces a detail drill-in over the existing deployment data shape (status + carried fields)', () => {
    const row = readRepoFile('src/desktop-alt/components/DeploymentRow.svelte');

    // Drill-in is gated behind an expand toggle (row + more button both flip it).
    expect(row).toContain("let expanded = $state(false)");
    expect(row).toContain('function toggleDetail()');
    expect(row).toContain('aria-expanded={expanded}');
    expect(row).toContain('{#if expanded}');
    expect(row).toContain('class="deployment-detail"');

    // The detail surfaces ONLY fields the data already carries — status, last
    // deploy, size, version, URL, access. No invented logs/commit fields.
    expect(row).toContain('>Status<');
    expect(row).toContain('{stateLabel}');
    expect(row).toContain('>Last deploy<');
    expect(row).toContain('{deployment.lastDeploy}');
    expect(row).toContain('>Size<');
    expect(row).toContain('{deployment.size}');
    expect(row).toContain('>Version<');
    expect(row).toContain('{deployment.ver}');
    expect(row).toContain('>Access<');
    expect(row).toContain("deployment.pwd ? 'Password protected' : 'Public'");
    // No fabricated backend fields leaked into the UI shape.
    expect(row).not.toMatch(/deployment\.(logs|logsUrl|commit|sha|branch|repo)\b/);
  });

  it('takes the calm-note redeploy path — no dead redeploy button or command (no trigger surface)', () => {
    const row = readRepoFile('src/desktop-alt/components/DeploymentRow.svelte');
    const rust = readRepoFile('src-tauri/src/commands/desktop_alt.rs');
    const main = readRepoFile('src-tauri/src/main.rs');
    const caps = readRepoFile('src-tauri/capabilities/desktop-alt.json');

    // Calm note instead of an action: "Managed via hq-deploy".
    expect(row).toContain('class="detail-note"');
    expect(row).toContain('Managed via');
    expect(row).toContain('hq-deploy');

    // No redeploy ACTION (button/handler/invoke) is wired anywhere — the word
    // "redeploy" only appears in the calm prose note, never as a control.
    expect(row).not.toMatch(/function (redeploy|onRedeploy)/i);
    expect(row).not.toMatch(/onclick=\{[^}]*[Rr]edeploy/);
    expect(row).not.toMatch(/aria-label=\{?[`"][^`"]*[Rr]edeploy/);
    expect(row).not.toMatch(/invoke\(/);

    // No redeploy/trigger Tauri command was added (would have to be registered
    // in main.rs + allow-listed in capabilities — assert none of that exists).
    expect(rust).not.toMatch(/fn (redeploy|trigger_deploy|trigger_redeploy)/);
    expect(main).not.toMatch(/redeploy|trigger_deploy/);
    expect(caps).not.toMatch(/redeploy|trigger_deploy/);
  });

  it('carries no dead or no-op action controls in the row detail (honesty)', () => {
    const row = readRepoFile('src/desktop-alt/components/DeploymentRow.svelte');

    // The old per-row "Rollback" presented a destructive confirm whose Confirm
    // handler was a pure no-op (it just closed the dialog) — a control that
    // implied a revert happened when nothing did. It is gone entirely, along
    // with the permanently-disabled dead "Deploy" button. The honest actionable
    // path is the panel-level Deploy (which opens the hq-deploy agent workflow).
    expect(row).not.toContain('rollbackConfirm');
    expect(row).not.toMatch(/function (beginRollback|cancelRollback|confirmRollback)/);
    expect(row).not.toMatch(/>\s*Rollback\s*</);
    // No permanently-disabled action button masquerading as a control.
    expect(row).not.toMatch(/<button[^>]*\sdisabled[^>]*>[\s\S]*?Deploy/);
    expect(row).not.toContain('class="detail-actions"');
    expect(row).not.toContain('class="rollback-confirm"');

    // The calm note still points at the real, working entry points.
    expect(row).toContain('class="detail-note"');
    expect(row).toContain('Managed via');
    expect(row).toContain('deploy workflow');
  });

  it('gives the drill-in proper affordances (cursor, hover, focus ring)', () => {
    const row = readRepoFile('src/desktop-alt/components/DeploymentRow.svelte');
    const styleBlock = row.split('<style>')[1] ?? '';

    expect(styleBlock).toContain('cursor: pointer');
    expect(styleBlock).toContain('.subdomain-cell:hover');
    expect(styleBlock).toContain(':focus-visible');
    expect(styleBlock).toContain('outline:');
  });

  it('keeps the row token-driven (no hardcoded hex)', () => {
    const styleBlock =
      readRepoFile('src/desktop-alt/components/DeploymentRow.svelte').split('<style>')[1] ?? '';
    expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });
});
