<script lang="ts">
  /**
   * Meeting-invite icon in the Popover header.
   *
   * Discreet calendar/video glyph; click opens the upcoming-meetings modal
   * (rendered by the parent based on its own state). The parent gates
   * rendering on `meetings_feature_enabled` so this component is only ever
   * mounted for users on the @getindigo.ai allowlist.
   */
  interface Props {
    onclick: () => void;
    /** Optional badge — e.g. number of upcoming meetings (SYNC-3+). */
    count?: number;
  }
  let { onclick, count }: Props = $props();
</script>

<button
  type="button"
  class="meeting-icon-btn"
  {onclick}
  title="Upcoming meetings"
  aria-label="Open meetings"
>
  <svg
    width="14"
    height="14"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <!-- Calendar outline with a small dot in the corner — reads as "agenda"
         + "live indicator", distinct from the sync/refresh glyph -->
    <rect
      x="1.75"
      y="2.75"
      width="12.5"
      height="11.5"
      rx="1.75"
      stroke="currentColor"
      stroke-width="1.5"
    />
    <path
      d="M5 1.5v2.5M11 1.5v2.5M1.75 6.25h12.5"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
    />
    <circle cx="11.5" cy="10.5" r="1.25" fill="currentColor" />
  </svg>
  {#if count !== undefined && count > 0}
    <span class="meeting-icon-badge">{count > 9 ? '9+' : count}</span>
  {/if}
</button>

<style>
  .meeting-icon-btn {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.08));
    background: var(--popover-surface-soft, rgba(255, 255, 255, 0.04));
    color: var(--popover-primary-text, #f4f4f5);
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
    padding: 0;
    margin-right: 6px;
  }
  .meeting-icon-btn:hover {
    background: var(--popover-surface-hover, rgba(255, 255, 255, 0.08));
    border-color: var(--popover-border-strong, rgba(255, 255, 255, 0.16));
  }
  .meeting-icon-btn:focus-visible {
    outline: 2px solid var(--popover-focus, rgba(180, 180, 255, 0.7));
    outline-offset: 1px;
  }
  .meeting-icon-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    border-radius: 7px;
    background: var(--popover-badge, #ff4d4f);
    color: white;
    font-size: 9px;
    font-weight: 600;
    line-height: 14px;
    text-align: center;
  }
</style>
