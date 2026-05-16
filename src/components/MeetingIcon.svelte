<script lang="ts">
  /**
   * Meeting-invite icon in the Popover header.
   *
   * Calendar/dot glyph; click opens the standalone `meetings-window` (via
   * `invoke('open_meetings_window')`). Parent gates rendering on the
   * `meetings_feature_enabled` check so this component is only mounted for
   * users on the @getindigo.ai allowlist.
   */
  interface Props {
    onclick: () => void;
    /** Optional badge — e.g. number of upcoming meetings. Future use. */
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
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <!-- Calendar with a small filled dot — reads as "agenda + live indicator". -->
    <rect
      x="1.5"
      y="2.5"
      width="13"
      height="12"
      rx="2"
      stroke="currentColor"
      stroke-width="1.6"
    />
    <path
      d="M4.5 1v3M11.5 1v3M1.5 6h13"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
    />
    <circle cx="11.5" cy="10.75" r="1.5" fill="currentColor" />
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
    width: 30px;
    height: 30px;
    border-radius: 7px;
    /* Stronger border + slightly brighter background than the prior version,
       so the glyph reads against the Liquid Glass header background. */
    border: 1px solid rgba(255, 255, 255, 0.20);
    background: rgba(255, 255, 255, 0.10);
    /* High-contrast glyph color — was inheriting the muted popover text;
       now uses a near-white so the calendar reads at a glance. */
    color: #f4f4f5;
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
    padding: 0;
    margin-right: 8px;
  }
  .meeting-icon-btn:hover {
    background: rgba(255, 255, 255, 0.18);
    border-color: rgba(255, 255, 255, 0.32);
  }
  .meeting-icon-btn:focus-visible {
    outline: 2px solid rgba(180, 180, 255, 0.7);
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
    background: #ff4d4f;
    color: white;
    font-size: 9px;
    font-weight: 600;
    line-height: 14px;
    text-align: center;
  }
</style>
