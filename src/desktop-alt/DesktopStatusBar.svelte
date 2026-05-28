<script lang="ts">
  import type { SyncProgress, SyncState } from './lib/sync-model';

  interface Props {
    version: string;
    state?: SyncState;
    progress?: SyncProgress | null;
  }

  let { version, state = 'idle', progress = null }: Props = $props();

  const statusLabel = $derived(
    state === 'syncing'
      ? `Syncing ${progress?.company ?? 'workspace'}`
      : state === 'error' || state === 'auth-error' || state === 'conflict'
        ? 'Sync needs attention'
        : 'Ready',
  );
</script>

<footer class="desktop-status-bar" aria-label="Status">
  <span>{statusLabel}</span>
  <span>v{version}</span>
</footer>
