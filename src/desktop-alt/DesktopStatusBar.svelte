<script lang="ts">
  import type { SyncProgress, SyncState } from './lib/sync-model';

  interface Props {
    version: string;
    state?: SyncState;
    progress?: SyncProgress | null;
    filesProgressed?: number;
    totalFiles?: number;
    nextMeetingLabel?: string | null;
  }

  let {
    version,
    state = 'idle',
    progress = null,
    filesProgressed = 0,
    totalFiles = 0,
    nextMeetingLabel = null,
  }: Props = $props();

  const connectionTone = $derived.by(() => {
    if (state === 'syncing') return 'syncing';
    if (state === 'error' || state === 'auth-error') return 'error';
    if (state === 'conflict' || state === 'setup-needed') return 'conflict';
    return 'idle';
  });
  const syncPercent = $derived(
    totalFiles > 0 ? Math.min(100, Math.max(0, Math.round((filesProgressed / totalFiles) * 100))) : 0,
  );
  const syncLabel = $derived(
    state === 'syncing'
      ? `Syncing ${progress?.company ?? 'workspace'} · ${syncPercent}%`
      : state === 'error' || state === 'auth-error'
        ? 'Sync error'
        : state === 'conflict'
          ? 'Conflict'
          : 'Sync idle',
  );
  const sparkBars = [6, 11, 8, 14, 5, 9, 13, 7, 15, 10, 6, 12, 8, 14];
</script>

<footer class="desktop-status-bar" aria-label="Status">
  <div class="status-left">
    <span class={`connected-pill ${connectionTone}`}>
      <span class="status-dot" aria-hidden="true"></span>
      Connected
    </span>
    <span class="status-group" aria-label="Sync status">
      <span class="status-icon" aria-hidden="true">↻</span>
      <span>{syncLabel}</span>
    </span>
    {#if nextMeetingLabel}
      <span class="status-group" aria-label="Next meeting">
        <span class="status-icon" aria-hidden="true">□</span>
        <span>{nextMeetingLabel}</span>
      </span>
    {/if}
  </div>

  <div class="status-right">
    <span class="status-group net-group" aria-label="Network activity">
      <span>net</span>
      <span class="sparkbars" aria-hidden="true">
        {#each sparkBars as bar, index (`bar-${index}`)}
          <span style={`height: ${bar}px`}></span>
        {/each}
      </span>
    </span>
    <span class="status-group" aria-label="VPN status">
      <span class="status-icon" aria-hidden="true">◒</span>
      <span>indigo-vpn</span>
    </span>
    <span class="version">v{version}</span>
  </div>
</footer>

<style>
  .desktop-status-bar,
  .status-left,
  .status-right,
  .status-group,
  .connected-pill {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .desktop-status-bar {
    justify-content: space-between;
    gap: 16px;
  }

  .status-left,
  .status-right {
    gap: 14px;
  }

  .status-left {
    overflow: hidden;
  }

  .status-right {
    flex: 0 0 auto;
  }

  .status-group,
  .connected-pill {
    flex: 0 1 auto;
    gap: 6px;
    white-space: nowrap;
  }

  .status-group {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .connected-pill {
    flex: 0 0 auto;
    font-weight: 600;
  }

  .connected-pill.idle {
    color: #16a34a;
  }

  .connected-pill.syncing {
    color: #2563eb;
  }

  .connected-pill.error {
    color: #dc2626;
  }

  .connected-pill.conflict {
    color: #d97706;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: currentColor;
    box-shadow: 0 0 6px currentColor;
  }

  .status-icon {
    flex: 0 0 auto;
    color: #71717a;
    font-size: 12px;
    line-height: 1;
  }

  .sparkbars {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 16px;
  }

  .sparkbars span {
    display: block;
    width: 3px;
    background: #71717a;
    opacity: 0.78;
  }

  .version {
    font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  }

  @media (max-width: 760px) {
    .net-group,
    .status-right .status-group:nth-child(2) {
      display: none;
    }
  }
</style>
