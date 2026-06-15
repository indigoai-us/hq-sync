<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import * as Sentry from '@sentry/svelte';
  import type { Workspace } from '../../lib/workspaces';
  import { buildClaudeCodeUrl } from '../../lib/claude-code-link';
  import { hqSkillMarkdownLink } from '../../lib/hq-skill-link';
  import {
    getCompaniesPageModel,
    type CompanySyncMode,
  } from '../v4/companies-model';
  import SyncModeControl from '../components/SyncModeControl.svelte';
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
    /**
     * Whether the last workspace fetch reached the vault. When false, every
     * cloud write (Connect, Retry, sync-mode toggle) is disabled and one quiet
     * notice replaces a scatter of per-row errors. Defaults true so a warm
     * cache never gates.
     */
    cloudReachable?: boolean;
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
    cloudReachable = true,
    onopencompany,
    onrefresh,
  }: Props = $props();

  // Slugs with an in-flight Connect — rendered as amber provisioning rows.
  let connecting = $state<string[]>([]);
  // slug → message from a failed Connect attempt.
  let connectErrors = $state<Record<string, string>>({});
  // slug → resolved per-membership sync mode (get_sync_mode).
  let syncModes = $state<Record<string, CompanySyncMode>>({});
  // slug → a set_sync_mode write is in flight (locks that row's control).
  let savingModes = $state<Record<string, boolean>>({});
  // slug → message from a failed sync-mode write (inline, transient).
  let modeErrors = $state<Record<string, string>>({});
  // The slug awaiting the All→Shared confirm (a switch that removes the
  // non-shared tree from this Mac on the next sync), or null when none.
  let confirmShared = $state<string | null>(null);
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

  // slug → team member count (get_company_activity stats.members), fetched
  // lazily per connected company row to fill the MEMBERS lane.
  let memberCounts = $state<Record<string, number>>({});
  const requestedMembers = new Set<string>();

  // One lazy get_company_activity round-trip per connected company row, purely
  // to resolve the MEMBERS lane. Best-effort: a failure leaves the lane at "—".
  // Mirrors the get_sync_mode guard so a re-render never refetches a slug.
  $effect(() => {
    for (const workspace of workspaces) {
      if (workspace.kind !== 'company') continue;
      if (workspace.state !== 'synced' && workspace.state !== 'cloud-only') continue;
      const slug = workspace.slug;
      if (requestedMembers.has(slug)) continue;
      requestedMembers.add(slug);
      void invoke<{ stats?: { members?: number } }>('get_company_activity', { slug })
        .then((activity) => {
          const count = activity?.stats?.members;
          if (typeof count === 'number') {
            memberCounts = { ...memberCounts, [slug]: count };
          }
        })
        .catch((err) => {
          console.warn(`get_company_activity(${slug}) members fetch failed:`, err);
        });
    }
  });

  const model = $derived(
    getCompaniesPageModel({
      workspaces,
      connectingSlugs: connecting,
      connectErrors,
      syncModes,
      memberCounts,
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

  // Sync mode is a LOCAL FOOTPRINT control, never access. Shared→All is purely
  // additive (the next sync downloads more), so it commits immediately;
  // All→Shared removes the non-shared tree from THIS Mac on the next sync — the
  // cloud copy stays and returns on All, and files changed-but-not-yet-synced
  // are never removed — so it routes through an explicit confirm first.
  function handleSelectMode(slug: string, next: 'all' | 'shared') {
    const current = syncModes[slug] ?? null;
    if (current === next || savingModes[slug]) return;
    if (next === 'shared') {
      confirmShared = slug;
      return;
    }
    void commitMode(slug, next);
  }

  function cancelConfirmShared() {
    confirmShared = null;
  }

  function confirmSwitchToShared() {
    const slug = confirmShared;
    confirmShared = null;
    if (slug) void commitMode(slug, 'shared');
  }

  async function commitMode(slug: string, next: 'all' | 'shared') {
    const prev = syncModes[slug] ?? null;
    if (prev === next) return;
    const { [slug]: _clearedErr, ...restErrors } = modeErrors;
    modeErrors = restErrors;
    savingModes = { ...savingModes, [slug]: true };
    // Optimistic flip — the row's control + lane label both re-derive from
    // syncModes, so the UI reflects the new footprint immediately.
    syncModes = { ...syncModes, [slug]: next };
    try {
      const cfg = await invoke<{ syncMode: CompanySyncMode }>('set_sync_mode', {
        companySlug: slug,
        mode: next,
      });
      syncModes = { ...syncModes, [slug]: cfg.syncMode };
    } catch (err) {
      // Revert the optimistic flip and surface a quiet, retryable note.
      syncModes = { ...syncModes, [slug]: prev };
      const msg = String(err);
      console.error(`set_sync_mode(${slug}, ${next}) failed:`, msg);
      Sentry.captureException(err instanceof Error ? err : new Error(msg), {
        tags: { slug, action: 'set-sync-mode', mode: next, source: 'desktop-alt' },
      });
      modeErrors = { ...modeErrors, [slug]: 'Couldn’t change sync mode — try again.' };
    } finally {
      savingModes = { ...savingModes, [slug]: false };
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
    {#if !cloudReachable}
      <p class="companies-offline" data-testid="companies-cloud-unreachable">
        Cloud unreachable — showing local folders only. Connecting and sync-mode changes are
        paused until it’s back.
      </p>
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
              disabled={connecting.includes(row.slug) || !cloudReachable}
              onclick={() => void handleConnect(row.slug)}
            >
              Retry
            </button>
          {:else if row.canToggleSyncMode && row.syncMode}
            <SyncModeControl
              mode={row.syncMode === 'shared' ? 'shared' : 'all'}
              saving={savingModes[row.slug] ?? false}
              disabled={!cloudReachable}
              onselect={(next) => handleSelectMode(row.slug, next)}
            />
          {:else}
            {row.sync}
          {/if}
        </span>
        <span class="companies-chevron-slot" aria-hidden="true">{row.open ? '›' : ''}</span>
      {/snippet}
      {#each model.connected as row (row.slug)}
        <div class="companies-row" class:openable={row.open}>
          {#if row.open}
            <!-- Stretched-link pattern (mirrors the popover WorkspaceList): a
                 real <button> overlays the whole row for navigation, while the
                 Sync-lane control sits above it via z-index. Avoids nesting the
                 toggle's buttons inside a row <button> (invalid + click steal). -->
            <button
              type="button"
              class="companies-rowlink"
              onclick={() => handleOpen(row.slug, true)}
              aria-label={`Open ${row.name}`}
            ></button>
          {/if}
          {@render connectedCells(row)}
        </div>
        {#if modeErrors[row.slug]}
          <p class="companies-mode-error" role="status">{modeErrors[row.slug]}</p>
        {/if}
        {#if confirmShared === row.slug}
          <div class="companies-confirm" data-testid="sync-mode-confirm">
            <p class="companies-confirm-text">
              Switch <strong>{row.name}</strong> to <strong>Shared</strong>? The next sync removes
              files that aren’t shared with you from this Mac. They stay in the cloud and return if
              you switch back to All — anything you’ve changed but not yet synced is never removed.
            </p>
            <div class="companies-confirm-actions">
              <button type="button" class="companies-action" onclick={cancelConfirmShared}>
                Cancel
              </button>
              <button
                type="button"
                class="companies-action primary"
                onclick={confirmSwitchToShared}
              >
                Switch to Shared
              </button>
            </div>
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
                    disabled={connecting.includes(row.slug) || !cloudReachable}
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
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.3;
  }

  .companies-summary {
    margin: 0;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.4;
  }

  .companies-error {
    margin: 0;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  /* Quiet, no red/amber — an unreachable cloud is a transient state, not an
     error the user caused (status colour stays reserved for the dots). */
  .companies-offline {
    margin: 0;
    color: var(--v4-text-3);
    font-size: var(--text-base);
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
    font-size: var(--text-base);
    font-weight: 400;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .companies-row {
    position: relative;
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

  .companies-row.openable {
    cursor: pointer;
  }

  .companies-row.openable:hover {
    background: var(--v4-active-row);
  }

  /* Stretched-link overlay: a real <button> (keyboard + a11y) pulled out of the
     grid flow so it doesn't consume a column; its ::after fills the row to make
     the whole row a click target. Sibling interactive controls (the Sync-lane
     toggle / Retry) sit above it via z-index — no nested buttons, no
     stopPropagation. Mirrors the popover WorkspaceList .row-link. */
  .companies-rowlink {
    position: absolute;
    inset: 0;
    z-index: 0;
    appearance: none;
    margin: 0;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .companies-rowlink:focus-visible {
    outline: 1px solid var(--v4-control-border);
    outline-offset: -2px;
    border-radius: 6px;
  }

  /* Lift the Sync lane above the stretched-link overlay so the toggle / Retry
     receive their own clicks; the rest of the row stays whole-row-clickable. */
  .companies-row .companies-lane.sync {
    position: relative;
    z-index: 1;
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
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .companies-sub {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .companies-note {
    color: var(--v4-text-2);
    font-size: var(--text-base);
    line-height: 1.4;
    overflow-wrap: anywhere;
    white-space: normal;
  }

  .companies-lane {
    color: var(--v4-text-2);
    font-size: var(--text-base);
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
    font-size: var(--text-base);
    text-align: right;
  }

  .companies-empty {
    margin: 0;
    padding: 14px 16px;
    color: var(--v4-text-3);
    font-size: var(--text-base);
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
    font-size: var(--text-base);
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

  /* All→Shared confirm — a full-width strip beneath the row (the Sync lane is
     too narrow to host it). Quiet treatment: this is reversible, not an error,
     so no red/amber — just a clear explanation + two actions. */
  .companies-confirm {
    display: grid;
    gap: 10px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--v4-rowline);
    background: var(--v4-control-faint);
  }

  .companies-confirm:last-child {
    border-bottom: none;
  }

  .companies-confirm-text {
    margin: 0;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .companies-confirm-text strong {
    color: var(--v4-text-1);
    font-weight: 500;
  }

  .companies-confirm-actions {
    display: flex;
    gap: 8px;
  }

  /* Failed-write note — transient, quiet, retryable (the toggle stays live). */
  .companies-mode-error {
    margin: 0;
    padding: 8px 16px;
    border-bottom: 1px solid var(--v4-rowline);
    color: var(--v4-text-2);
    font-size: var(--text-base);
    line-height: 1.4;
  }

  .companies-mode-error:last-child {
    border-bottom: none;
  }

  .companies-footnote {
    margin: 0;
    color: var(--v4-text-3);
    font-size: var(--text-base);
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
      font-size: var(--text-base);
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
