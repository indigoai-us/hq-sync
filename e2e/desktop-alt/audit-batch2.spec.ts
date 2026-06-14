import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

// Source-contract assertions locking the audit-batch-2 fixes (adversarially
// confirmed findings). These wire behaviors that can't be unit-tested without a
// DOM/Tauri runtime, so we assert the contract in source — a dropped wire fails
// fast without a macOS build. Mirrors the US-* / cli-update-notice story tests.

describe('audit batch 2: confirmed-finding fixes', () => {
  it('desktop shell listens for sync:setup-needed and does not let all-complete clobber it', () => {
    const app = readRepoFile('src/desktop-alt/DesktopApp.svelte');
    // The listener that was missing — without it the status bar showed
    // "Idle · all safe" for an un-provisioned brand-new account.
    expect(app).toContain("listen('sync:setup-needed'");
    expect(app).toContain("syncState = 'setup-needed'");
    // all-complete must preserve the setup-needed state (not reset to idle).
    expect(app).toContain("syncState !== 'setup-needed'");
  });

  it('invited channel renders a read-only preview, not a fake working composer', () => {
    const view = readRepoFile('src/components/messaging/ChannelView.svelte');
    // The invited Conversation must be readonly so a typed message can't silently
    // vanish through a no-op onsend.
    const invitedBlock = view.slice(view.indexOf('{#if invited}'));
    expect(invitedBlock).toContain('readonly={true}');
  });

  it('messages rail load errors offer a retry instead of a dead-end', () => {
    const shell = readRepoFile('src/components/messaging/MessagesShell.svelte');
    expect(shell).toContain('class="rail-retry"');
    expect(shell).toContain('onclick={() => loadContacts()}');
    expect(shell).toContain('onclick={() => loadRequests()}');
  });

  it('CreateChannel company dropdown never shows a raw cmp_ UID', () => {
    const create = readRepoFile('src/components/messaging/CreateChannel.svelte');
    expect(create).not.toContain('|| co.companyUid}</option>');
    expect(create).toContain("co.companyName?.trim() || 'Company'");
  });

  it('command palette always closes even if a command action throws', () => {
    const palette = readRepoFile('src/desktop-alt/components/CommandPalette.svelte');
    // try/finally so a throwing action can't leave the modal palette stuck open.
    expect(palette).toContain('try {');
    expect(palette).toContain('await command.action();');
    expect(palette).toContain('} finally {');
    expect(palette).toContain('onclose();');
  });

  it('deployments counts read as "unknown" (—) on a load error, not a fake empty', () => {
    const panel = readRepoFile('src/desktop-alt/panels/DeploymentsPanel.svelte');
    expect(panel).toContain("{error ? '—' : activeCount}");
    expect(panel).toContain('error ? "Couldn\'t load"');
  });
});
