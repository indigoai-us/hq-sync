<script lang="ts">
  /** Mission Control — running hq-pack-agency teams + per-worker status. */
  import { agencyStore } from '../lib/agency-store.svelte';
  import { statusTone, relativeTime, shortDuration, type AgencyWorker } from '../lib/agency';

  const teams = $derived(agencyStore.teams);
  const questions = $derived(agencyStore.questions);

  function workerLabel(worker: string, instance: string): string {
    return instance === 'main' ? worker : `${worker}:${instance}`;
  }
  function statusLabel(status: string, ready: boolean): string {
    if (status === 'running' && !ready) return 'booting';
    return status;
  }
  /** Uptime while running, else how long since the last status write. */
  function workerMeta(w: AgencyWorker): string {
    if (w.status === 'running' && w.startedAt) {
      const up = shortDuration(w.startedAt);
      return up ? `up ${up}` : '';
    }
    return relativeTime(w.updatedAt);
  }
  const runningCount = (workers: AgencyWorker[]) =>
    workers.filter((w) => w.status === 'running').length;
  const pendingFor = (company: string, team: string) =>
    questions.filter((q) => q.company === company && q.team === team).length;
</script>

<div class="atp">
  <header class="atp-head"><h2>Teams <span class="count">{teams.length}</span></h2></header>

  {#if teams.length === 0}
    <p class="empty">No agency teams running.</p>
  {:else}
    <div class="teams">
      {#each teams as t (t.company + '/' + t.team)}
        <section class="team">
          <div class="team-head">
            <span class="tname">{t.team}</span>
            <span class="tco">{t.company}</span>
            <span class="tsummary">
              {runningCount(t.workers)}/{t.workers.length} running
              {#if pendingFor(t.company, t.team)}<span class="waiting">{pendingFor(t.company, t.team)} waiting</span>{/if}
            </span>
          </div>
          <ul class="workers">
            {#each t.workers as w (w.worker + '/' + w.instance)}
              <li class="worker">
                <span class={`dot ${statusTone(w.status, w.ready)}`} aria-hidden="true"></span>
                <span class="wname">{workerLabel(w.worker, w.instance)}</span>
                <span class="wstatus">{statusLabel(w.status, w.ready)}</span>
                {#if workerMeta(w)}<span class="wmeta">{workerMeta(w)}</span>{/if}
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .atp { display: flex; flex-direction: column; gap: 12px; min-height: 0; }
  .atp-head h2 {
    margin: 0;
    font-size: var(--text-lg, 15px);
    color: var(--v4-text-1);
    display: flex; align-items: center; gap: 8px;
  }
  .count {
    font-family: var(--font-display);
    font-size: var(--text-base);
    color: var(--v4-text-3);
    background: var(--v4-inset);
    border: 1px solid var(--v4-hairline);
    border-radius: 999px;
    padding: 0 8px;
  }
  .empty { color: var(--v4-text-3); font-size: var(--text-base); margin: 8px 0; }
  .teams { display: flex; flex-direction: column; gap: 10px; overflow-y: auto; }
  .team {
    border: 1px solid var(--v4-hairline);
    border-radius: 10px;
    background: var(--v4-inset);
    padding: 10px 12px;
  }
  .team-head { display: flex; align-items: baseline; gap: 8px; margin-bottom: 6px; }
  .tname { color: var(--v4-text-1); font-weight: 600; font-size: var(--text-base); }
  .tco { color: var(--v4-text-3); font-size: var(--text-base); text-transform: uppercase; letter-spacing: 0.04em; }
  .tsummary { margin-left: auto; color: var(--v4-text-3); font-size: var(--text-base); display: flex; align-items: baseline; gap: 8px; }
  .waiting { color: var(--v4-warn); }
  .workers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .worker { display: flex; align-items: center; gap: 8px; font-size: var(--text-base); }
  .wname { color: var(--v4-text-2); flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wstatus { color: var(--v4-text-3); }
  .wmeta { color: var(--v4-text-3); font-variant-numeric: tabular-nums; }
  .dot { flex: 0 0 6px; width: 6px; height: 6px; border-radius: 999px; background: var(--v4-idle); }
  .dot.ok { background: var(--v4-ok); }
  .dot.warn { background: var(--v4-warn); }
  .dot.idle { background: var(--v4-idle); }
</style>
