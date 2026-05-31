<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-shell';
  import * as Sentry from '@sentry/svelte';
  import type { Workspace } from '../lib/workspaces';
  import { parseLocalEnvFailure } from '../lib/copy-prompts';
  import CopyPromptButton from './CopyPromptButton.svelte';
  import OpenInClaudeCodeButton from './OpenInClaudeCodeButton.svelte';
  import SyncModeToggle from './SyncModeToggle.svelte';

  interface Props {
    workspaces: Workspace[];
    cloudReachable: boolean;
    cloudError?: string | null;
    /** Top-level manifest parse error. Non-null = soft warning notice
     *  rendered above the list; workspaces fell back to folder enumeration. */
    manifestError?: string | null;
    /** Absolute path to the HQ root folder on this machine. Passed through
     *  from `Popover` (`config.hqFolderPath`) so the "Fix in Claude Code"
     *  button can launch a Claude session in the right cwd. Optional — when
     *  empty (config not yet loaded) the button is suppressed gracefully. */
    hqFolderPath?: string;
    /** Called after a successful Connect so the parent re-fetches workspaces. */
    onrefresh?: () => void;
  }

  let {
    workspaces,
    cloudReachable,
    cloudError = null,
    manifestError = null,
    hqFolderPath = '',
    onrefresh,
  }: Props = $props();

  // Sort so cloud-backed rows (synced + cloud-only) float above local-only /
  // broken ones. Personal stays first. Stable within each group so the
  // backend-supplied ordering (e.g. alphabetical) is preserved.
  const stateOrder: Record<Workspace['state'], number> = {
    personal: 0,
    synced: 1,
    'cloud-only': 2,
    broken: 3,
    'local-only': 4,
  };
  const sortedWorkspaces = $derived(
    [...workspaces].sort((a, b) => stateOrder[a.state] - stateOrder[b.state]),
  );

  // Per-row connect state. Keys are slugs; absent = idle, true = in flight,
  // string = error message from the last attempt. Reset on next click.
  let connectState = $state<Record<string, true | string>>({});

  /**
   * Short, human-readable label per local-env failure kind. Kept in this
   * component (not `copy-prompts.ts`) because it's UI copy, not prompt
   * copy. Update both when adding a new `LocalEnvKind`.
   */
  function localEnvLabel(kind: string): string {
    switch (kind) {
      case 'npm-cache-permission':
        return 'npm cache locked (root-owned)';
      case 'disk-full':
        return 'Disk full';
      case 'npm-registry-unreachable':
        return 'npm registry unreachable';
      case 'npm-registry-timeout':
        return 'npm registry timed out';
      default:
        return 'Local environment failure';
    }
  }

  function badgeAriaLabel(state: Workspace['state']): string {
    switch (state) {
      case 'personal':   return 'Personal';
      case 'synced':     return 'Synced';
      case 'cloud-only': return 'Cloud only';
      case 'local-only': return 'Local only';
      case 'broken':     return 'Broken';
    }
  }

  function badgeTooltip(w: Workspace): string {
    switch (w.state) {
      case 'personal':
        return w.cloudUid
          ? 'Your personal vault — always synced'
          : 'Personal vault (cloud unreachable; will sync when reconnected)';
      case 'synced':
        return `Cloud + local both present${w.lastSyncedAt ? ` · last sync ${w.lastSyncedAt}` : ''}`;
      case 'cloud-only':
        return 'In your cloud vault but not on this machine yet — Sync Now will download it';
      case 'local-only':
        return 'Local folder exists but no matching cloud vault — click the cloud icon to connect';
      case 'broken':
        return w.brokenReason
          ? `Manifest is out of sync with cloud — click Connect to reconcile.\n${w.brokenReason}`
          : 'Manifest is out of sync with cloud — click Connect to reconcile';
    }
  }

  function formatLastSynced(iso: string | null): string {
    if (!iso) return '';
    const d = new Date(iso);
    if (isNaN(d.getTime())) return '';
    const diffMs = Date.now() - d.getTime();
    const diffMin = Math.floor(diffMs / 60000);
    if (diffMin < 1) return 'just now';
    if (diffMin < 60) return `${diffMin}m ago`;
    const diffHr = Math.floor(diffMin / 60);
    if (diffHr < 24) return `${diffHr}h ago`;
    const diffDay = Math.floor(diffHr / 24);
    if (diffDay < 30) return `${diffDay}d ago`;
    return d.toLocaleDateString();
  }

  function isCompanyClickable(w: Workspace): boolean {
    return w.kind === 'company' && (w.state === 'synced' || w.state === 'cloud-only');
  }

  // Show the sync-mode toggle only for cloud-backed company rows. `cloud-only`
  // is included on purpose: a user can pre-set `shared` before the first
  // download so a never-synced company never pulls its full tree. The personal
  // vault has no membership sync-config, so it never gets the toggle.
  function showSyncMode(w: Workspace): boolean {
    return w.kind === 'company' && (w.state === 'synced' || w.state === 'cloud-only');
  }

  async function handleOpenCompany(w: Workspace) {
    if (!isCompanyClickable(w)) return;
    try {
      await open(`https://hq.getindigo.ai/companies/${w.slug}`);
    } catch (err) {
      console.error('Failed to open company URL:', err);
    }
  }

  async function handleConnect(slug: string) {
    // Block double-clicks while in flight.
    if (connectState[slug] === true) return;
    connectState = { ...connectState, [slug]: true };
    try {
      await invoke('connect_workspace_to_cloud', { slug });
      // Drop the in-flight marker before refresh so the badge transition is clean.
      const { [slug]: _done, ...rest } = connectState;
      connectState = rest;
      onrefresh?.();
    } catch (err) {
      const msg = String(err);
      console.error('connect_workspace_to_cloud failed:', msg);
      // Belt-and-suspenders capture: the backend already reports CLI failures
      // via run_cli_provision::report_provision_error and validation failures
      // via workspaces::capture_connect_error, but capturing here too means
      // any frontend-only failure mode (Tauri invoke serialization, IPC
      // disconnect, plugin error) still reaches Sentry. Tagged distinctly so
      // we can filter without losing the backend events.
      Sentry.captureException(err instanceof Error ? err : new Error(msg), {
        tags: { slug, action: 'connect', source: 'frontend' },
        extra: { msg },
      });
      connectState = { ...connectState, [slug]: msg };
    }
  }
</script>

<div class="workspace-list-wrapper">
  {#if manifestError}
    <!-- Manifest unreadable — workspaces fell back to dir enumeration. Surface
         the parser error so the user can fix or report it. Friendly notice
         treatment (no red, no amber); the Copy-prompt button hands the error
         to an agent that can patch the YAML. -->
    <div class="cloud-warning" title={manifestError}>
      <span class="cloud-warning-text">
        companies/manifest.yaml couldn't be read — showing folder list instead
      </span>
      <CopyPromptButton
        variant="compact"
        label="Copy fix-manifest prompt"
        issue={{ kind: 'manifest-error', payload: { error: manifestError } }}
      />
    </div>
  {/if}

  {#if !cloudReachable}
    <!-- Soft notice: cloud unreachable. We still rendered local data, so this
         is a heads-up rather than a blocker. -->
    <div class="cloud-warning" title={cloudError ?? ''}>
      <span class="cloud-warning-text">Cloud unreachable — showing local folders only</span>
      <CopyPromptButton
        variant="compact"
        label="Copy diagnose-cloud prompt"
        issue={{ kind: 'cloud-unreachable', payload: { error: cloudError ?? '' } }}
      />
    </div>
  {/if}

  <ul class="workspace-list">
    <!-- Composite key (kind:slug) is required because the Personal vault row
         AND a manifest-declared `personal` company entry can both legitimately
         carry slug="personal" — they're conceptually distinct (vault vs.
         company) and target different cloud buckets. Keying by slug alone
         caused Svelte 5 to silently drop the duplicate, hiding the manifest's
         personal company entry from the UI (v0.1.23 regression). -->
    {#each sortedWorkspaces as w (`${w.kind}:${w.slug}`)}
      <li
        class="workspace-row"
        class:local-only={w.state === 'local-only'}
        class:broken={w.state === 'broken'}
        class:clickable={isCompanyClickable(w)}
      >
        {#if isCompanyClickable(w)}
          <!-- Stretched-link pattern: a real <button> sits in the row-main
               flow, and its ::after pseudo-element overlays the entire
               .workspace-row to capture clicks anywhere on the row. The
               sibling Connect button + status badge use position:relative
               + z-index to stay above the overlay. No nested handlers, no
               stopPropagation needed. -->
          <button
            class="row-link"
            type="button"
            onclick={() => handleOpenCompany(w)}
            title={`Open ${w.displayName} in HQ`}
            aria-label={`Open ${w.displayName} in HQ`}
          ></button>
        {/if}
        <div class="row-main">
          <div class="row-name-line">
            <span class="row-name" title={w.displayName}>{w.displayName}</span>
            {#if w.slug !== w.displayName.toLowerCase().replace(/\s+/g, '-')}
              <span class="row-slug">{w.slug}</span>
            {/if}
            {#if w.lastSyncedAt && w.state !== 'broken'}
              <!-- Inline with the name, right-aligned (margin-left: auto in
                   CSS), hover-only via .row-meta-lastsync. Excluded for
                   broken rows so the row-meta-line below carries the
                   reconnect affordance without competing meta. -->
              <span class="row-meta-lastsync" title={`Last sync · ${formatLastSynced(w.lastSyncedAt)}`}>
                {formatLastSynced(w.lastSyncedAt)}
              </span>
            {/if}
          </div>
          {#if w.state === 'broken'}
            <span class="row-meta-line">
              <span
                class="row-meta row-meta-error"
                title={w.brokenReason ?? 'Manifest cloud_uid does not match cloud reality'}
              >
                {#if typeof connectState[w.slug] === 'string'}
                  Reconnect failed — click to retry
                {:else}
                  Manifest out of sync — click to reconnect
                {/if}
              </span>
              <CopyPromptButton
                variant="compact"
                label="Copy repair prompt"
                issue={{
                  kind: 'workspace-broken',
                  payload: {
                    slug: w.slug,
                    reason: w.brokenReason ?? '',
                  },
                }}
              />
            </span>
          {:else if w.state === 'cloud-only'}
            <span class="row-meta">Not yet on this machine</span>
          {:else if w.state === 'local-only' && typeof connectState[w.slug] === 'string'}
            {@const errMsg = connectState[w.slug] as string}
            {@const localEnv = parseLocalEnvFailure(errMsg)}
            <span class="row-meta-line">
              <span class="row-meta row-meta-error" title={errMsg}>
                {#if localEnv}
                  {localEnvLabel(localEnv.kind)} — click "Fix in Claude Code"
                {:else}
                  Connect failed — click to retry
                {/if}
              </span>
              {#if localEnv && hqFolderPath}
                <!-- Action-button row for local-environment failures: the
                     issue isn't a vault outage but a fixable user-laptop
                     problem (npm cache EACCES is the canonical case). Open
                     a prefilled Claude Code session and let the agent walk
                     the user through `chown` / disk / registry remediation. -->
                <OpenInClaudeCodeButton
                  variant="compact"
                  label="Fix in Claude Code"
                  folder={hqFolderPath}
                  issue={{
                    kind: 'local-env-failure',
                    payload: {
                      slug: w.slug,
                      kind: localEnv.kind,
                      detail: localEnv.detail,
                    },
                  }}
                />
              {/if}
            </span>
          {:else if w.state === 'personal' && !w.cloudUid}
            <span class="row-meta">Cloud unreachable</span>
          {/if}
        </div>

        <!-- Sync-mode toggle (Shared / All) — local download footprint, not
             access. Cloud-backed company rows only. Hidden until the row is
             hovered or keyboard-focused (focus-within keeps it reachable
             without a mouse); the tooltip on the toggle explains what it does. -->
        {#if showSyncMode(w)}
          <div class="sync-mode-slot">
            <SyncModeToggle slug={w.slug} {cloudReachable} />
          </div>
        {/if}

        <!-- Connect icon button — for local-only AND broken rows. The same
             command (connect_workspace_to_cloud) handles both: for local-only
             it provisions fresh; for broken it re-finds by slug and overwrites
             the manifest cloud_uid with the current truth. -->
        {#if w.state === 'local-only' || w.state === 'broken'}
          <button
            class="row-action"
            class:connecting={connectState[w.slug] === true}
            class:row-action-broken={w.state === 'broken'}
            disabled={connectState[w.slug] === true || !cloudReachable}
            onclick={() => handleConnect(w.slug)}
            title={
              !cloudReachable
                ? 'Cloud unreachable — try again later'
                : w.state === 'broken'
                  ? 'Reconnect to reconcile the manifest with the cloud'
                  : 'Connect this folder to a cloud vault'
            }
            aria-label={(w.state === 'broken' ? 'Reconnect ' : 'Connect ') + w.displayName + ' to cloud'}
          >
            {#if connectState[w.slug] === true}
              <span class="row-action-spinner" aria-hidden="true"></span>
            {:else}
              <!-- Cloud + plus icon -->
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
                <path d="M11.5 11.5h1a3 3 0 0 0 .3-5.98 4.5 4.5 0 0 0-8.85-.4A3 3 0 0 0 4 11.5h.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
                <path d="M8 8.5v5M5.5 11l2.5 2.5L10.5 11" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            {/if}
          </button>
        {/if}

        <!-- Hide the badge for local-only rows: the Connect button already
             communicates "needs to be connected", and the yellow laptop badge
             beside it was redundant noise. -->
        {#if w.state !== 'local-only'}
        <span
          class="row-badge"
          class:badge-personal={w.state === 'personal'}
          class:badge-synced={w.state === 'synced'}
          class:badge-cloud={w.state === 'cloud-only'}
          class:badge-broken={w.state === 'broken'}
          title={badgeTooltip(w)}
          aria-label={badgeAriaLabel(w.state)}
          role="img"
        >
          {#if w.state === 'personal'}
            <!-- person -->
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
              <circle cx="8" cy="5.5" r="2.6" stroke="currentColor" stroke-width="1.4" />
              <path d="M3 13.2c0-2.3 2.2-3.7 5-3.7s5 1.4 5 3.7" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
            </svg>
          {:else if w.state === 'synced'}
            <!-- check -->
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
              <path d="M3.5 8.5l3 3 6-6.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
          {:else if w.state === 'cloud-only'}
            <!-- cloud -->
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
              <path d="M11.5 12h1a3 3 0 0 0 .3-5.98 4.5 4.5 0 0 0-8.85-.4A3 3 0 0 0 4 12h7.5z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" />
            </svg>
          {:else if w.state === 'broken'}
            <!-- warning triangle -->
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
              <path d="M8 2.5L14 13H2L8 2.5z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" />
              <path d="M8 6.5v3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
              <circle cx="8" cy="11.3" r="0.6" fill="currentColor" />
            </svg>
          {/if}
        </span>
        {/if}
      </li>
    {/each}
  </ul>
</div>

<style>
  .workspace-list-wrapper {
    display: flex;
    flex-direction: column;
    /* Tightened 0.625rem → 0.5rem (v0.1.85) to match popover body gap. */
    gap: 0.5rem;
  }

  /* Soft notice strip — used for cloud-unreachable and manifest-parse-error.
     Both surfaces share one calm grey treatment; the surface tells you which
     by its copy + the Copy-prompt button it carries. No severity colour. */
  .cloud-warning {
    display: flex;
    align-items: center;
    gap: 0.4375rem;
    padding: 0.4375rem 0.625rem;
    border-radius: 6px;
    background: var(--popover-notice-bg, rgba(255, 255, 255, 0.05));
    border: 1px solid var(--popover-notice-border, rgba(255, 255, 255, 0.16));
  }

  .cloud-warning-text {
    flex: 1;
    min-width: 0;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.3;
  }

  .workspace-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }

  .workspace-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    /* Tightened from 0.4375rem (v0.1.85). Combined with the hover-only
       .row-meta-lastsync below, the steady-state row collapses to a single
       name line at ~24px tall so more workspaces fit without scrolling. */
    padding: 0.25rem 0.5rem;
    border-radius: 6px;
    transition: background-color 0.1s ease;
  }

  .workspace-row:hover {
    background: rgba(255, 255, 255, 0.025);
  }

  .workspace-row.clickable {
    cursor: pointer;
  }

  /* Stretched-link button: invisible, zero-size, but its ::after expands to
     fill the entire .workspace-row, making the whole row clickable while
     keeping a real <button> in the DOM (proper keyboard + a11y semantics).
     Sibling .row-action / .row-badge use z-index to stay above this overlay. */
  /* Pulled out of the flex flow so it doesn't consume a slot or trigger
     the row's `gap`, which would shift sibling content right on clickable
     rows and visually misalign clickable vs. non-clickable rows. */
  .row-link {
    position: absolute;
    inset: 0;
    appearance: none;
    background: none;
    border: 0;
    padding: 0;
    margin: 0;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .row-link::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
  }

  .row-link:focus-visible::after {
    outline: 1px solid var(--popover-highlight, rgba(255, 255, 255, 0.34));
    outline-offset: -1px;
  }

  .workspace-row.local-only {
    /* Local-only rows are slightly muted — they need attention but aren't broken. */
    opacity: 0.92;
  }

  /* Broken rows: same hover treatment as any other row. The visual hierarchy
     is carried by the "Manifest out of sync — click to reconnect" meta line
     and the Copy-prompt button, not by colour. */
  .workspace-row.broken {
    background: var(--popover-notice-bg, rgba(255, 255, 255, 0.05));
  }

  .workspace-row.broken:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
  }

  .row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.0625rem;
  }

  .row-name-line {
    display: flex;
    align-items: baseline;
    gap: 0.4375rem;
    min-width: 0;
  }

  .row-name {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--popover-text, #e0e0e0);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .row-slug {
    /* Sans-serif (inherit from body) + pill (v0.1.85). Was monospace bare
       text — felt out of place next to the rest of the UI which is all
       system sans. Pill background separates the slug from the displayName
       without leaning on a different font family. */
    font-family: inherit;
    font-size: 0.6875rem;
    font-weight: 500;
    line-height: 1;
    padding: 0.1875rem 0.4375rem;
    border-radius: 999px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    color: var(--popover-text-muted, #a0a0b0);
    flex-shrink: 0;
  }

  .row-meta {
    /* Same consolidation as row-slug (v0.1.85): 10px → 11px. */
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.3;
  }

  /* Last-sync time, sitting inline at the right edge of the name line
     (margin-left: auto pushes it). Hover-only so the steady-state row is
     just the name + slug pill — keeps the list dense, surfaces the
     timestamp on demand. State/info metas (cloud-only, broken,
     connect-failed) below the name line stay visible without this
     modifier — those carry an action or unrecoverable status that
     shouldn't require hover to discover. */
  .row-meta-lastsync {
    display: none;
    margin-left: auto;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .workspace-row:hover .row-meta-lastsync {
    display: inline;
  }

  /* Sync-mode toggle: revealed only on row hover / keyboard focus so it
     doesn't add visual noise to every row. Opacity (not display) so it can
     transition and so layout doesn't jump as it appears. Stays interactive
     only when shown — pointer-events follow visibility. */
  .sync-mode-slot {
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s ease;
  }

  .workspace-row:hover .sync-mode-slot,
  .workspace-row:focus-within .sync-mode-slot {
    opacity: 1;
    pointer-events: auto;
  }

  /* "Connect failed" / "Manifest out of sync" meta lines — same muted grey
     as any other row-meta; copy carries the meaning. */
  .row-meta-error {
    color: var(--popover-text-muted, #a0a0b0);
  }

  /* Row-meta line that mixes the message + Copy-prompt button for broken
     rows. Keeps both inline so the row stays single-line tall. */
  .row-meta-line {
    display: inline-flex;
    align-items: center;
    gap: 0.4375rem;
    min-width: 0;
  }

  .row-action {
    position: relative;
    z-index: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    background: rgba(56, 189, 248, 0.10);
    color: #7dd3fc;
    border: 1px solid rgba(56, 189, 248, 0.28);
    border-radius: 6px;
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease, opacity 0.1s ease;
    flex-shrink: 0;
  }

  .row-action:hover:not(:disabled) {
    background: rgba(56, 189, 248, 0.18);
    color: #bae6fd;
  }

  /* Broken-state Connect button: same secondary-button treatment as the
     local-only Connect. Visual hierarchy is carried by the row text +
     Copy-prompt affordance, not by colour. */
  .row-action-broken {
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border-color: var(--popover-border, rgba(255, 255, 255, 0.18));
  }

  .row-action-broken:hover:not(:disabled) {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text-heading, #ffffff);
  }

  .row-action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .row-action.connecting {
    opacity: 0.85;
    cursor: progress;
  }

  .row-action-spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 1.5px solid rgba(125, 211, 252, 0.3);
    border-top-color: #7dd3fc;
    border-radius: 50%;
    animation: row-spin 0.7s linear infinite;
  }

  @keyframes row-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .row-badge {
    position: relative;
    z-index: 1;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 6px;
    border: 1px solid transparent;
  }

  .badge-personal {
    background: rgba(99, 102, 241, 0.14);
    color: #a5a8ff;
    border-color: rgba(99, 102, 241, 0.32);
  }

  .badge-synced {
    background: rgba(34, 197, 94, 0.10);
    color: #86efac;
    border-color: rgba(34, 197, 94, 0.28);
  }

  .badge-cloud {
    background: rgba(56, 189, 248, 0.10);
    color: #7dd3fc;
    border-color: rgba(56, 189, 248, 0.28);
  }

  /* Broken badge: muted grey, same notice tone. The triangle icon + tooltip
     ("Manifest is out of sync with cloud") communicate status. */
  .badge-broken {
    background: var(--popover-notice-bg, rgba(255, 255, 255, 0.05));
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.52));
    border-color: var(--popover-notice-border, rgba(255, 255, 255, 0.16));
  }
</style>
