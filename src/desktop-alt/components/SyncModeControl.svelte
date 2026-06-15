<script lang="ts">
  /**
   * V4 desktop per-company sync-mode control — the quiet Shared / All segmented
   * toggle that lives in the Companies page Sync lane, bringing the classic
   * popover's per-company footprint control to the desktop window.
   *
   * Presentational only: it reflects the resolved `mode` and emits
   * `onselect(next)`. The parent (CompaniesPage) owns the get_sync_mode /
   * set_sync_mode round-trip, the optimistic flip + revert, and the All→Shared
   * confirm step — the Sync lane is too narrow to host a confirm inline, and
   * centralising the write next to the page's `syncModes` state keeps the lane
   * label and the toggle in lockstep.
   *
   * Deliberately NOT a reuse of the popover's SyncModeToggle.svelte: that one
   * hardcodes a cyan active state, which would violate the V4 rule that status
   * colour is carried by dots only. The active option here is a neutral fill.
   */
  interface Props {
    /** Resolved footprint mode. Only the binary values reach this control —
     *  `custom`/loading rows render the plain label instead (see the model's
     *  `canToggleSyncMode`). */
    mode: 'all' | 'shared';
    /** A write for this row is in flight — show progress + lock the control. */
    saving?: boolean;
    /** Vault unreachable: the control stays visible (so the current footprint
     *  reads) but is read-only until the cloud comes back. */
    disabled?: boolean;
    onselect: (next: 'all' | 'shared') => void;
  }

  let { mode, saving = false, disabled = false, onselect }: Props = $props();

  const title = $derived(
    disabled
      ? 'Cloud unreachable — sync mode can’t be changed right now'
      : 'Controls what this company downloads to THIS Mac — not who can access it. ' +
        'Shared = only files shared with you; All = the whole company.',
  );

  function choose(next: 'all' | 'shared') {
    if (saving || disabled || next === mode) return;
    onselect(next);
  }
</script>

<span class="syncmode" class:saving {title} data-testid="sync-mode-control">
  <button
    type="button"
    class="syncmode-opt"
    class:active={mode === 'shared'}
    disabled={disabled || saving}
    aria-pressed={mode === 'shared'}
    onclick={() => choose('shared')}
  >
    Shared
  </button>
  <button
    type="button"
    class="syncmode-opt"
    class:active={mode === 'all'}
    disabled={disabled || saving}
    aria-pressed={mode === 'all'}
    onclick={() => choose('all')}
  >
    All
  </button>
</span>

<style>
  .syncmode {
    display: inline-flex;
    align-items: center;
    gap: 1px;
    padding: 2px;
    border-radius: 999px;
    background: var(--v4-control-faint);
    border: 1px solid var(--v4-hairline);
    flex-shrink: 0;
  }

  .syncmode.saving {
    opacity: 0.7;
    cursor: progress;
  }

  .syncmode-opt {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--v4-text-3);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 400;
    line-height: 1;
    padding: 3px 9px;
    border-radius: 999px;
    cursor: pointer;
    transition:
      background-color 0.12s ease,
      color 0.12s ease;
  }

  .syncmode-opt:hover:not(:disabled):not(.active) {
    color: var(--v4-text-2);
  }

  .syncmode-opt.active {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  .syncmode-opt:disabled {
    cursor: default;
  }

  /* A disabled-but-inactive option dims; the active one keeps its fill so the
     current footprint stays legible while the cloud is unreachable. */
  .syncmode-opt:disabled:not(.active) {
    opacity: 0.55;
  }
</style>
