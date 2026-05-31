<script lang="ts">
  /**
   * Meeting-invite icon in the Popover header.
   *
   * Calendar/dot glyph; click opens the standalone `meetings-window` (via
   * `invoke('open_meetings_window')`). Parent gates rendering on the
   * `meetings_feature_enabled` check so this component is only mounted for
   * users on the @getindigo.ai allowlist.
   *
   * Three visual states drive the glyph colour:
   *   - `recording` (red): at least one ActiveMeeting in the `recording`
   *     state. Highest priority — recording is the most actionable state.
   *   - `detected` (yellow): at least one ActiveMeeting awaiting a record
   *     decision (state `detected` or `error`) and none recording. The
   *     in-popover Detected/Record row was removed in favour of this
   *     discreet header affordance — clicking it opens MeetingsWindow,
   *     where the user can act on the detection.
   *   - `idle` (default near-white): no active meetings.
   *
   * Priority is intentional: a parallel "recording" + "detected" pair
   * (e.g. one Zoom recording, a new Slack call detected) reads as
   * recording first so the user doesn't miss the active red dot. The
   * MeetingsWindow shows both rows for the granular picture.
   */
  type ActiveState = 'detected' | 'starting' | 'recording' | 'stopping' | 'error';
  interface Props {
    onclick: () => void;
    /** Optional badge — e.g. number of upcoming meetings. Future use. */
    count?: number;
    /**
     * Lightweight summary of the parent's `activeMeetings` array. Only
     * the per-row `state` is needed to colour the icon; passing the
     * derived bools (instead of the full array) avoids leaking the
     * window-id / company-uid shape across the component boundary.
     */
    detected?: boolean;
    recording?: boolean;
  }
  let {
    onclick,
    count,
    detected = false,
    recording = false,
  }: Props = $props();

  // Recording wins over detected so the icon reads as "live capture in
  // progress" whenever any meeting is being recorded.
  const visualState = $derived<'idle' | 'detected' | 'recording'>(
    recording ? 'recording' : detected ? 'detected' : 'idle',
  );
</script>

<button
  type="button"
  class="meeting-icon-btn"
  data-state={visualState}
  {onclick}
  title={
    visualState === 'recording'
      ? 'Recording in progress — click to manage'
      : visualState === 'detected'
        ? 'Meeting detected — click to record'
        : 'Upcoming meetings'
  }
  aria-label={
    visualState === 'recording'
      ? 'Open meetings — recording in progress'
      : visualState === 'detected'
        ? 'Open meetings — meeting detected'
        : 'Open meetings'
  }
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
    transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
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
  /* Detected — yellow/amber. A meeting is in progress and the user has
     not yet pressed Record. Matches the macOS HUD amber for "attention
     needed but non-urgent". */
  .meeting-icon-btn[data-state='detected'] {
    color: #facc15;
    background: rgba(250, 204, 21, 0.16);
    border-color: rgba(250, 204, 21, 0.42);
  }
  .meeting-icon-btn[data-state='detected']:hover {
    background: rgba(250, 204, 21, 0.24);
    border-color: rgba(250, 204, 21, 0.62);
  }
  /* Recording — red. A SDK upload is actively in flight (the "live
     indicator" on the calendar dot). Same palette as the popover Stop
     button so the user reads it as the same state. */
  .meeting-icon-btn[data-state='recording'] {
    color: #f87171;
    background: rgba(248, 113, 113, 0.16);
    border-color: rgba(248, 113, 113, 0.45);
  }
  .meeting-icon-btn[data-state='recording']:hover {
    background: rgba(248, 113, 113, 0.24);
    border-color: rgba(248, 113, 113, 0.65);
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
