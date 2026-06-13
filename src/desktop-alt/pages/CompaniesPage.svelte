<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { Workspace } from '../../lib/workspaces';
  import { buildClaudeCodeUrl } from '../../lib/claude-code-link';
  import { hqSkillMarkdownLink } from '../../lib/hq-skill-link';
  import {
    getCompaniesPageModel,
    type CompanySyncMode,
  } from '../v4/companies-model';
  import '../v4/tokens.css';

  /**
   * V4 Companies overview — the one page managing connection state (SPEC
   * section 5, companies.png): a CONNECTED table (role / members / last
   * change / sync lanes, amber provisioning rows, red error rows with Retry)
   * and a NOT CONNECTED section (local directories → Connect via
   * `connect_workspace_to_cloud`; pending invites → Open invite flow).
   *
   * Workspaces are prop-fed from DesktopApp's `list_syncable_workspaces`
   * load; per-company sync modes resolve lazily here via `get_sync_mode`
   * (best-effort — a failure leaves the lane at its "Auto"/"Manual" default).
   */
  interface Props {
    workspaces: Workspace[];
    /** False during the first real-state fetch (skeleton instead of 0-flash). */
    ready?: boolean;
    /** `realtimeSync` preference; null while loading. */
    autoSyncOn?: boolean | null;
    /** Top-level workspaces load error, surfaced as a quiet inline line. */
    workspaceError?: string | null;
    /** Row click on a synced company opens its workspace. */
    onopencompany?: (slug: string) => void;
    /** Called after a successful Connect so the parent re-fetches workspaces. */
    onrefresh?: () => void;
  }

  let {
    workspaces,
    ready = true,
    autoSyncOn = null,
    workspaceError = null,
    onopencompany,
    onrefresh,
  }: Props = $props();

  // Slugs with an in-flight Connect — rendered as amber provisioning rows.
  let connecting = $state<string[]>([]);
  // slug → message from a failed Connect attempt.
  let connectErrors = $state<Record<string, string>>({});
  // slug → resolved per-membership sync mode (get_sync_mode).
  let syncModes = $state<Record<string, CompanySyncMode>>({});
  // Invites are accepted from the emailed magic link. The desktop membership
  // row carries inviter/time metadata but not the one-time token, so this opens
  // the real /accept workflow and asks the user for the link/token there.
  let openingInvite = $state<string | null>(null);
  let inviteNotices = $state<Record<string, string>>({});

  // One lazy get_sync_mode round-trip per synced company row. Non-reactive
  // guard so a re-render never re-requests a slug already in flight.
  const requestedModes = new Set<string>();

  $effect(() => {
    for (const workspace of workspaces) {
      if (workspace.kind !== 'company' || workspace.state !== 'synced') continue;
      const slug = workspace.slug;
      if (requestedModes.has(slug)) continue;
      requestedModes.add(slug);
      void invoke<{ syncMode: CompanySyncMode }>('get_sync_mode', { companySlug: slug })
        .then((cfg) => {
          syncModes = { ...syncModes, [slug]: cfg.syncMode };
        })
        .catch((err) => {
          // A freshly-connected company with no sync-config row, or a vault
          // blip — leave the lane at its default rather than spamming errors.
          console.warn(`get_sync_mode(${slug}) failed:`, err);
        });
    }
  });

  const model = $derived(
    getCompaniesPageModel({
      workspaces,
      connectingSlugs: connecting,
      connectErrors,
      syncModes,
      autoSyncOn,
    }),
  );

  async function handleConnect(slug: string) {
    if (connecting.includes(slug)) return;
    const { [slug]: _cleared, ...rest } = connectErrors;
    connectErrors = rest;
    connecting = [...connecting, slug];
    try {
      await invoke('connect_workspace_to_cloud', { slug });
      onrefresh?.();
    } catch (err) {
      console.error(`connect_workspace_to_cloud(${slug}) failed:`, err);
      connectErrors = { ...connectErrors, [slug]: String(err) };
    } finally {
      connecting = connecting.filter((entry) => entry !== slug);
    }
  }

  async function handleInviteAction(row: (typeof model.notConnected)[number]) {
    if (openingInvite) return;
    openingInvite = row.slug;
    inviteNotices = { ...inviteNotices, [row.slug]: '' };
    const config = await invoke<{ hqFolderPath?: string }>('get_config').catch(() => ({
      hqFolderPath: '',
    }));
    const prompt = [
      hqSkillMarkdownLink('accept', config.hqFolderPath),
      '',
      `Help me accept the pending HQ company invite for ${row.name}.`,
      `Company slug shown in HQ Sync: ${row.slug}.`,
      `Invite context: ${row.sub}.`,
      'The desktop app does not have the magic-link token. Ask me to paste the invite link or raw token, then complete the HQ accept flow.',
    ].join('\n');

    try {
      const url = buildClaudeCodeUrl({ folder: config.hqFolderPath ?? '', prompt });
      await invoke('open_claude_code_link', { url });
      inviteNotices = { ...inviteNotices, [row.slug]: 'Opened invite flow in Claude Code.' };
    } catch (err) {
      console.error(`open invite flow (${row.slug}) failed:`, err);
      try {
        await navigator.clipboard.writeText(prompt);
        inviteNotices = {
          ...inviteNotices,
          [row.slug]: 'Invite prompt copied. Paste it into Claude Code to continue.',
        };
      } catch {
        inviteNotices = { ...inviteNotices, [row.slug]: 'Could not open invite flow.' };
      }
    } finally {
      openingInvite = null;
    }
  }

  function handleOpen(slug: string, open: boolean) {
    if (open) onopencompany?.(slug);
  }
</script>

<section class="companies" aria-label="Companies">
  <header class="companies-header">
    <h1 class="companies-title">Companies</h1>
    <p class="companies-summary">{model.summary}</p>
    {#if workspaceError}
      <p class="companies-error">{workspaceError}</p>
    {/if}
  </header>

  {#if !ready}
    <div class="companies-skeleton" aria-busy="true">
      {#each [0, 1, 2] as row (row)}
        <span class="companies-skeleton-bar" style={`width: ${82 - row * 16}%`}></span>
      {/each}
    </div>
  {:else}
    <div class="companies-table" data-testid="companies-connected">
      <div class="companies-head" aria-hidden="true">
        <span class="companies-head-name">Connected</span>
        <span class="companies-lane">Members</span>
        <span class="companies-lane">Last change</span>
        <span class="companies-lane sync">Sync</span>
        <span class="companies-chevron-slot"></span>
      </div>
      {#if model.connected.length === 0}
        <p class="companies-empty">No companies connected yet. Run a sync to connect your workspaces.</p>
      {/if}
      {#snippet connectedCells(row: (typeof model.connected)[number])}
        <span class="companies-name-cell">
          <span class={`companies-dot ${row.tone}`} aria-hidden="true"></span>
          <span class="companies-name-copy">
            <span class="companies-name">{row.name}</span>
            <span class="companies-sub">{row.sub}</span>
          </span>
        </span>
        <span class="companies-lane">{row.members}</span>
        <span class="companies-lane">{row.lastChange}</span>
        <span class="companies-lane sync">
          {#if row.retry}
            <button
              type="button"
              class="companies-action"
              disabled={connecting.includes(row.slug)}
              onclick={() => void handleConnect(row.slug)}
            >
              Retry
            </button>
          {:else}
            {row.sync}
          {/if}
        </span>
        <span class="companies-chevron-slot" aria-hidden="true">{row.open ? '›' : ''}</span>
      {/snippet}
      {#each model.connected as row (row.slug)}
        {#if row.open}
          <button
            type="button"
            class="companies-row openable"
            onclick={() => handleOpen(row.slug, row.open)}
          >
            {@render connectedCells(row)}
          </button>
        {:else}
          <div class="companies-row">
            {@render connectedCells(row)}
          </div>
        {/if}
      {/each}
    </div>

    {#if model.notConnected.length > 0}
      <div class="companies-table" data-testid="companies-not-connected">
        <div class="companies-head" aria-hidden="true">
          <span class="companies-head-name">Not connected</span>
        </div>
        {#each model.notConnected as row (row.slug)}
          <div class="companies-row static">
            <span class="companies-name-cell">
              <span class="companies-name-copy">
                <span class="companies-name">{row.name}</span>
                <span class="companies-sub">{row.sub}</span>
                {#if row.note}
                  <span class="companies-note">Connect failed — {row.note}</span>
                {/if}
                {#if row.kind === 'invite' && inviteNotices[row.slug]}
                  <span class="companies-note">{inviteNotices[row.slug]}</span>
                {/if}
              </span>
            </span>
            <span class="companies-row-actions">
              {#each row.actions as action (action)}
                {#if action === 'open'}
                  <button
                    type="button"
                    class="companies-action"
                    onclick={() => handleOpen(row.slug, true)}
                  >
                    Open
                  </button>
                {:else if action === 'connect'}
                  <button
                    type="button"
                    class="companies-action primary"
                    disabled={connecting.includes(row.slug)}
                    onclick={() => void handleConnect(row.slug)}
                  >
                    {connecting.includes(row.slug) ? 'Connecting…' : 'Connect'}
                  </button>
                {:else if action === 'open-invite'}
                  <button
                    type="button"
                    class="companies-action primary"
                    disabled={openingInvite !== null}
                    onclick={() => void handleInviteAction(row)}
                  >
                    {openingInvite === row.slug ? 'Opening…' : 'Open invite'}
                  </button>
                {/if}
              {/each}
            </span>
          </div>
        {/each}
      </div>
    {/if}

    <p class="companies-footnote">
      Per-company sync rules, excluded paths, and member roles live inside each company →
      Settings.
    </p>
  {/if}
</section>

<style>
  .companies {
    container: companies / inline-size;
    display: grid;
    gap: 16px;
    align-content: start;
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .companies-header {
    display: grid;
    gap: 4px;
  }

  .companies-title {
    margin: 0;
    color: var(--v4-text-1);
    font-size: 14px;
    font-weight: 500;
    line-height: 1.3;
  }

  .companies-summary {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.4;
  }

  .companies-error {
    margin: 0;
    color: var(--v4-text-2);
    font-size: 11px;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  /* ── Table containers (inset surface per SPEC section 2) ───────────────── */
  .companies-table {
    display: grid;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    overflow: hidden;
  }

  .companies-head,
  .companies-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 90px 110px 130px 20px;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
  }

  .companies-row.static {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .companies-head {
    border-bottom: 1px solid var(--v4-hairline);
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .companies-row {
    border: none;
    border-bottom: 1px solid var(--v4-rowline);
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: default;
  }

  .companies-row:last-child {
    border-bottom: none;
  }

  button.companies-row.openable {
    cursor: pointer;
  }

  button.companies-row.openable:hover {
    background: var(--v4-active-row);
  }

  .companies-name-cell {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .companies-dot {
    flex: 0 0 6px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .companies-dot.ok {
    background: var(--v4-ok);
  }

  .companies-dot.idle {
    background: var(--v4-idle);
  }

  .companies-dot.warn {
    background: var(--v4-warn);
  }

  .companies-dot.error {
    background: var(--v4-error);
  }

  .companies-name-copy {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  .companies-name {
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .companies-sub {
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .companies-note {
    color: var(--v4-text-2);
    font-size: 11px;
    line-height: 1.4;
    overflow-wrap: anywhere;
    white-space: normal;
  }

  .companies-lane {
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .companies-head .companies-lane {
    color: inherit;
    font-size: inherit;
  }

  .companies-head-name {
    min-width: 0;
  }

  .companies-chevron-slot {
    color: var(--v4-text-3);
    font-size: 13px;
    text-align: right;
  }

  .companies-empty {
    margin: 0;
    padding: 14px 16px;
    color: var(--v4-text-3);
    font-size: 13px;
  }

  .companies-row-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 0 auto;
  }

  .companies-action {
    display: inline-block;
    padding: 5px 10px;
    border: 1px solid var(--v4-control-border);
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-1);
    font: inherit;
    font-size: 13px;
    font-weight: 400;
    line-height: 1;
    white-space: nowrap;
    cursor: pointer;
  }

  .companies-action.primary {
    border-color: transparent;
    background: var(--v4-control-bg);
  }

  .companies-action:hover:not(:disabled) {
    background: var(--v4-control-bg);
  }

  .companies-action:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .companies-footnote {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.4;
  }

  /* ── First-load skeleton ───────────────────────────────────────────────── */
  .companies-skeleton {
    display: grid;
    gap: 10px;
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
  }

  .companies-skeleton-bar {
    display: block;
    height: 10px;
    border-radius: 999px;
    background: var(--v4-control-faint);
    animation: companies-skeleton-pulse 1.2s ease-in-out infinite;
  }

  @keyframes companies-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }

    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .companies-skeleton-bar {
      animation: none;
    }
  }

  @container companies (max-width: 520px) {
    .companies-head {
      display: none;
    }

    .companies-row {
      grid-template-columns: minmax(0, 1fr) auto;
      align-items: start;
      gap: 8px 12px;
      padding: 12px 14px;
    }

    .companies-row.static {
      grid-template-columns: minmax(0, 1fr);
    }

    .companies-row > .companies-lane:not(.sync),
    .companies-chevron-slot {
      display: none;
    }

    .companies-lane.sync {
      justify-self: end;
      max-width: 96px;
      color: var(--v4-text-3);
      font-size: 11px;
      line-height: 1.3;
      text-align: right;
      white-space: normal;
    }

    .companies-name,
    .companies-sub {
      overflow: visible;
      text-overflow: initial;
      white-space: normal;
    }

    .companies-row-actions {
      flex-wrap: wrap;
      justify-content: flex-start;
      margin-top: 8px;
    }
  }

  @container companies (max-width: 340px) {
    .companies-row {
      grid-template-columns: minmax(0, 1fr);
    }

    .companies-lane.sync {
      justify-self: start;
      max-width: 100%;
      text-align: left;
    }
  }
</style>
