<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import * as Sentry from '@sentry/svelte';

  /**
   * Per-company sync-mode toggle (Phase D, HQ Pro selective-download).
   *
   * `all`    → a sync downloads the company's full tree.
   * `shared` → a sync downloads only granted/shared prefixes (+ pins).
   *
   * Mode is a LOCAL FOOTPRINT control, never access. The authoritative store
   * is server-side per-membership; we read it lazily on mount and write it via
   * `set_sync_mode`. `custom` is CLI-only and rendered read-only here.
   */

  interface MembershipSyncConfig {
    membershipId: string;
    syncMode: 'all' | 'shared' | 'custom';
    isDefault: boolean;
    customPaths?: string[] | null;
    updatedBy?: string | null;
  }

  interface Props {
    slug: string;
    /** Disable interaction when the vault is unreachable. */
    cloudReachable: boolean;
  }

  let { slug, cloudReachable }: Props = $props();

  // null = not yet loaded; otherwise the resolved mode.
  let mode = $state<'all' | 'shared' | 'custom' | null>(null);
  let saving = $state(false);
  let error = $state<string | null>(null);

  async function load() {
    error = null;
    try {
      const cfg = await invoke<MembershipSyncConfig>('get_sync_mode', {
        companySlug: slug,
      });
      mode = cfg.syncMode;
    } catch (err) {
      // Don't Sentry-spam read failures — a freshly-connected company with no
      // sync-config row, or a transient vault blip, both land here. Surface a
      // muted inline hint and let the user retry by reopening the popover.
      console.warn(`get_sync_mode(${slug}) failed:`, err);
      error = 'mode unavailable';
    }
  }

  // Lazily resolve on mount. One vault round-trip per company row; cheap and
  // only runs for cloud-backed rows (the parent gates rendering).
  $effect(() => {
    if (mode === null && error === null) {
      void load();
    }
  });

  async function setMode(next: 'all' | 'shared') {
    if (saving || mode === next) return;
    const prev = mode;
    saving = true;
    error = null;
    // Optimistic flip — revert on failure.
    mode = next;
    try {
      const cfg = await invoke<MembershipSyncConfig>('set_sync_mode', {
        companySlug: slug,
        mode: next,
      });
      mode = cfg.syncMode;
    } catch (err) {
      mode = prev;
      const msg = String(err);
      console.error(`set_sync_mode(${slug}, ${next}) failed:`, msg);
      Sentry.captureException(err instanceof Error ? err : new Error(msg), {
        tags: { slug, action: 'set-sync-mode', mode: next, source: 'frontend' },
      });
      error = 'save failed';
    } finally {
      saving = false;
    }
  }
</script>

{#if mode === 'custom'}
  <!-- Custom paths are CLI-managed (`hq sync mode custom --paths …`); the
       popover has no surface to edit a path list, so we render it read-only. -->
  <span class="sync-mode sync-mode-custom" title="Custom paths — managed via `hq sync mode custom`">
    custom
  </span>
{:else if mode !== null}
  <span
    class="sync-mode-toggle"
    class:saving
    title={
      cloudReachable
        ? 'Controls what this company downloads to THIS machine — not who can access it. Shared = only files shared with you; All = the whole company. Switching to Shared removes the rest from this machine on the next sync (they stay in the cloud and come back if you switch to All). Files you’ve changed but not yet synced are never removed.'
        : 'Cloud unreachable — sync mode can’t be changed right now'
    }
  >
    <button
      type="button"
      class="sync-mode-opt"
      class:active={mode === 'shared'}
      disabled={!cloudReachable || saving}
      onclick={() => setMode('shared')}
    >
      Shared
    </button>
    <button
      type="button"
      class="sync-mode-opt"
      class:active={mode === 'all'}
      disabled={!cloudReachable || saving}
      onclick={() => setMode('all')}
    >
      All
    </button>
  </span>
{:else if error}
  <span class="sync-mode sync-mode-error" title={error}>—</span>
{/if}

<style>
  .sync-mode-toggle {
    position: relative;
    z-index: 1;
    display: inline-flex;
    align-items: center;
    gap: 1px;
    padding: 1px;
    border-radius: 999px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.14));
    flex-shrink: 0;
  }

  .sync-mode-toggle.saving {
    opacity: 0.7;
    cursor: progress;
  }

  .sync-mode-opt {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--popover-text-muted, #a0a0b0);
    font: inherit;
    font-size: 0.625rem;
    font-weight: 600;
    line-height: 1;
    padding: 0.1875rem 0.4375rem;
    border-radius: 999px;
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease;
  }

  .sync-mode-opt:hover:not(:disabled):not(.active) {
    color: var(--popover-text, #e0e0e0);
  }

  .sync-mode-opt.active {
    background: rgba(56, 189, 248, 0.18);
    color: #bae6fd;
  }

  .sync-mode-opt:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }

  .sync-mode {
    position: relative;
    z-index: 1;
    flex-shrink: 0;
    font-size: 0.625rem;
    font-weight: 600;
    color: var(--popover-text-muted, #a0a0b0);
    padding: 0.1875rem 0.4375rem;
    border-radius: 999px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
  }

  .sync-mode-custom {
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.14));
  }

  .sync-mode-error {
    opacity: 0.6;
  }
</style>
