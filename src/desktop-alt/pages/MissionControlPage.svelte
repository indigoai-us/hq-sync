<script lang="ts">
  /**
   * Mission Control — global, cross-company fleet view of running Claude Code
   * and Codex agent sessions (local + outpost), observed best-effort with no
   * vendor auth (US-006).
   *
   * THIS STORY IS THE SHELL. It renders the page chrome only:
   *   1. Header — title + best-effort / poll-cadence subtitle.
   *   2. Summary strip — 4 inset stat tiles (RUNNING / AWAITING INPUT / IDLE /
   *      OUTPOST), per design.md "Page structure".
   *   3. Two columns — clearly-marked mount points for the Live Sessions panel
   *      (US-007) and the History timeline panel (US-008).
   *
   * The live + history panels are deliberately NOT built here — US-007 mounts
   * LiveSessionsPanel into `.mc-live-mount` and US-008 mounts
   * SessionHistoryPanel into `.mc-history-mount`. Until then those slots show a
   * design-spec'd empty placeholder so the destination is navigable and the
   * layout is fixed before the panels land.
   *
   * Built entirely on the V4 "Liquid Glass" tokens (src/desktop-alt/v4/tokens.css);
   * no new colors, fonts, or spacing primitives (design.md "Tokens used").
   */

  /**
   * Desktop polling cadence, in seconds. The sessions store (US-005) re-scans on
   * this interval and emits a typed event the panels subscribe to; surfaced in
   * the header subtitle so the operator knows liveness is sampled, not live.
   * Shown here as the shell's best-effort label until the store lands.
   */
  const POLL_CADENCE_SECONDS = 5;

  /**
   * Summary tiles, in design.md "Page structure" order. Values stay em-dash
   * placeholders in the shell — US-007 wires the live counts from the sessions
   * store. The dot tone is the status taxonomy color (tokens.css `--v4-*`).
   */
  const SUMMARY_TILES: ReadonlyArray<{
    id: string;
    label: string;
    tone: 'ok' | 'warn' | 'idle';
    hint: string;
  }> = [
    { id: 'running', label: 'RUNNING', tone: 'ok', hint: 'live now' },
    { id: 'awaiting', label: 'AWAITING INPUT', tone: 'warn', hint: 'needs you' },
    { id: 'idle', label: 'IDLE', tone: 'idle', hint: 'quiet' },
    { id: 'outpost', label: 'OUTPOST', tone: 'idle', hint: 'not connected' },
  ];
</script>

<section class="mc" aria-labelledby="mc-page-title">
  <header class="page-header mc-header">
    <h1 id="mc-page-title">Mission Control</h1>
    <p class="mc-subtitle">
      Best-effort liveness · polled every {POLL_CADENCE_SECONDS}s · no sessions yet
    </p>
  </header>

  <div class="mc-summary" role="list" aria-label="Session summary">
    {#each SUMMARY_TILES as tile (tile.id)}
      <div class="mc-tile" role="listitem">
        <div class="mc-tile-label">
          <span class={`mc-dot ${tile.tone}`} aria-hidden="true"></span>
          {tile.label}
        </div>
        <div class="mc-tile-value">—</div>
        <div class="mc-tile-hint">{tile.hint}</div>
      </div>
    {/each}
  </div>

  <div class="mc-columns">
    <!-- US-007 mount point: LiveSessionsPanel renders into .mc-live-mount,
         subscribing to the sessions store and grouping live sessions by type. -->
    <div class="mc-col mc-col-live mc-live-mount" aria-label="Live sessions">
      <div class="mc-panel-eyebrow">LIVE SESSIONS</div>
      <div class="mc-placeholder">
        <div class="mc-placeholder-glyph" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor"
            stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M5 12a7 7 0 0 1 14 0" opacity="0.45" />
            <path d="M2 12a10 10 0 0 1 20 0" opacity="0.2" />
          </svg>
        </div>
        <p class="mc-placeholder-title">Nothing running right now</p>
        <p class="mc-placeholder-help">
          Claude Code and Codex sessions show up here the moment they start —
          locally or on your outpost.
        </p>
      </div>
    </div>

    <!-- US-008 mount point: SessionHistoryPanel renders into .mc-history-mount,
         a chronological feed derived from the audit log + thread files. -->
    <div class="mc-col mc-col-history mc-history-mount" aria-label="History">
      <div class="mc-panel-eyebrow">HISTORY</div>
      <div class="mc-placeholder">
        <div class="mc-placeholder-glyph" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor"
            stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 7v5l3 2" />
          </svg>
        </div>
        <p class="mc-placeholder-title">No activity yet</p>
        <p class="mc-placeholder-help">
          Completed stories, checkpoints and handoffs will appear here as your
          sessions work.
        </p>
      </div>
    </div>
  </div>
</section>

<style>
  /* Page shell mirrors the existing primary destinations (design.md
     "Navigation"): the outer .page already pads the surface; sections stack
     with an 18px gap. */
  .mc {
    display: flex;
    flex-direction: column;
    gap: 18px;
    min-height: 0;
  }

  .mc-header {
    margin-bottom: 0;
  }

  .mc-subtitle {
    margin: 6px 0 0;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    line-height: 1.3;
  }

  /* Summary strip — 4 inset tiles (#19191B, 10px radius): caps label + status
     dot + big Inter Tight value + hint. */
  .mc-summary {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
  }

  .mc-tile {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 10px;
    background: var(--v4-inset);
  }

  .mc-tile-label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .mc-tile-value {
    color: var(--v4-text-1);
    font-family: var(--font-display);
    font-size: var(--text-kpi);
    font-weight: 600;
    line-height: 1;
  }

  .mc-tile-hint {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.2;
  }

  .mc-dot {
    flex: 0 0 6px;
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--v4-idle);
  }

  .mc-dot.ok {
    background: var(--v4-ok);
  }

  .mc-dot.warn {
    background: var(--v4-warn);
  }

  .mc-dot.idle {
    background: var(--v4-idle);
  }

  /* Two columns — Live (flex-grow 5) left, History (flex-grow 3) right. */
  .mc-columns {
    display: flex;
    flex: 1 1 auto;
    gap: 18px;
    min-height: 0;
  }

  .mc-col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    min-height: 220px;
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 10px;
    background: var(--v4-raised);
  }

  .mc-col-live {
    flex: 5 1 0;
  }

  .mc-col-history {
    flex: 3 1 0;
  }

  .mc-panel-eyebrow {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }

  /* Placeholder mount-point content — replaced by the real panels in
     US-007 / US-008. Centered glyph + empty-state copy per design.md "States". */
  .mc-placeholder {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px 16px;
    text-align: center;
  }

  .mc-placeholder-glyph {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    color: var(--v4-text-3);
  }

  .mc-placeholder-title {
    margin: 0;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .mc-placeholder-help {
    max-width: 38ch;
    margin: 0;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.4;
  }

  @media (max-width: 720px) {
    .mc-summary {
      grid-template-columns: repeat(2, 1fr);
    }

    .mc-columns {
      flex-direction: column;
    }
  }
</style>
