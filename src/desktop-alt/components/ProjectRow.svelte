<script lang="ts">
  /**
   * ProjectRow — a single project rendered as a Foundry/"ops console" card in
   * the Board grid (US-007), matching the Library page card language
   * (LibraryList.svelte): dark surface tile, near-square corners, hairline
   * border, a status-colored left accent bar, a monospace ALL-CAPS status
   * micro-label, a scope pill, a 2-line description, and an acceptance progress
   * footer. Token-driven only — no hardcoded hex.
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
    /** Whether to show the company pill (hidden when grouped by company). */
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
</script>

<button
  type="button"
  class="project-card"
  class:is-live={isLive}
  data-status={status}
  data-testid="project-row"
  aria-label={`Project ${projectDisplayName(project)}`}
  onclick={activate}
>
  <span class="accent" data-status={status} aria-hidden="true"></span>

  <div class="card-head">
    <span class="status-tag" data-status={status}>
      <span class="status-dot" class:is-live={isLive} data-status={status} aria-hidden="true"></span>
      {PROJECT_LIST_STATUS_LABEL[status]}
    </span>
    {#if showCompany && project.company}
      <span class="pill company" title={project.company}>{project.company}</span>
    {/if}
  </div>

  <h3 class="card-name" title={projectDisplayName(project)}>{projectDisplayName(project)}</h3>

  {#if project.description}
    <p class="card-desc">{project.description}</p>
  {/if}

  {#if hasProgress}
    <div class="card-progress">
      <div class="progress-track" aria-hidden="true">
        <div
          class="progress-fill"
          data-status={status}
          style={`width: ${progress.percent}%;`}
        ></div>
      </div>
      <span class="progress-count">{progress.complete}/{progress.total}</span>
      <span class="progress-percent">{progress.percent}%</span>
    </div>
  {/if}
</button>

<style>
  .project-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    /* Extra left pad clears the accent bar (matches .lib-card). */
    padding: var(--space-3) var(--space-3) var(--space-3) calc(var(--space-3) + 4px);
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    text-align: left;
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      transform 140ms ease;
  }

  .project-card:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    transform: translateY(-1px);
  }

  .project-card:active {
    transform: translateY(0);
  }

  .project-card:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  /* Status-colored left accent bar — brightens on hover. Live pulses. */
  .accent {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    width: 3px;
    background: var(--muted-3);
    opacity: 0.55;
    transition: opacity 140ms ease;
  }
  .accent[data-status='live'] {
    background: var(--emerald);
  }
  .accent[data-status='in-progress'] {
    background: var(--blue);
  }
  .accent[data-status='complete'] {
    background: var(--muted-2);
  }
  .project-card:hover .accent {
    opacity: 1;
  }
  .project-card.is-live .accent {
    opacity: 1;
    animation: accent-pulse 1.8s ease-in-out infinite;
  }

  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-width: 0;
  }

  /* Monospace ALL-CAPS status micro-label — the signature ops-console tag. */
  .status-tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    color: var(--muted-2);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.09em;
    line-height: 15px;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .status-tag[data-status='live'] {
    color: var(--emerald);
  }

  .status-dot {
    flex: 0 0 auto;
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--muted-3);
  }
  .status-dot[data-status='in-progress'] {
    background: var(--blue);
  }
  .status-dot[data-status='complete'] {
    background: var(--muted-2);
  }
  .status-dot.is-live {
    background: var(--emerald);
    animation: dot-pulse 1.8s ease-in-out infinite;
  }

  /* Scope pill (company) — mono, uppercase, hairline (matches .lib-card .pill). */
  .pill {
    display: inline-flex;
    align-items: center;
    flex: 0 0 auto;
    max-width: 50%;
    overflow: hidden;
    padding: 1px 7px;
    border: 1px solid color-mix(in srgb, var(--blue) 38%, transparent);
    border-radius: 3px;
    background: var(--row-hover);
    color: var(--blue);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.05em;
    line-height: 15px;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .card-name {
    margin: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-desc {
    margin: 2px 0 0;
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    /* Clamp to two lines so tiles stay uniform. */
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .card-progress {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-top: var(--space-1);
  }

  .progress-track {
    flex: 1 1 auto;
    height: 4px;
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

  .progress-count {
    flex: 0 0 auto;
    color: var(--muted-2);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
    line-height: 16px;
  }

  .progress-percent {
    flex: 0 0 auto;
    min-width: 30px;
    color: var(--muted-3);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
    line-height: 16px;
    text-align: right;
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

  @keyframes accent-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.5;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .project-card,
    .progress-fill,
    .accent {
      transition: none;
    }

    .project-card:hover,
    .project-card:active {
      transform: none;
    }

    .status-dot.is-live,
    .project-card.is-live .accent {
      animation: none;
    }
  }
</style>
