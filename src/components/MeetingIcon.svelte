<script lang="ts">
  /**
   * Meeting-invite icon in the Popover header.
   *
   * Calendar/dot glyph; click opens the standalone `meetings-window` (via
   * `invoke('open_meetings_window')`). Parent gates rendering on the
   * `meetings_feature_enabled` check (GA) so this component is mounted for
   * any signed-in user.
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
  /* Matches the .header-icon-button vocabulary in Popover.svelte (32px, 8px
     radius, popover tokens) so the header action cluster is one consistent
     control family. Spacing is owned by the parent `.header-actions` gap. */
  .meeting-icon-btn {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    /* 28px square, transparent at rest — matched to the popover header's
       `.header-icon-button` so the secondary icon group is one consistent
       control family. The `detected`/`recording` states below still carry the
       stronger surface/primary fills, so meaningful state stays prominent
       while the idle control recedes. */
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-sm, 8px);
    border: 1px solid transparent;
    background: transparent;
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.52));
    cursor: pointer;
    transition: background-color 120ms ease, border-color 120ms ease, color 120ms ease;
    padding: 0;
    -webkit-app-region: no-drag;
  }
  .meeting-icon-btn:hover {
    color: var(--popover-text-heading, #ffffff);
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }
  .meeting-icon-btn:focus-visible {
    outline: 2px solid var(--popover-highlight, rgba(255, 255, 255, 0.34));
    outline-offset: 2px;
    color: var(--popover-text-heading, #ffffff);
  }
  /* Detected — a meeting is up and not yet recorded. Monochrome "attention":
     brighter glyph on a stronger surface (no amber). The calendar's filled dot
     carries the "something's live" cue; the app uses no severity colour. */
  .meeting-icon-btn[data-state='detected'] {
    color: var(--popover-text-heading, #ffffff);
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }
  /* Recording — capture in flight. The strongest, most "active" treatment: the
     primary fill with an inverted glyph, mirroring the selected/primary
     language used across the popover. Still monochrome — no red. */
  .meeting-icon-btn[data-state='recording'] {
    color: var(--popover-primary-text, #111113);
    background: var(--popover-primary, #ffffff);
    border-color: var(--popover-primary, #ffffff);
  }
  .meeting-icon-btn[data-state='recording']:hover {
    background: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
    border-color: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
  }
  .meeting-icon-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    border-radius: 7px;
    background: var(--popover-primary, #ffffff);
    color: var(--popover-primary-text, #111113);
    font-size: 9px;
    font-weight: 600;
    line-height: 14px;
    text-align: center;
  }
</style>
