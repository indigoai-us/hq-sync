<script lang="ts">
  import {
    rangeLabel,
    signalCounts,
    type MeetingEvent,
  } from '../lib/meetings-model';

  interface Props {
    events: MeetingEvent[];
    upNext: MeetingEvent | null;
  }

  let { events, upNext }: Props = $props();
</script>

<section class="today-panel" aria-labelledby="today-title">
  <div class="panel-header">
    <h2 id="today-title">Today</h2>
    <span>{events.length} meeting{events.length === 1 ? '' : 's'}</span>
  </div>

  <div class="up-next">
    <span>Up next</span>
    {#if upNext}
      <strong>{upNext.summary ?? '(no title)'}</strong>
      <small>{rangeLabel(upNext)}</small>
    {:else}
      <strong>No more meetings</strong>
      <small>Schedule is clear for today.</small>
    {/if}
  </div>

  <ol class="meeting-list">
    {#each events as event (event.id)}
      {@const counts = signalCounts(event)}
      <li>
        <time>{rangeLabel(event)}</time>
        <div class="meeting-main">
          <strong>{event.summary ?? '(no title)'}</strong>
          <span>
            {counts.actions} A / {counts.decisions} D / {counts.risks} R
          </span>
        </div>
      </li>
    {:else}
      <li class="empty-row">No meetings on today's cached schedule.</li>
    {/each}
  </ol>
</section>

<style>
  .today-panel {
    min-width: 0;
  }

  .panel-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 10px;
  }

  .panel-header h2 {
    margin: 0;
    color: var(--fg);
    font-size: 15px;
    font-weight: 680;
    line-height: 22px;
  }

  .panel-header span {
    color: var(--muted);
    font-size: 12px;
  }

  .up-next,
  .meeting-list {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }

  .up-next {
    display: grid;
    gap: 2px;
    padding: 12px;
    margin-bottom: 8px;
  }

  .up-next span,
  .up-next small,
  .meeting-main span,
  .meeting-list time {
    color: var(--muted);
    font-size: 12px;
    line-height: 17px;
  }

  .up-next span {
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
  }

  .up-next strong {
    overflow: hidden;
    color: var(--fg);
    font-size: 14px;
    font-weight: 680;
    line-height: 20px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meeting-list {
    display: grid;
    gap: 0;
    margin: 0;
    padding: 6px 0;
    list-style: none;
  }

  .meeting-list li {
    display: grid;
    grid-template-columns: 78px minmax(0, 1fr);
    gap: 10px;
    min-height: 46px;
    padding: 8px 12px;
    transition:
      background 140ms cubic-bezier(.2, .7, .2, 1),
      transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .meeting-list li:not(.empty-row):hover {
    background: var(--row-hover);
    transform: translateX(2px);
  }

  .meeting-list .empty-row {
    display: block;
    min-height: 0;
    color: var(--muted);
    font-size: 12px;
    line-height: 18px;
  }

  .meeting-list time {
    padding-top: 1px;
    white-space: nowrap;
  }

  .meeting-main {
    min-width: 0;
  }

  .meeting-main strong,
  .meeting-main span {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meeting-main strong {
    color: var(--fg);
    font-size: 13px;
    font-weight: 650;
    line-height: 18px;
  }

  @media (prefers-reduced-motion: reduce) {
    .meeting-list li {
      transition: none;
    }

    .meeting-list li:not(.empty-row):hover {
      transform: none;
    }
  }

  @media (max-width: 520px) {
    .meeting-list li {
      grid-template-columns: minmax(0, 1fr);
      gap: 2px;
    }

    .meeting-list time {
      padding-top: 0;
    }
  }
</style>
