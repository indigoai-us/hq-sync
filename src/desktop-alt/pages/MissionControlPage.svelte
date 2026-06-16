<script lang="ts">
  /**
   * Mission Control — global, cross-company fleet view of running Claude Code
   * and Codex agent sessions (local + outpost), observed best-effort with no
   * vendor auth (US-006).
   *
   * Renders the page chrome plus the two panels:
   *   1. Header — title + best-effort / poll-cadence subtitle.
   *   2. Summary strip — 4 inset stat tiles (RUNNING / AWAITING INPUT / IDLE /
   *      OUTPOST), per design.md "Page structure".
   *   3. Two columns — LiveSessionsPanel (US-007) into `.mc-live-mount` and
   *      SessionHistoryPanel (US-008) into `.mc-history-mount`, both subscribing
   *      to the shared sessions store.
   *
   * Built entirely on the V4 "Liquid Glass" tokens (src/desktop-alt/v4/tokens.css);
   * no new colors, fonts, or spacing primitives (design.md "Tokens used").
   */
  import { onMount } from 'svelte';
  import LiveSessionsPanel from '../panels/LiveSessionsPanel.svelte';
  import SessionHistoryPanel from '../panels/SessionHistoryPanel.svelte';
  import { sessionsStore, startSessionsStore } from '../lib/sessions-store.svelte';
  import type { SessionStatus } from '../lib/sessions';

  /**
   * Desktop polling cadence, in seconds. The sessions store (US-005) re-scans on
   * this interval and emits a typed event the panels subscribe to; surfaced in
   * the header subtitle so the operator knows liveness is sampled, not live.
   */
  const POLL_CADENCE_SECONDS = 5;

  onMount(() => {
    // Lifetime singleton; idempotent. Starting here makes the page self-sufficient.
    startSessionsStore();
  });

  // Live per-status counts off the store, for the summary strip. Derived so a
  // poll snapshot repaints the tiles with no manual refresh.
  const statusCounts = $derived.by(() => {
    const counts: Record<SessionStatus, number> = {
      running: 0,
      awaiting_input: 0,
      idle: 0,
      ended: 0,
    };
    for (const s of sessionsStore.sessions) counts[s.status] += 1;
    return counts;
  });
  const sessionTotal = $derived(sessionsStore.sessions.length);
  const localTotal = $derived(
    sessionsStore.sessions.filter((s) => s.origin === 'local').length,
  );
  const outpostTotal = $derived(
    sessionsStore.sessions.filter((s) => s.origin === 'outpost').length,
  );

  /**
   * Summary tiles, in design.md "Page structure" order. The value is read live
   * from the store via the `value` accessor. The dot tone is the status taxonomy
   * color (tokens.css `--v4-*`).
   */
  const SUMMARY_TILES: ReadonlyArray<{
    id: string;
    label: string;
    tone: 'ok' | 'warn' | 'idle';
    hint: () => string;
    value: () => number;
  }> = [
    { id: 'running', label: 'RUNNING', tone: 'ok', hint: () => 'live now', value: () => statusCounts.running },
    { id: 'awaiting', label: 'AWAITING INPUT', tone: 'warn', hint: () => 'needs you', value: () => statusCounts.awaiting_input },
    { id: 'idle', label: 'IDLE', tone: 'idle', hint: () => 'quiet', value: () => statusCounts.idle },
    { id: 'outpost', label: 'OUTPOST', tone: 'idle', hint: () => (outpostTotal > 0 ? 'reporting' : 'not connected'), value: () => outpostTotal },
  ];
</script>

<section class="mc" aria-labelledby="mc-page-title">
  <header class="page-header mc-header">
    <h1 id="mc-page-title">Mission Control</h1>
    <p class="mc-subtitle">
      Best-effort liveness · polled every {POLL_CADENCE_SECONDS}s ·
      {sessionTotal} {sessionTotal === 1 ? 'session' : 'sessions'} ·
      {localTotal} local · {outpostTotal} outpost
    </p>
  </header>

  <div class="mc-summary" role="list" aria-label="Session summary">
    {#each SUMMARY_TILES as tile (tile.id)}
      <div class="mc-tile" role="listitem">
        <div class="mc-tile-label">
          <span class={`mc-dot ${tile.tone}`} aria-hidden="true"></span>
          {tile.label}
        </div>
        <div class="mc-tile-value">{tile.value()}</div>
        <div class="mc-tile-hint">{tile.hint()}</div>
      </div>
    {/each}
  </div>

  <div class="mc-columns">
    <!-- US-007: LiveSessionsPanel renders into .mc-live-mount, subscribing to the
         sessions store and grouping live sessions by inferred type. -->
    <div class="mc-col mc-col-live mc-live-mount" aria-label="Live sessions">
      <LiveSessionsPanel />
    </div>

    <!-- US-008: SessionHistoryPanel renders into .mc-history-mount, a
         chronological timeline derived from the snapshot's history feed
         (audit log + thread files), filterable by tool + company. -->
    <div class="mc-col mc-col-history mc-history-mount" aria-label="History">
      <SessionHistoryPanel />
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

  @media (max-width: 720px) {
    .mc-summary {
      grid-template-columns: repeat(2, 1fr);
    }

    .mc-columns {
      flex-direction: column;
    }
  }
</style>
