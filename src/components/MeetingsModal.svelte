<script lang="ts">
  /**
   * Meetings modal — backdrop + close affordances + Esc-to-dismiss. The
   * body (upcoming-meetings list, URL-input invite row) is filled in by
   * SYNC-3 + SYNC-4; this component just owns the shell + dismissal so
   * the icon (SYNC-2) has something to open.
   */
  interface Props {
    open: boolean;
    onclose: () => void;
  }
  let { open, onclose }: Props = $props();

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
    }
  }

  function onBackdropClick(e: MouseEvent) {
    // Close only when the click landed on the backdrop itself, not on
    // children — matches the ConflictModal interaction model.
    if (e.target === e.currentTarget) onclose();
  }
</script>

<svelte:window {onkeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="meetings-backdrop"
    onclick={onBackdropClick}
    role="dialog"
    aria-modal="true"
    aria-label="Upcoming meetings"
  >
    <div class="meetings-shell">
      <header class="meetings-header">
        <h2>Upcoming Meetings</h2>
        <button
          type="button"
          class="meetings-close"
          aria-label="Close meetings"
          onclick={onclose}
        >
          ×
        </button>
      </header>

      <section class="meetings-body">
        <!-- SYNC-3 + SYNC-4 will replace this placeholder with the URL
             invite input + upcoming-meetings list. -->
        <p class="meetings-placeholder">Loading…</p>
      </section>
    </div>
  </div>
{/if}

<style>
  .meetings-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    z-index: 50;
    padding: 24px 12px 12px;
    overflow-y: auto;
  }
  .meetings-shell {
    width: 100%;
    max-width: 380px;
    max-height: calc(100vh - 36px);
    display: flex;
    flex-direction: column;
    background: var(--popover-surface, #18181b);
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.08));
    border-radius: 12px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.45);
    color: var(--popover-primary-text, #f4f4f5);
    overflow: hidden;
  }
  .meetings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px 10px;
    border-bottom: 1px solid var(--popover-border, rgba(255, 255, 255, 0.06));
  }
  .meetings-header h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .meetings-close {
    width: 24px;
    height: 24px;
    border: 0;
    background: transparent;
    color: var(--popover-secondary-text, #a1a1aa);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    border-radius: 4px;
    padding: 0;
  }
  .meetings-close:hover {
    background: var(--popover-surface-hover, rgba(255, 255, 255, 0.06));
    color: var(--popover-primary-text, #f4f4f5);
  }
  .meetings-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 14px;
  }
  .meetings-placeholder {
    margin: 0;
    color: var(--popover-secondary-text, #a1a1aa);
    font-size: 12px;
    text-align: center;
  }
</style>
