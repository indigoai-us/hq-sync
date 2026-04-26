<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { Workspace } from '../lib/workspaces';

  interface Props {
    workspaces: Workspace[];
    cloudReachable: boolean;
    cloudError?: string | null;
    /** Top-level manifest parse error. Non-null = soft warning notice
     *  rendered above the list; workspaces fell back to folder enumeration. */
    manifestError?: string | null;
    /** Called after a successful Connect so the parent re-fetches workspaces. */
    onrefresh?: () => void;
  }

  let {
    workspaces,
    cloudReachable,
    cloudError = null,
    manifestError = null,
    onrefresh,
  }: Props = $props();

  // Per-row connect state. Keys are slugs; absent = idle, true = in flight,
  // string = error message from the last attempt. Reset on next click.
  let connectState = $state<Record<string, true | string>>({});

  function badgeLabel(state: Workspace['state']): string {
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
      connectState = { ...connectState, [slug]: msg };
    }
  }
</script>

<div class="workspace-list-wrapper">
  {#if manifestError}
    <!-- Manifest unreadable — workspaces fell back to dir enumeration. Surface
         the parser error so the user can fix or report it. Distinct from the
         cloud-warning below (which is about reachability). -->
    <div class="cloud-warning manifest-warning" title={manifestError}>
      <span class="cloud-warning-dot manifest-warning-dot" aria-hidden="true"></span>
      <span class="cloud-warning-text">
        companies/manifest.yaml couldn't be read — showing folder list instead
      </span>
    </div>
  {/if}

  {#if !cloudReachable}
    <!-- Soft notice: cloud unreachable. We still rendered local data, so this
         is a heads-up rather than a blocker. -->
    <div class="cloud-warning" title={cloudError ?? ''}>
      <span class="cloud-warning-dot" aria-hidden="true"></span>
      <span class="cloud-warning-text">Cloud unreachable — showing local folders only</span>
    </div>
  {/if}

  <ul class="workspace-list">
    {#each workspaces as w (w.slug)}
      <li
        class="workspace-row"
        class:local-only={w.state === 'local-only'}
        class:broken={w.state === 'broken'}
      >
        <div class="row-main">
          <div class="row-name-line">
            <span class="row-name" title={w.displayName}>{w.displayName}</span>
            {#if w.slug !== w.displayName.toLowerCase().replace(/\s+/g, '-')}
              <span class="row-slug">{w.slug}</span>
            {/if}
          </div>
          {#if w.state === 'broken'}
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
          {:else if w.lastSyncedAt}
            <span class="row-meta">Last sync · {formatLastSynced(w.lastSyncedAt)}</span>
          {:else if w.state === 'cloud-only'}
            <span class="row-meta">Not yet on this machine</span>
          {:else if w.state === 'local-only'}
            {#if typeof connectState[w.slug] === 'string'}
              <span class="row-meta row-meta-error" title={connectState[w.slug] as string}>
                Connect failed — click to retry
              </span>
            {:else}
              <span class="row-meta">Not connected to cloud</span>
            {/if}
          {:else if w.state === 'personal' && !w.cloudUid}
            <span class="row-meta">Cloud unreachable</span>
          {/if}
        </div>

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

        <span
          class="row-badge"
          class:badge-personal={w.state === 'personal'}
          class:badge-synced={w.state === 'synced'}
          class:badge-cloud={w.state === 'cloud-only'}
          class:badge-local={w.state === 'local-only'}
          class:badge-broken={w.state === 'broken'}
          title={badgeTooltip(w)}
        >
          {badgeLabel(w.state)}
        </span>
      </li>
    {/each}
  </ul>
</div>

<style>
  .workspace-list-wrapper {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
  }

  .cloud-warning {
    display: flex;
    align-items: center;
    gap: 0.4375rem;
    padding: 0.4375rem 0.625rem;
    border-radius: 6px;
    background: rgba(245, 158, 11, 0.08);
    border: 1px solid rgba(245, 158, 11, 0.22);
  }

  /* Manifest-error variant — red instead of amber to distinguish "broken
     local file" from "transient connectivity". */
  .manifest-warning {
    background: rgba(239, 68, 68, 0.08);
    border-color: rgba(239, 68, 68, 0.22);
  }

  .cloud-warning-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #f59e0b;
    flex-shrink: 0;
  }

  .manifest-warning-dot {
    background: #ef4444;
  }

  .cloud-warning-text {
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
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4375rem 0.5rem;
    border-radius: 6px;
    transition: background-color 0.1s ease;
  }

  .workspace-row:hover {
    background: rgba(255, 255, 255, 0.025);
  }

  .workspace-row.local-only {
    /* Local-only rows are slightly muted — they need attention but aren't broken. */
    opacity: 0.92;
  }

  .workspace-row.broken {
    /* Broken rows: faint red wash so the eye lands on them in the list. */
    background: rgba(239, 68, 68, 0.04);
  }

  .workspace-row.broken:hover {
    background: rgba(239, 68, 68, 0.08);
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
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.625rem;
    color: var(--popover-text-muted, #a0a0b0);
    flex-shrink: 0;
  }

  .row-meta {
    font-size: 0.625rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.3;
  }

  .row-meta-error {
    color: #ef4444;
  }

  .row-action {
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

  /* Broken-state Connect button: red palette to match the row warning. */
  .row-action-broken {
    background: rgba(239, 68, 68, 0.10);
    color: #fca5a5;
    border-color: rgba(239, 68, 68, 0.32);
  }

  .row-action-broken:hover:not(:disabled) {
    background: rgba(239, 68, 68, 0.18);
    color: #fecaca;
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
    flex-shrink: 0;
    padding: 0.125rem 0.4375rem;
    border-radius: 999px;
    font-size: 0.5625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    line-height: 1.4;
    border: 1px solid transparent;
    white-space: nowrap;
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

  .badge-local {
    background: rgba(245, 158, 11, 0.10);
    color: #fbbf24;
    border-color: rgba(245, 158, 11, 0.28);
  }

  .badge-broken {
    background: rgba(239, 68, 68, 0.12);
    color: #fca5a5;
    border-color: rgba(239, 68, 68, 0.36);
  }
</style>
