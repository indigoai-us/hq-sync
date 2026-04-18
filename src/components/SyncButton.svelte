<script lang="ts">
  interface Props {
    syncState: 'idle' | 'syncing' | 'error' | 'conflict';
    progress?: { filesComplete: number; filesTotal: number } | null;
    onclick: () => void;
  }

  let { syncState, progress = null, onclick }: Props = $props();

  let label = $derived(
    syncState === 'syncing'
      ? 'Syncing...'
      : syncState === 'error'
        ? 'Retry Sync'
        : syncState === 'conflict'
          ? 'Sync Now'
          : 'Sync Now'
  );

  let disabled = $derived(syncState === 'syncing');

  let progressPct = $derived(
    progress && progress.filesTotal > 0
      ? Math.round((progress.filesComplete / progress.filesTotal) * 100)
      : 0
  );
</script>

<div class="sync-button-wrapper">
  <button
    class="sync-button"
    class:syncing={syncState === 'syncing'}
    class:error={syncState === 'error'}
    {disabled}
    {onclick}
  >
    {#if syncState === 'syncing'}
      <span class="spinner"></span>
    {:else if syncState === 'error'}
      <!-- Retry / alert-circle icon -->
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M1.5 8a6.5 6.5 0 0 1 11.48-4.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M14.5 8A6.5 6.5 0 0 1 3.02 12.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M11 1.5v2.5h2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M5 12h-2.5v2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    {:else}
      <!-- Refresh / sync icon -->
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M1.5 8a6.5 6.5 0 0 1 11.48-4.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M14.5 8A6.5 6.5 0 0 1 3.02 12.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M11 1.5v2.5h2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M5 12h-2.5v2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    {/if}
    {label}
  </button>

  {#if syncState === 'syncing' && progress && progress.filesTotal > 0}
    <div class="progress-bar">
      <div class="progress-fill" style="width: {progressPct}%"></div>
    </div>
    <span class="progress-text">{progress.filesComplete} / {progress.filesTotal} files</span>
  {/if}
</div>

<style>
  .sync-button-wrapper {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .sync-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.625rem 1.25rem;
    font-size: 0.875rem;
    font-weight: 600;
    font-family: inherit;
    color: #ffffff;
    background-color: var(--popover-primary, #6366f1);
    border: none;
    border-radius: 8px;
    cursor: pointer;
    transition: background-color 0.15s ease, opacity 0.15s ease;
  }

  .sync-button:hover:not(:disabled) {
    background-color: var(--popover-primary-hover, #4f46e5);
  }

  .sync-button:active:not(:disabled) {
    background-color: var(--popover-primary-active, #4338ca);
  }

  .sync-button:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .sync-button.syncing {
    background-color: var(--popover-primary, #6366f1);
    opacity: 0.85;
  }

  .sync-button.error {
    background-color: var(--popover-danger, #ef4444);
  }

  .sync-button.error:hover:not(:disabled) {
    background-color: var(--popover-danger, #dc2626);
    filter: brightness(0.9);
  }

  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #ffffff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .progress-bar {
    width: 100%;
    height: 3px;
    border-radius: 2px;
    background: var(--popover-progress-track, rgba(99, 102, 241, 0.15));
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    border-radius: 2px;
    background: var(--popover-progress-fill, #6366f1);
    transition: width 0.3s ease;
  }

  .progress-text {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    text-align: center;
  }
</style>
