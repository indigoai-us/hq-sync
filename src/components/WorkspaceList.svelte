<script lang="ts">
  import { open as openExternal } from '@tauri-apps/plugin-shell';
  import type { Workspace } from '../lib/workspaces';

  interface Props {
    workspaces: Workspace[];
    cloudReachable: boolean;
    cloudError?: string | null;
  }

  let { workspaces, cloudReachable, cloudError = null }: Props = $props();

  // Onboarding URL — canonical domain. Do NOT use indigo-hq.com (retired).
  const ONBOARDING_URL = 'https://onboarding.getindigo.ai';

  function badgeLabel(state: Workspace['state']): string {
    switch (state) {
      case 'personal':   return 'Personal';
      case 'synced':     return 'Synced';
      case 'cloud-only': return 'Cloud only';
      case 'local-only': return 'Local only';
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
        return 'Local folder exists but no matching cloud vault — Connect to create one';
    }
  }

  async function openOnboarding(path = '') {
    try {
      await openExternal(ONBOARDING_URL + path);
    } catch (err) {
      console.error('Failed to open onboarding:', err);
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
</script>

<div class="workspace-list-wrapper">
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
      <li class="workspace-row" class:local-only={w.state === 'local-only'}>
        <div class="row-main">
          <div class="row-name-line">
            <span class="row-name" title={w.displayName}>{w.displayName}</span>
            {#if w.slug !== w.displayName.toLowerCase().replace(/\s+/g, '-')}
              <span class="row-slug">{w.slug}</span>
            {/if}
          </div>
          {#if w.lastSyncedAt}
            <span class="row-meta">Last sync · {formatLastSynced(w.lastSyncedAt)}</span>
          {:else if w.state === 'cloud-only'}
            <span class="row-meta">Not yet on this machine</span>
          {:else if w.state === 'local-only'}
            <span class="row-meta">Not connected to cloud</span>
          {:else if w.state === 'personal' && !w.cloudUid}
            <span class="row-meta">Cloud unreachable</span>
          {/if}
        </div>
        <span
          class="row-badge"
          class:badge-personal={w.state === 'personal'}
          class:badge-synced={w.state === 'synced'}
          class:badge-cloud={w.state === 'cloud-only'}
          class:badge-local={w.state === 'local-only'}
          title={badgeTooltip(w)}
        >
          {badgeLabel(w.state)}
        </span>
      </li>
    {/each}
  </ul>

  <!-- Affordances — always visible so the menubar is never a dead-end.
       These deep-link to onboarding rather than spawning sub-flows in the
       menubar itself; the popover is intentionally lightweight. -->
  <div class="affordances">
    <button class="affordance" onclick={() => openOnboarding('/setup/company')}>
      <span class="affordance-icon" aria-hidden="true">+</span>
      <span class="affordance-text">Create a company</span>
    </button>
    <button class="affordance affordance-secondary" onclick={() => openOnboarding('')}>
      <span class="affordance-icon" aria-hidden="true">↗</span>
      <span class="affordance-text">Join via invite</span>
    </button>
  </div>
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

  .cloud-warning-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #f59e0b;
    flex-shrink: 0;
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

  .affordances {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin-top: 0.25rem;
    padding-top: 0.5rem;
    border-top: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
  }

  .affordance {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4375rem 0.5rem;
    background: none;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
    font-size: 0.75rem;
    color: var(--popover-text, #e0e0e0);
    text-align: left;
    transition: background-color 0.1s ease;
  }

  .affordance:hover {
    background: rgba(99, 102, 241, 0.08);
  }

  .affordance-secondary {
    color: var(--popover-text-muted, #a0a0b0);
  }

  .affordance-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: 4px;
    background: rgba(99, 102, 241, 0.16);
    color: var(--popover-primary, #6366f1);
    font-weight: 700;
    font-size: 0.75rem;
    flex-shrink: 0;
  }

  .affordance-secondary .affordance-icon {
    background: rgba(255, 255, 255, 0.06);
    color: var(--popover-text-muted, #a0a0b0);
  }

  .affordance-text {
    flex: 1;
  }
</style>
