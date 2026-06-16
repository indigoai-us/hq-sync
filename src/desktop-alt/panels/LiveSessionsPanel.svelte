<script lang="ts">
  /**
   * Live Sessions panel (US-007) — the dense, collapsible, grouped view of the
   * running fleet, built to design.md "Live Sessions panel (US-007)".
   *
   * Subscribes to the sessions store (fed by `list_agent_sessions` + the
   * `sessions:updated` poll event, US-005) and renders ACTIVE sessions as a list
   * of type *groups*, each a collapsible cluster:
   *   - group header: caret · type name · count chip · status pips · freshest
   *     relative activity
   *   - dense single-line rows: status dot · name · company · model (mono) ·
   *     relative last-activity (mono)
   *   - a "+N more" roll-up when a group overflows the row cap.
   *
   * Grouping is by an inferred best-effort "kind" (see `deriveSessionKind` /
   * `groupSessions` in lib/sessions.ts) since the AgentSession contract has no
   * type field. Liveness is visibly labeled best-effort. V4 Liquid Glass tokens
   * only — no new colors/fonts/spacing.
   */
  import { onMount } from 'svelte';
  import {
    groupSessions,
    isActiveForLivePanel,
    relativeActivity,
    SESSION_STATUSES,
    type AgentSession,
    type SessionStatus,
  } from '../lib/sessions';
  import { sessionsStore, startSessionsStore } from '../lib/sessions-store.svelte';

  /** Max dense rows rendered per group before the "+N more" roll-up. */
  const ROWS_PER_GROUP = 6;

  /** Backend poll cadence, surfaced in the best-effort label (mirrors the
   *  Rust SESSIONS_POLL_INTERVAL_SECS default). Display-only. */
  const POLL_CADENCE_SECONDS = 5;

  // A monotonic "now" tick so relative-time labels refresh on their own even
  // between poll snapshots; recomputed every 15s.
  let now = $state(Date.now());

  onMount(() => {
    // The store is a lifetime singleton; calling start here keeps the panel
    // self-sufficient (tests / direct mount) and is idempotent.
    startSessionsStore();
    const tick = setInterval(() => {
      now = Date.now();
    }, 15_000);
    return () => clearInterval(tick);
  });

  // Active fleet → groups. `isActiveForLivePanel` drops long-ended sessions but
  // keeps recently-ended ones so a just-finished session is still visible.
  const activeSessions = $derived(
    sessionsStore.sessions.filter((s) => isActiveForLivePanel(s, now)),
  );
  const groups = $derived(groupSessions(activeSessions));
  const totalActive = $derived(activeSessions.length);
  const typeCount = $derived(groups.length);

  // Collapsed groups, keyed by group key. Default: everything expanded; the
  // operator collapses the clusters they don't care about. Re-assigned a cloned
  // Set on toggle so Svelte 5 sees a fresh reference.
  let collapsed = $state<Set<string>>(new Set());

  function toggle(key: string): void {
    const next = new Set(collapsed);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    collapsed = next;
  }

  /** Display name for a dense row: the channel handle / project / session slug,
   *  falling back to a short session-id when nothing else is known. */
  function rowName(session: AgentSession): string {
    if (session.project.trim()) return session.project;
    const tail = session.cwd.split('/').filter(Boolean).pop();
    if (tail) return tail;
    return session.id.slice(0, 8);
  }

  /** Status pips for a group header — non-zero statuses only, in taxonomy order. */
  function pips(counts: Record<SessionStatus, number>): Array<{ status: SessionStatus; count: number }> {
    return SESSION_STATUSES.filter((s) => counts[s] > 0).map((s) => ({
      status: s,
      count: counts[s],
    }));
  }

  /** Whether a row's name should dim (idle / ended) per the status taxonomy. */
  function isDimmed(status: SessionStatus): boolean {
    return status === 'idle' || status === 'ended';
  }
</script>

<section class="ls" aria-label="Live sessions">
  <header class="ls-head">
    <div class="ls-eyebrow">LIVE SESSIONS</div>
    <div class="ls-head-meta">
      <span class="ls-count">{totalActive} across {typeCount} {typeCount === 1 ? 'type' : 'types'}</span>
      <span class="ls-group-ctl" aria-hidden="true">Group: Type ▾</span>
      <span class="ls-besteffort" title="Liveness is sampled from on-disk activity, not a live connection.">
        <span class="ls-dot warn" aria-hidden="true"></span>
        best-effort · {POLL_CADENCE_SECONDS}s
      </span>
    </div>
  </header>

  {#if sessionsStore.loading}
    <!-- Loading — first scan: skeleton group header + rows (design.md States). -->
    <div class="ls-skeleton" aria-hidden="true">
      {#each [0, 1] as g (g)}
        <div class="ls-skel-head"></div>
        {#each [0, 1, 2] as r (r)}
          <div class="ls-skel-row" style={`width:${70 - r * 12}%`}></div>
        {/each}
      {/each}
    </div>
  {:else if sessionsStore.error}
    <div class="ls-empty">
      <p class="ls-empty-title">Couldn't load sessions</p>
      <p class="ls-empty-help">{sessionsStore.error}</p>
    </div>
  {:else if totalActive === 0}
    <!-- Empty — live (design.md States). -->
    <div class="ls-empty">
      <div class="ls-empty-glyph" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor"
          stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M5 12a7 7 0 0 1 14 0" opacity="0.45" />
          <path d="M2 12a10 10 0 0 1 20 0" opacity="0.2" />
        </svg>
      </div>
      <p class="ls-empty-title">Nothing running right now</p>
      <p class="ls-empty-help">
        Claude Code and Codex sessions show up here the moment they start —
        locally or on your outpost.
      </p>
    </div>
  {:else}
    <div class="ls-groups" role="list">
      {#each groups as group (group.key)}
        {@const isCollapsed = collapsed.has(group.key)}
        {@const shown = group.sessions.slice(0, ROWS_PER_GROUP)}
        {@const overflow = group.count - shown.length}
        <div class="ls-group" role="listitem">
          <button
            type="button"
            class="ls-group-head"
            aria-expanded={!isCollapsed}
            onclick={() => toggle(group.key)}
          >
            <span class="ls-caret" aria-hidden="true">{isCollapsed ? '▸' : '▾'}</span>
            <span class="ls-group-name">{group.label}</span>
            <span class="ls-chip">{group.count}</span>
            <span class="ls-spacer"></span>
            <span class="ls-pips" aria-hidden="true">
              {#each pips(group.statusCounts) as pip (pip.status)}
                <span class="ls-pip">
                  <span class={`ls-dot ${pip.status}`}></span>{pip.count}
                </span>
              {/each}
            </span>
            <span class="ls-fresh">{relativeActivity(group.freshestActivityAt, now)}</span>
          </button>

          {#if !isCollapsed}
            <div class="ls-rows">
              {#each shown as session (session.id)}
                <div class="ls-row" class:dim={isDimmed(session.status)}>
                  <span class={`ls-dot ${session.status}`} aria-hidden="true"></span>
                  <span class="ls-name" title={session.cwd}>{rowName(session)}</span>
                  <span class="ls-company">{session.company || '—'}</span>
                  <span class="ls-spacer"></span>
                  <span class="ls-model">{session.model || '—'}</span>
                  <span class="ls-time">{relativeActivity(session.lastActivityAt, now)}</span>
                </div>
              {/each}
              {#if overflow > 0}
                <div class="ls-more">+{overflow} more …</div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .ls {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
  }

  .ls-head {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .ls-eyebrow {
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }

  .ls-head-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .ls-count {
    color: var(--v4-text-2);
    font-size: 12px;
  }

  .ls-group-ctl {
    padding: 2px 8px;
    border: 1px solid var(--v4-control-border);
    border-radius: 8px;
    background: var(--v4-control-faint);
    color: var(--v4-text-2);
    font-size: 11px;
  }

  .ls-besteffort {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: 999px;
    background: rgba(254, 188, 46, 0.12);
    color: var(--v4-warn);
    font-size: 11px;
  }

  /* Status dot — shared across pips, rows, and the best-effort label. */
  .ls-dot {
    display: inline-block;
    flex: 0 0 6px;
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--v4-idle);
  }
  .ls-dot.running {
    background: var(--v4-ok);
  }
  .ls-dot.awaiting_input,
  .ls-dot.warn {
    background: var(--v4-warn);
  }
  .ls-dot.idle {
    background: var(--v4-idle);
  }
  .ls-dot.ended {
    background: var(--v4-idle);
    opacity: 0.55;
  }

  .ls-groups {
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
    min-height: 0;
  }

  .ls-group {
    display: flex;
    flex-direction: column;
  }

  /* Group header — raised, 8px radius, 36px tall, caret + name + chip + pips. */
  .ls-group-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 36px;
    padding: 8px 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
    color: var(--v4-text-1);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .ls-group-head:hover {
    background: var(--v4-active-row);
  }

  .ls-caret {
    flex: 0 0 auto;
    color: var(--v4-text-3);
    font-size: 10px;
  }

  .ls-group-name {
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 500;
  }

  .ls-chip {
    padding: 0 6px;
    border-radius: 999px;
    background: var(--v4-control-faint);
    color: var(--v4-text-2);
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 18px;
  }

  .ls-spacer {
    flex: 1 1 auto;
  }

  .ls-pips {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .ls-pip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--v4-text-2);
    font-size: 11px;
  }

  .ls-fresh {
    color: var(--v4-text-3);
    font-family: var(--font-mono);
    font-size: 11px;
    text-align: right;
    min-width: 36px;
  }

  /* Dense rows — ~30px, indented under the caret, hairline separators. */
  .ls-rows {
    display: flex;
    flex-direction: column;
    padding-left: 30px;
  }

  .ls-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 30px;
    padding: 6px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  }

  .ls-row:last-child {
    border-bottom: none;
  }

  .ls-name {
    color: var(--v4-text-1);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 38%;
  }

  .ls-row.dim .ls-name {
    color: var(--v4-text-2);
  }

  .ls-company {
    flex: 0 0 auto;
    color: var(--v4-text-2);
    font-size: 11px;
    white-space: nowrap;
  }

  .ls-model {
    color: var(--v4-text-3);
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: nowrap;
  }

  .ls-time {
    color: var(--v4-text-3);
    font-family: var(--font-mono);
    font-size: 11px;
    text-align: right;
    min-width: 34px;
  }

  /* "+N more" roll-up. */
  .ls-more {
    padding: 6px 12px;
    color: var(--v4-text-3);
    font-size: 11px;
  }

  /* Empty + error states. */
  .ls-empty {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px 16px;
    text-align: center;
  }

  .ls-empty-glyph {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    color: var(--v4-text-3);
  }

  .ls-empty-title {
    margin: 0;
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 500;
  }

  .ls-empty-help {
    max-width: 38ch;
    margin: 0;
    color: var(--v4-text-3);
    font-size: 12px;
    line-height: 1.4;
  }

  /* Loading skeleton — rounded bars, varied widths (design.md States). */
  .ls-skeleton {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .ls-skel-head {
    height: 36px;
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.07);
  }

  .ls-skel-row {
    height: 14px;
    margin-left: 30px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.04);
  }
</style>
