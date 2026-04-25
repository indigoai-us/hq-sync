<script lang="ts">
  import type { ConflictFile } from '../stores/conflicts';

  interface Props {
    conflict: ConflictFile;
    onresolve: (path: string, strategy: 'keep-local' | 'keep-remote') => void;
    onopen: (path: string) => void;
  }

  let { conflict, onresolve, onopen }: Props = $props();

  let fileName = $derived(conflict.path.split('/').pop() ?? conflict.path);

  let localShort = $derived(conflict.localHash.slice(0, 7));
  let remoteShort = $derived(conflict.remoteHash.slice(0, 7));
</script>

<div
  class="conflict-row"
  class:resolved={conflict.status === 'resolved'}
  class:error={conflict.status === 'error'}
>
  <div class="row-header">
    <div class="file-info">
      {#if conflict.status === 'resolved'}
        <svg class="status-icon resolved-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.5" />
          <path d="M5.5 8.5l1.5 1.5 3.5-3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      {:else if conflict.status === 'error'}
        <svg class="status-icon error-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.5" />
          <path d="M10 6L6 10M6 6l4 4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        </svg>
      {:else}
        <svg class="status-icon warning-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M8 1.5L1 13.5h14L8 1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M8 6v3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          <circle cx="8" cy="11.5" r="0.75" fill="currentColor" />
        </svg>
      {/if}

      <span class="file-path" title={conflict.path}>
        {fileName}
      </span>

      {#if conflict.canAutoResolve}
        <span class="auto-badge">auto</span>
      {/if}
    </div>
  </div>

  <div class="hash-row">
    <span class="hash-label">local</span>
    <code class="hash-value">{localShort}</code>
    <span class="hash-separator">vs</span>
    <span class="hash-label">remote</span>
    <code class="hash-value">{remoteShort}</code>
  </div>

  {#if conflict.status === 'error' && conflict.error}
    <p class="error-message">{conflict.error}</p>
  {/if}

  {#if conflict.status === 'resolving'}
    <div class="resolving-state">
      <span class="spinner"></span>
      <span class="resolving-text">Resolving...</span>
    </div>
  {:else if conflict.status === 'resolved'}
    <div class="resolved-state">
      Resolved: {conflict.resolution === 'keep-local' ? 'kept local' : 'kept remote'}
    </div>
  {:else}
    <div class="actions">
      <button class="action-btn local-btn" aria-label="Keep local version of {fileName}" onclick={() => onresolve(conflict.path, 'keep-local')}>
        Keep Local
      </button>
      <button class="action-btn remote-btn" aria-label="Keep remote version of {fileName}" onclick={() => onresolve(conflict.path, 'keep-remote')}>
        Keep Remote
      </button>
      <button class="action-btn editor-btn" onclick={() => onopen(conflict.path)} title="Open in editor" aria-label="Open {fileName} in editor">
        <svg width="12" height="12" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M11.5 1.5l3 3-9 9H2.5v-3l9-9Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M9.5 3.5l3 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
    </div>
  {/if}
</div>

<style>
  .conflict-row {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    padding: 0.5rem 0.625rem;
    border-radius: 8px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    transition: border-color 0.15s ease;
  }

  .conflict-row.resolved {
    border-color: rgba(34, 197, 94, 0.2);
    background: rgba(34, 197, 94, 0.06);
  }

  .conflict-row.error {
    border-color: rgba(239, 68, 68, 0.2);
    background: rgba(239, 68, 68, 0.06);
  }

  .row-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.375rem;
  }

  .file-info {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    min-width: 0;
    flex: 1;
  }

  .status-icon {
    flex-shrink: 0;
  }

  .warning-icon {
    color: var(--popover-warning, #f59e0b);
  }

  .resolved-icon {
    color: #22c55e;
  }

  .error-icon {
    color: var(--popover-danger, #ef4444);
  }

  .file-path {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--popover-text, #e0e0e0);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .auto-badge {
    flex-shrink: 0;
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.1rem 0.35rem;
    border-radius: 4px;
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    color: var(--popover-text-heading, #ffffff);
  }

  .hash-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding-left: 1.625rem;
  }

  .hash-label {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .hash-value {
    font-size: 0.6875rem;
    font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
    color: var(--popover-text, #e0e0e0);
    background: rgba(255, 255, 255, 0.05);
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
  }

  .hash-separator {
    font-size: 0.625rem;
    color: var(--popover-text-muted, #a0a0b0);
    opacity: 0.6;
  }

  .error-message {
    font-size: 0.6875rem;
    color: var(--popover-danger, #ef4444);
    margin: 0;
    padding-left: 1.625rem;
  }

  .resolving-state {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding-left: 1.625rem;
  }

  .spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 1.5px solid var(--popover-progress-track, rgba(255, 255, 255, 0.14));
    border-top-color: var(--popover-progress-fill, #ffffff);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .resolving-text {
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .resolved-state {
    font-size: 0.75rem;
    color: #22c55e;
    padding-left: 1.625rem;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding-left: 1.625rem;
  }

  .action-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.25rem;
    padding: 0.25rem 0.5rem;
    font-size: 0.6875rem;
    font-weight: 500;
    font-family: inherit;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease;
  }

  .local-btn {
    color: var(--popover-primary-text, #111113);
    background: var(--popover-primary, #ffffff);
  }

  .local-btn:hover {
    background: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
  }

  .remote-btn {
    color: var(--popover-warning, #f59e0b);
    background: rgba(245, 158, 11, 0.12);
  }

  .remote-btn:hover {
    background: rgba(245, 158, 11, 0.22);
  }

  .editor-btn {
    color: var(--popover-text-muted, #a0a0b0);
    background: rgba(255, 255, 255, 0.05);
    padding: 0.25rem;
  }

  .editor-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: var(--popover-text, #e0e0e0);
  }

  @media (prefers-color-scheme: light) {
    .conflict-row {
      background: var(--popover-surface, rgba(255, 255, 255, 0.5));
      border-color: var(--popover-border, rgba(0, 0, 0, 0.12));
    }

    .conflict-row.resolved {
      background: rgba(34, 197, 94, 0.05);
      border-color: rgba(34, 197, 94, 0.15);
    }

    .conflict-row.error {
      background: rgba(239, 68, 68, 0.05);
      border-color: rgba(239, 68, 68, 0.15);
    }

    .hash-value {
      background: rgba(0, 0, 0, 0.04);
    }

    .editor-btn {
      background: rgba(0, 0, 0, 0.04);
    }

    .editor-btn:hover {
      background: rgba(0, 0, 0, 0.08);
    }
  }
</style>
