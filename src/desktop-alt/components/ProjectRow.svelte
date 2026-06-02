<script lang="ts">
  /**
   * ProjectRow — a single clickable project in the Board list (US-007).
   *
   * Ported from hq-desktop's ProjectRow: status dot (pulsing when live), title,
   * status badge, company badge, one-line description, and a complete/total
   * progress bar driven by US-004's `projectProgress`. Live projects get a
   * left-edge accent + faint glow for visual emphasis. Token-driven only.
   */
  import {
    projectDisplayName,
    projectListStatus,
    projectProgress,
    PROJECT_LIST_STATUS_LABEL,
    type Project,
  } from '../lib/projects-model';

  interface Props {
    project: Project;
    /** Whether to show the company badge (hidden when grouped by company). */
    showCompany?: boolean;
    onselect?: (project: Project) => void;
  }

  let { project, showCompany = true, onselect }: Props = $props();

  const status = $derived(projectListStatus(project));
  const isLive = $derived(status === 'live');
  const progress = $derived(projectProgress(project.storiesComplete, project.storiesTotal));
  const hasProgress = $derived(progress.total > 0);

  function activate() {
    onselect?.(project);
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      activate();
    }
  }
</script>

<div
  class="project-row"
  class:is-live={isLive}
  role="button"
  tabindex="0"
  data-status={status}
  data-testid="project-row"
  onclick={activate}
  onkeydown={onKeydown}
>
  <div class="row-main">
    <div class="row-heading">
      <span class="status-dot" class:is-live={isLive} data-status={status} aria-hidden="true"></span>
      <h3 class="project-name">{projectDisplayName(project)}</h3>
      <span class="status-badge" data-status={status}>{PROJECT_LIST_STATUS_LABEL[status]}</span>
      {#if showCompany && project.company}
        <span class="company-badge">{project.company}</span>
      {/if}
    </div>
    {#if project.description}
      <p class="project-description">{project.description}</p>
    {/if}
  </div>

  <div class="row-trailing">
    {#if hasProgress}
      <span class="progress-count">{progress.complete}/{progress.total}</span>
      <div class="progress-track" aria-hidden="true">
        <div
          class="progress-fill"
          data-status={status}
          style={`width: ${progress.percent}%;`}
        ></div>
      </div>
      <span class="progress-percent">{progress.percent}%</span>
    {/if}
    <span class="chevron" aria-hidden="true">›</span>
  </div>
</div>

<style>
  .project-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    min-width: 0;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      transform 140ms ease;
  }

  .project-row:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    transform: translateY(-1px);
  }

  .project-row:active {
    transform: translateY(0);
  }

  .project-row:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  /* Live projects get a left accent rail + a faint glow. */
  .project-row.is-live {
    border-left: 2px solid var(--emerald);
    box-shadow: inset 3px 0 0 -1px var(--emerald);
  }

  .row-main {
    min-width: 0;
    flex: 1 1 auto;
  }

  .row-heading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .status-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--muted-3);
  }

  .status-dot[data-status='in-progress'] {
    background: var(--blue);
  }

  .status-dot[data-status='complete'] {
    background: var(--muted-2);
  }

  .status-dot[data-status='archived'] {
    background: var(--muted-3);
  }

  .status-dot.is-live {
    background: var(--emerald);
    animation: dot-pulse 1.8s ease-in-out infinite;
  }

  .project-name {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status-badge {
    flex: 0 0 auto;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-xs);
    font-weight: 650;
    line-height: 15px;
    white-space: nowrap;
  }

  .status-badge[data-status='live'] {
    background: color-mix(in srgb, var(--emerald) 18%, transparent);
    color: var(--emerald);
  }

  .company-badge {
    flex: 0 0 auto;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: var(--row-hover);
    color: var(--muted);
    font-size: var(--text-xs);
    font-weight: 600;
    line-height: 15px;
    white-space: nowrap;
  }

  .project-description {
    margin: var(--space-1) 0 0;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-xs);
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row-trailing {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: var(--space-3);
    padding-top: 1px;
  }

  .progress-count {
    color: var(--muted-2);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    line-height: 16px;
  }

  .progress-track {
    width: 84px;
    height: 6px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-hover);
  }

  .progress-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--muted-2);
    transition: width 300ms ease;
  }

  .progress-fill[data-status='live'] {
    background: var(--emerald);
  }

  .progress-fill[data-status='in-progress'] {
    background: var(--blue);
  }

  .progress-fill[data-status='complete'] {
    background: var(--muted-2);
  }

  .progress-percent {
    width: 34px;
    color: var(--muted-3);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    line-height: 16px;
    text-align: right;
  }

  .chevron {
    color: var(--muted-3);
    font-size: var(--text-lg);
    line-height: 1;
  }

  @keyframes dot-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .project-row,
    .progress-fill {
      transition: none;
    }

    .project-row:hover,
    .project-row:active {
      transform: none;
    }

    .status-dot.is-live {
      animation: none;
    }
  }
</style>
