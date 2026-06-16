<script lang="ts">
  /**
   * History timeline panel (US-008) — the chronological feed of what the fleet
   * *did* over time (tasks dispatched, stories completed, checkpoints, handoffs),
   * built to design.md "History timeline panel (US-008)".
   *
   * Subscribes to the SAME sessions store the live panel uses (US-005): the
   * `history` half of the `list_agent_sessions` snapshot / `sessions:updated`
   * poll event. No new backend call — the history feed is already client-side.
   *
   * Renders:
   *   - header: caps "HISTORY" + range ("last 24h").
   *   - filters row: a segmented tool filter (All · Claude · Codex) + a company
   *     dropdown chip.
   *   - feed: a vertical timeline rail (node dot on a line) + event body (title +
   *     project/company · relative time), newest-first, node color by event kind.
   *   - an empty state when no history exists (design.md "Empty — history").
   *
   * All filtering / sorting / tool-inference lives as pure helpers in
   * lib/sessions.ts (deriveEventTool / filterHistory / historyCompanies /
   * eventNodeTone) so it's trivially unit-testable; this file is just markup +
   * V4 Liquid Glass tokens (no new colors/fonts/spacing).
   */
  import { onMount } from 'svelte';
  import {
    eventNodeTone,
    filterHistory,
    historyCompanies,
    HISTORY_PAGE_SIZE,
    HISTORY_TOOL_FILTERS,
    relativeActivity,
    type HistoryEvent,
    type HistoryToolFilter,
  } from '../lib/sessions';
  import { sessionsStore, startSessionsStore } from '../lib/sessions-store.svelte';

  /** The range the feed covers, surfaced beside the header (design.md "last 24h"). */
  const HISTORY_RANGE_LABEL = 'last 24h';

  // Monotonic "now" tick so relative-time labels refresh on their own between
  // poll snapshots; recomputed every 15s (mirrors LiveSessionsPanel).
  let now = $state(Date.now());

  // Active filter selection (design.md "Filters row"). Default: everything.
  let toolFilter = $state<HistoryToolFilter>('all');
  let companyFilter = $state('');
  // How many events are currently revealed; grows by a page on "show more".
  let limit = $state(HISTORY_PAGE_SIZE);

  onMount(() => {
    // Lifetime singleton; idempotent. Keeps the panel self-sufficient (tests /
    // direct mount) just like the live panel.
    startSessionsStore();
    const tick = setInterval(() => {
      now = Date.now();
    }, 15_000);
    return () => clearInterval(tick);
  });

  // Company options come from the FULL feed (not the tool-filtered view) so the
  // dropdown is stable regardless of the active tool segment.
  const companies = $derived(historyCompanies(sessionsStore.history));

  // The filtered, newest-first feed. Re-derives on every poll snapshot or filter
  // change, with no manual refresh (design.md "Implementation notes").
  const filtered = $derived(
    filterHistory(sessionsStore.history, { tool: toolFilter, company: companyFilter }),
  );
  const shown = $derived(filtered.slice(0, limit));
  const overflow = $derived(Math.max(0, filtered.length - shown.length));

  function selectTool(value: HistoryToolFilter): void {
    toolFilter = value;
    limit = HISTORY_PAGE_SIZE; // reset pagination when the filter changes
  }

  function onCompanyChange(event: Event): void {
    companyFilter = (event.currentTarget as HTMLSelectElement).value;
    limit = HISTORY_PAGE_SIZE;
  }

  function showMore(): void {
    limit += HISTORY_PAGE_SIZE;
  }

  /** Event body meta: "project · company", omitting empty parts gracefully. */
  function metaFor(event: HistoryEvent): string {
    return [event.project, event.company].filter(Boolean).join(' · ');
  }
</script>

<section class="hi" aria-label="History">
  <header class="hi-head">
    <div class="hi-eyebrow">HISTORY</div>
    <span class="hi-range">{HISTORY_RANGE_LABEL}</span>
  </header>

  <div class="hi-filters" role="group" aria-label="History filters">
    <div class="hi-seg" role="tablist" aria-label="Tool filter">
      {#each HISTORY_TOOL_FILTERS as opt (opt.value)}
        <button
          type="button"
          role="tab"
          class="hi-seg-btn"
          class:active={toolFilter === opt.value}
          aria-selected={toolFilter === opt.value}
          onclick={() => selectTool(opt.value)}
        >
          {opt.label}
        </button>
      {/each}
    </div>

    <label class="hi-company">
      <span class="hi-sr">Company</span>
      <select class="hi-company-sel" value={companyFilter} onchange={onCompanyChange}>
        <option value="">All companies</option>
        {#each companies as company (company)}
          <option value={company}>{company}</option>
        {/each}
      </select>
    </label>
  </div>

  {#if sessionsStore.loading}
    <!-- Loading — first scan: skeleton timeline rows. -->
    <div class="hi-skeleton" aria-hidden="true">
      {#each [0, 1, 2, 3] as r (r)}
        <div class="hi-skel-row">
          <span class="hi-skel-dot"></span>
          <span class="hi-skel-bar" style={`width:${72 - r * 10}%`}></span>
        </div>
      {/each}
    </div>
  {:else if sessionsStore.error}
    <div class="hi-empty">
      <p class="hi-empty-title">Couldn't load history</p>
      <p class="hi-empty-help">{sessionsStore.error}</p>
    </div>
  {:else if filtered.length === 0}
    <!-- Empty — history (design.md "States"). -->
    <div class="hi-empty">
      <div class="hi-empty-glyph" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor"
          stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7v5l3 2" />
        </svg>
      </div>
      <p class="hi-empty-title">No activity yet</p>
      <p class="hi-empty-help">
        Completed stories, checkpoints and handoffs will appear here as your
        sessions work.
      </p>
    </div>
  {:else}
    <ol class="hi-feed">
      {#each shown as event, i (event.source + '|' + event.timestamp + '|' + event.title + '|' + i)}
        {@const isLast = i === shown.length - 1 && overflow === 0}
        <li class="hi-item">
          <div class="hi-rail" aria-hidden="true">
            <span class={`hi-node ${eventNodeTone(event.kind)}`}></span>
            {#if !isLast}
              <span class="hi-line"></span>
            {/if}
          </div>
          <div class="hi-body">
            <div class="hi-title">{event.title}</div>
            <div class="hi-meta">
              {#if metaFor(event)}
                <span class="hi-meta-where">{metaFor(event)}</span>
                <span class="hi-meta-sep" aria-hidden="true">·</span>
              {/if}
              <span class="hi-meta-time">{relativeActivity(event.timestamp, now)}</span>
            </div>
          </div>
        </li>
      {/each}
    </ol>

    {#if overflow > 0}
      <button type="button" class="hi-more" onclick={showMore}>
        Show {Math.min(overflow, HISTORY_PAGE_SIZE)} more …
      </button>
    {/if}
  {/if}
</section>

<style>
  .hi {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
  }

  .hi-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }

  .hi-eyebrow {
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }

  .hi-range {
    color: var(--v4-text-3);
    font-family: var(--font-mono);
    font-size: 11px;
  }

  /* Filters row — segmented tool filter + company dropdown chip. */
  .hi-filters {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .hi-seg {
    display: inline-flex;
    padding: 2px;
    border-radius: 8px;
    background: var(--v4-control-faint);
  }

  .hi-seg-btn {
    padding: 3px 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }

  .hi-seg-btn:hover {
    color: var(--v4-text-1);
  }

  .hi-seg-btn.active {
    background: var(--v4-active-row);
    color: var(--v4-text-1);
    font-weight: 500;
  }

  .hi-company {
    display: inline-flex;
    align-items: center;
  }

  .hi-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .hi-company-sel {
    padding: 3px 8px;
    border: 1px solid var(--v4-control-border);
    border-radius: 8px;
    background: var(--v4-control-faint);
    color: var(--v4-text-2);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }

  /* Feed — vertical timeline rail + event body. */
  .hi-feed {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
    min-height: 0;
  }

  .hi-item {
    display: flex;
    gap: 10px;
  }

  /* Rail — the node dot sits on a connecting line; the last node has no
     trailing line (design.md "Last event's trailing rail line is hidden"). */
  .hi-rail {
    position: relative;
    display: flex;
    flex: 0 0 8px;
    flex-direction: column;
    align-items: center;
    width: 8px;
  }

  .hi-node {
    flex: 0 0 8px;
    width: 8px;
    height: 8px;
    margin-top: 4px;
    border-radius: 999px;
    background: var(--v4-idle);
  }

  .hi-node.ok {
    background: var(--v4-ok);
  }

  .hi-node.neutral {
    background: var(--v4-text-2);
  }

  .hi-node.faint {
    background: var(--v4-text-3);
  }

  .hi-node.error {
    background: var(--v4-error);
  }

  .hi-line {
    flex: 1 1 auto;
    width: 1px;
    margin-top: 2px;
    background: var(--v4-control-border);
  }

  .hi-body {
    flex: 1 1 auto;
    min-width: 0;
    padding-bottom: 12px;
  }

  .hi-title {
    color: var(--v4-text-1);
    font-size: 12px;
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hi-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    color: var(--v4-text-2);
    font-size: 11px;
  }

  .hi-meta-where {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .hi-meta-sep {
    color: var(--v4-text-3);
  }

  .hi-meta-time {
    flex: 0 0 auto;
    color: var(--v4-text-3);
    font-family: var(--font-mono);
  }

  /* "Show N more" roll-up. */
  .hi-more {
    align-self: flex-start;
    padding: 4px 0;
    border: none;
    background: transparent;
    color: var(--v4-text-3);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }

  .hi-more:hover {
    color: var(--v4-text-2);
  }

  /* Empty + error states. */
  .hi-empty {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px 16px;
    text-align: center;
  }

  .hi-empty-glyph {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    color: var(--v4-text-3);
  }

  .hi-empty-title {
    margin: 0;
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 500;
  }

  .hi-empty-help {
    max-width: 38ch;
    margin: 0;
    color: var(--v4-text-3);
    font-size: 12px;
    line-height: 1.4;
  }

  /* Loading skeleton — node + bar rows. */
  .hi-skeleton {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .hi-skel-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .hi-skel-dot {
    flex: 0 0 8px;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.07);
  }

  .hi-skel-bar {
    height: 12px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.04);
  }
</style>
