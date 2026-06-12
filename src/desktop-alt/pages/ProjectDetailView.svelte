<script lang="ts">
  /**
   * ProjectDetailView — a single project's detail surface (US-009).
   *
   * Ported from hq-desktop's project-detail-view.tsx: a header with the project
   * title, description, company, content indicators (PRD / README present),
   * progress, and a status control — then the project's README rendered as
   * markdown in-app, with the story Kanban embedded below via a small
   * Overview / Board tab.
   *
   * README reader: the project's directory is the parent of its prdPath, so the
   * sibling README.md is read by the US-009 Rust command
   * `get_local_project_readme(prdPath)` (mirrors US-003's path-traversal guard).
   * Markdown is rendered by the dependency-free `lib/markdown.ts` helper (escaped
   * input + a fixed safe tag set — no `marked`, no DOM sanitizer, CSP-safe).
   *
   * Status control: WRITABLE (US-010). Selecting a status persists to the
   * company `board.json` via projects-store with OPTIMISTIC UI — the rendered
   * status updates immediately, then rolls back + shows a clear error if the
   * write fails. Persistence + rollback live in `lib/projects-store.svelte.ts`.
   */
  import { onMount } from 'svelte';
  import {
    loadLocalProjectPrd,
    loadLocalProjectReadme,
    type LocalProjectPrdWire,
    type Objective,
  } from '../lib/local-projects';
  import { setProjectStatus } from '../lib/projects-store.svelte';
  import { renderMarkdown } from '../lib/markdown';
  import {
    classifyStories,
    projectDisplayName,
    projectProgress,
    toEditableStatus,
    EDITABLE_PROJECT_STATUSES,
    EDITABLE_PROJECT_STATUS_LABEL,
    type EditableProjectStatus,
    type Project,
    type Story,
  } from '../lib/projects-model';
  import StoryKanban from '../components/StoryKanban.svelte';

  interface Props {
    /** The project whose detail to show. */
    project: Project;
    /** Stories for this project's prd (loaded by the caller). */
    stories: Story[];
    /** Whether the stories are still loading. */
    storiesLoading?: boolean;
    /** Error string if the stories failed to load. */
    storiesError?: string | null;
    /** Back to the project list. */
    onback: () => void;
    /** Open a story's detail panel. */
    onselectStory: (story: Story) => void;
    /** Company objectives used for the goal chip + KR card. */
    objectives?: Objective[];
    /**
     * Notify the caller a status persisted (US-010) so it can refresh its list.
     * Optional — the detail view persists + paints optimistically on its own.
     */
    onStatusChange?: (projectId: string, status: EditableProjectStatus) => void;
  }

  let {
    project,
    stories,
    storiesLoading = false,
    storiesError = null,
    onback,
    onselectStory,
    objectives = [],
    onStatusChange,
  }: Props = $props();

  // ---- README load (sibling of the prd.json) -------------------------------
  let readme = $state<string | null>(null);
  let readmeLoading = $state(false);
  let prd = $state<LocalProjectPrdWire | null>(null);
  let prdLoading = $state(false);

  // Reload the README whenever the open project (its prdPath) changes.
  $effect(() => {
    const prdPath = project.prdPath;
    readme = null;
    if (!prdPath) {
      readmeLoading = false;
      return;
    }
    readmeLoading = true;
    let cancelled = false;
    void (async () => {
      try {
        const content = await loadLocalProjectReadme(prdPath);
        if (!cancelled) readme = content;
      } catch (err) {
        console.error('get_local_project_readme failed:', err);
        if (!cancelled) readme = null;
      } finally {
        if (!cancelled) readmeLoading = false;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const prdPath = project.prdPath;
    prd = null;
    if (!prdPath) {
      prdLoading = false;
      return;
    }
    prdLoading = true;
    let cancelled = false;
    void (async () => {
      try {
        const content = await loadLocalProjectPrd(prdPath);
        if (!cancelled) prd = content;
      } catch (err) {
        console.error('get_local_project_prd failed:', err);
        if (!cancelled) prd = null;
      } finally {
        if (!cancelled) prdLoading = false;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  const hasPrd = $derived(Boolean(project.prdPath));
  const hasReadme = $derived(readme !== null && readme.trim() !== '');
  const readmeHtml = $derived(hasReadme ? renderMarkdown(readme as string) : '');

  const progress = $derived(
    projectProgress(project.storiesComplete, project.storiesTotal),
  );

  // ---- KPI strip (Paper editorial) -----------------------------------------
  // Computed from the loaded `stories` via classifyStories: the headline STORIES
  // tile shows complete/total + a progress bar; the others are live state counts.
  const classified = $derived(classifyStories(stories));
  const linkedGoal = $derived(findLinkedGoal(project, objectives));
  const keyResults = $derived(linkedGoal?.keyResults ?? []);
  const prdDescription = $derived(prd?.description?.trim() || project.description);
  const taskRail = $derived.by(() => {
    const sections = [
      { label: 'In progress', count: 0 },
      { label: 'Blocked', count: 0 },
      { label: 'To do', count: 0 },
      { label: 'Done', count: 0 },
    ];
    for (const item of classified) {
      if (item.state === 'in-progress') sections[0].count += 1;
      else if (item.state === 'blocked') sections[1].count += 1;
      else if (item.state === 'complete') sections[3].count += 1;
      else sections[2].count += 1;
    }
    return sections;
  });
  const kpi = $derived.by(() => {
    let inProgress = 0;
    let blocked = 0;
    let complete = 0;
    for (const item of classified) {
      if (item.state === 'in-progress') inProgress += 1;
      else if (item.state === 'blocked') blocked += 1;
      else if (item.state === 'complete') complete += 1;
    }
    const total = stories.length;
    return {
      inProgress,
      blocked,
      complete,
      total,
      progress: projectProgress(complete, total),
    };
  });

  // ---- status control (WRITABLE — US-010 optimistic persist) ---------------
  // The rendered status is a local override so it can update optimistically on
  // select and roll back on a failed write. It re-syncs whenever the open
  // project (or its raw status) changes — i.e. when a different project mounts.
  let statusOverride = $state<EditableProjectStatus | null>(null);
  $effect(() => {
    // Track the project's raw status; reset the local override when it changes.
    void project.id;
    void project.status;
    statusOverride = null;
  });
  const currentStatus = $derived(
    statusOverride ?? toEditableStatus(project.status),
  );
  let statusOpen = $state(false);
  let statusError = $state<string | null>(null);
  let statusSaving = $state(false);

  async function selectStatus(next: EditableProjectStatus) {
    statusOpen = false;
    const previous = currentStatus;
    if (next === previous) return;

    // Optimistic paint: show the new status immediately, clear any prior error.
    statusOverride = next;
    statusError = null;
    statusSaving = true;
    try {
      const result = await setProjectStatus(
        { id: project.id, company: project.company },
        previous,
        next,
      );
      if (result.ok) {
        onStatusChange?.(project.id, next);
      } else {
        // Roll back to the prior value and surface the error.
        statusOverride = previous;
        statusError = result.error;
      }
    } finally {
      statusSaving = false;
    }
  }

  // ---- Overview / Board tab ------------------------------------------------
  // Default to the Board (stories) view so opening a project shows its stories
  // immediately; Overview still renders the README on demand.
  type Tab = 'overview' | 'board';
  let tab = $state<Tab>('board');

  // Closing the status dropdown on outside click.
  function onDocClick(event: MouseEvent) {
    const target = event.target as HTMLElement | null;
    if (target && !target.closest('[data-status-control]')) {
      statusOpen = false;
    }
  }

  onMount(() => {
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  });

  function normalizeLink(value: string | null | undefined): string {
    return (value ?? '').toLowerCase().replace(/[^a-z0-9]+/g, '');
  }

  function projectTokensForGoal(value: Project): string[] {
    return [
      value.id,
      value.name,
      value.title,
      value.prdPath.split('/').filter(Boolean).at(-2),
    ]
      .map(normalizeLink)
      .filter(Boolean);
  }

  function findLinkedGoal(value: Project, goals: Objective[]): Objective | null {
    const projectTokens = new Set(projectTokensForGoal(value));
    return (
      goals.find((goal) => {
        const goalTokens = [
          goal.id,
          goal.linearInitiativeId ?? undefined,
          ...(goal.initiativeIds ?? []),
        ]
          .map(normalizeLink)
          .filter(Boolean);
        return goalTokens.some((token) => projectTokens.has(token));
      }) ?? null
    );
  }

  function formatKrValue(value: unknown, unit?: string): string {
    if (value === null || value === undefined || value === '') return '—';
    return `${value}${unit ?? ''}`;
  }

  function krProgress(current: unknown, target: unknown): number {
    const currentNum = Number(current);
    const targetNum = Number(target);
    if (!Number.isFinite(currentNum) || !Number.isFinite(targetNum) || targetNum <= 0) return 0;
    return Math.max(0, Math.min(100, Math.round((currentNum / targetNum) * 100)));
  }
</script>

<section
  class="project-detail"
  aria-labelledby="project-detail-title"
  data-testid="project-detail-view"
>
  <header class="detail-header">
    <!-- Breadcrumb: "‹ Board" is the back affordance; project name follows. -->
    <nav class="breadcrumb" aria-label="Breadcrumb">
      <button
        type="button"
        class="back-button"
        data-testid="detail-back"
        onclick={onback}
      >
        <span class="back-chevron" aria-hidden="true">‹</span>
        <span>Board</span>
      </button>
      <span class="crumb-sep" aria-hidden="true">/</span>
      <span class="crumb-current">{projectDisplayName(project)}</span>
    </nav>

    <h1 id="project-detail-title">{projectDisplayName(project)}</h1>
    {#if project.description}
      <p class="detail-description">{project.description}</p>
    {/if}

    <!-- Status row: writable status pill, then company badge + indicators. -->
    <div class="status-row">
      <!-- Status control (writable; US-010 persists with optimistic UI). -->
      <div class="status-control" data-status-control data-testid="status-control">
        <button
          type="button"
          class="status-badge status-{currentStatus}"
          data-testid="status-trigger"
          aria-haspopup="listbox"
          aria-expanded={statusOpen}
          disabled={statusSaving}
          onclick={() => (statusOpen = !statusOpen)}
        >
          <span class="status-dot" aria-hidden="true"></span>
          <span>{EDITABLE_PROJECT_STATUS_LABEL[currentStatus]}</span>
          <span class="status-caret" aria-hidden="true">⌄</span>
        </button>
        {#if statusOpen}
          <ul class="status-menu" role="listbox" data-testid="status-menu">
            {#each EDITABLE_PROJECT_STATUSES as status (status)}
              <li>
                <button
                  type="button"
                  class="status-option"
                  role="option"
                  aria-selected={status === currentStatus}
                  data-testid="status-option-{status}"
                  onclick={() => selectStatus(status)}
                >
                  <span class="status-dot status-{status}" aria-hidden="true"></span>
                  <span>{EDITABLE_PROJECT_STATUS_LABEL[status]}</span>
                  {#if status === currentStatus}
                    <span class="status-current">current</span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>

      {#if statusError}
        <span class="status-error" role="alert" data-testid="status-error">
          {statusError}
        </span>
      {/if}

      {#if project.company}
        <span class="badge company-badge" data-testid="company-badge">
          <span class="status-dot" aria-hidden="true"></span>
          {project.company}
        </span>
      {/if}

      {#if hasPrd}
        <span class="indicator" data-testid="indicator-prd">
          <span aria-hidden="true">▤</span> PRD
        </span>
      {/if}
      {#if hasReadme}
        <span class="indicator" data-testid="indicator-readme">
          <span aria-hidden="true">▦</span> README
        </span>
      {/if}
      {#if linkedGoal}
        <span class="indicator goal-indicator" data-testid="detail-goal-chip">
          <span aria-hidden="true">◎</span> {linkedGoal.title}
        </span>
      {/if}
    </div>

    <!-- KPI strip — editorial stat tiles computed from the loaded stories. -->
    {#if hasPrd && kpi.total > 0}
      <div class="kpi-strip" aria-label="Project metrics">
        <div class="kpi-tile kpi-stories">
          <span class="kpi-label">Stories</span>
          <span class="kpi-value">{kpi.complete}<span class="kpi-slash"> / </span>{kpi.total}</span>
          <span class="kpi-bar" data-testid="detail-progress">
            <span class="kpi-bar-fill" style="width: {kpi.progress.percent}%"></span>
          </span>
        </div>
        <div class="kpi-tile">
          <span class="kpi-label">In Progress</span>
          <span class="kpi-value" class:is-zero={kpi.inProgress === 0}>{kpi.inProgress}</span>
        </div>
        <div class="kpi-tile">
          <span class="kpi-label">Blocked</span>
          <span
            class="kpi-value"
            class:is-zero={kpi.blocked === 0}
            class:is-blocked={kpi.blocked > 0}>{kpi.blocked}</span
          >
        </div>
        <div class="kpi-tile">
          <span class="kpi-label">Done</span>
          <span
            class="kpi-value"
            class:is-zero={kpi.complete === 0}
            class:is-done={kpi.complete > 0}>{kpi.complete}</span
          >
        </div>
      </div>
    {/if}

    <nav class="tabs" aria-label="Project sections">
      <button
        type="button"
        class="tab"
        class:active={tab === 'overview'}
        data-testid="tab-overview"
        onclick={() => (tab = 'overview')}
      >
        Overview
      </button>
      <button
        type="button"
        class="tab"
        class:active={tab === 'board'}
        data-testid="tab-board"
        onclick={() => (tab = 'board')}
      >
        Board
      </button>
    </nav>
  </header>

  <div class="detail-body">
    {#if tab === 'overview'}
      <div class="overview detail-layout" data-testid="detail-overview">
        <main class="detail-main">
          <section class="info-card prd-card" data-testid="detail-prd-card">
            <h2>PRD</h2>
            {#if prdLoading}
              <p class="muted-note">Loading PRD...</p>
            {:else}
              <p>{prdDescription || 'No PRD summary yet.'}</p>
              <dl class="info-list">
                <div>
                  <dt>Stories</dt>
                  <dd>{kpi.complete}/{kpi.total}</dd>
                </div>
                <div>
                  <dt>Branch</dt>
                  <dd>{prd?.branchName ?? 'not set'}</dd>
                </div>
              </dl>
            {/if}
          </section>

          {#if linkedGoal}
            <section class="info-card key-results-card" data-testid="detail-key-results">
              <h2>Key results</h2>
              <p class="goal-line">{linkedGoal.title}</p>
              {#if keyResults.length === 0}
                <p class="muted-note">No key results yet.</p>
              {:else}
                <div class="kr-list">
                  {#each keyResults as kr, index (kr.id || index)}
                    {@const progressValue = krProgress(kr.current, kr.target)}
                    <div class="kr-row">
                      <span>{kr.title || kr.metric || 'Key result'}</span>
                      <strong>{formatKrValue(kr.current, kr.unit)} -> {formatKrValue(kr.target, kr.unit)}</strong>
                      <span class="mini-track" aria-label={`${progressValue}% progress`}>
                        <span style={`width: ${progressValue}%`}></span>
                      </span>
                    </div>
                  {/each}
                </div>
              {/if}
            </section>
          {/if}

          {#if readmeLoading}
            <p class="muted-note">Loading README...</p>
          {:else if hasReadme}
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            <article
              class="markdown-body"
              data-testid="readme-markdown"
            >{@html readmeHtml}</article>
          {:else if project.description}
            <section class="info-card">
              <h2>Description</h2>
              <p>{project.description}</p>
            </section>
          {/if}
        </main>

        <aside class="task-rail" data-testid="detail-task-rail">
          <h2>Tasks roll-up</h2>
          {#each taskRail as item (item.label)}
            <div class="rail-row">
              <span>{item.label}</span>
              <strong>{item.count}</strong>
            </div>
          {/each}
        </aside>
      </div>
    {:else}
      <div class="board-tab" data-testid="detail-board">
        {#if storiesError}
          <div class="drill-error" role="alert">{storiesError}</div>
        {:else if !hasPrd}
          <div class="drill-empty">
            <p>This project has no linked PRD yet, so there are no stories to show.</p>
          </div>
        {:else}
          <StoryKanban {stories} loading={storiesLoading} onselect={onselectStory} />
        {/if}
      </div>
    {/if}
  </div>
</section>

<style>
  .project-detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    height: 100%;
  }

  .detail-header {
    flex-shrink: 0;
    min-width: 0;
  }

  /* ---- Breadcrumb ------------------------------------------------------- */
  .breadcrumb {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-bottom: var(--space-3);
    margin-left: calc(-1 * var(--space-2));
    min-width: 0;
    font-size: var(--text-sm);
  }

  .back-button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted-2);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .back-button:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  .back-button:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .back-chevron {
    font-size: var(--text-base);
    line-height: 1;
  }

  .crumb-sep {
    color: var(--muted-3);
  }

  .crumb-current {
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ---- Editorial title -------------------------------------------------- */
  #project-detail-title {
    margin: 0;
    color: var(--fg);
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 600;
    letter-spacing: -0.02em;
    line-height: 1.15;
  }

  .detail-description {
    margin: var(--space-2) 0 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  /* ---- Status row ------------------------------------------------------- */
  .status-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .badge,
  .status-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-raise);
    color: var(--muted-2);
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .status-control {
    position: relative;
  }

  .status-badge {
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease;
  }

  .status-badge:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .status-badge:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .status-caret {
    color: var(--muted-3);
    font-size: var(--text-sm);
    line-height: 1;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--muted-2);
  }

  /* Status accent dots — token-driven, one hue per state. */
  .status-planned .status-dot,
  .status-dot.status-planned {
    background: var(--muted-2);
  }
  .status-prd_created .status-dot,
  .status-dot.status-prd_created {
    background: var(--blue);
  }
  .status-in_progress .status-dot,
  .status-dot.status-in_progress {
    background: var(--amber);
  }
  .status-completed .status-dot,
  .status-dot.status-completed {
    background: var(--emerald);
  }
  .status-archived .status-dot,
  .status-dot.status-archived {
    background: var(--muted-3);
  }

  .status-menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    left: 0;
    z-index: 50;
    min-width: 160px;
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.32);
  }

  .status-option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-base);
    text-align: left;
    cursor: pointer;
  }

  .status-option:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  .status-option[aria-selected='true'] {
    color: var(--fg);
  }

  .status-current {
    margin-left: auto;
    color: var(--muted-3);
    font-size: var(--text-base);
  }

  .status-badge:disabled {
    cursor: progress;
    opacity: 0.6;
  }

  .status-error {
    display: inline-flex;
    align-items: center;
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-raise);
    color: var(--amber);
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .indicator {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--muted-3);
    font-size: var(--text-sm);
  }

  /* ---- KPI strip -------------------------------------------------------- */
  .kpi-strip {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-4);
    max-width: 760px;
  }

  .kpi-tile {
    display: flex;
    flex: 1 1 130px;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    padding: 11px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-raise);
  }

  .kpi-label {
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .kpi-value {
    color: var(--fg);
    font-family: var(--font-display);
    font-size: var(--text-kpi);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }

  .kpi-value.is-zero {
    color: var(--muted-3);
  }

  .kpi-value.is-blocked {
    color: var(--amber);
  }

  .kpi-value.is-done {
    color: var(--emerald);
  }

  .kpi-slash {
    color: var(--muted-3);
    font-weight: 600;
  }

  .kpi-bar {
    display: block;
    width: 100%;
    height: 4px;
    margin-top: var(--space-1);
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-active);
  }

  .kpi-bar-fill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--emerald);
    transition: width 180ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  @media (prefers-reduced-motion: reduce) {
    .kpi-bar-fill {
      transition: none;
    }
  }

  .tabs {
    display: inline-flex;
    gap: var(--space-1);
    margin-top: var(--space-4);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
  }

  .tab {
    padding: var(--space-1) var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .tab:hover {
    color: var(--fg);
  }

  .tab.active {
    background: var(--bg);
    color: var(--fg);
  }

  .tab:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .detail-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  .overview {
    max-width: 760px;
  }

  .muted-note {
    color: var(--muted-3);
    font-size: var(--text-base);
  }

  .no-readme {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .info-card {
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
  }

  .info-card h2 {
    margin: 0 0 var(--space-2);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .info-card p {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .info-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
  }

  .info-list div {
    display: flex;
    gap: var(--space-3);
    font-size: var(--text-base);
  }

  .info-list dt {
    flex-shrink: 0;
    width: 88px;
    color: var(--muted-3);
  }

  .info-list dd {
    margin: 0;
    color: var(--muted);
  }

  .board-tab {
    height: 100%;
    min-height: 0;
  }

  .drill-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-base);
  }

  .drill-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-md);
    color: var(--muted-3);
    font-size: var(--text-base);
  }

  .drill-empty p {
    margin: 0;
  }

  /* ---- README markdown typography (token-driven) ------------------------ */
  .markdown-body {
    color: var(--fg);
    font-size: var(--text-base);
    line-height: 1.6;
  }

  .markdown-body :global(h1),
  .markdown-body :global(h2),
  .markdown-body :global(h3),
  .markdown-body :global(h4),
  .markdown-body :global(h5),
  .markdown-body :global(h6) {
    margin: var(--space-5) 0 var(--space-2);
    color: var(--fg);
    font-weight: 600;
    line-height: 1.3;
  }

  .markdown-body :global(h1) {
    font-size: var(--text-base);
  }
  .markdown-body :global(h2) {
    padding-bottom: var(--space-1);
    border-bottom: 1px solid var(--border);
    font-size: var(--text-base);
  }
  .markdown-body :global(h3) {
    font-size: var(--text-base);
  }

  .markdown-body :global(p) {
    margin: var(--space-2) 0;
    color: var(--muted);
  }

  .markdown-body :global(ul),
  .markdown-body :global(ol) {
    margin: var(--space-2) 0;
    padding-left: var(--space-5);
    color: var(--muted);
  }

  .markdown-body :global(li) {
    margin: var(--space-1) 0;
  }

  .markdown-body :global(a) {
    color: var(--blue);
    text-decoration: none;
  }

  .markdown-body :global(a:hover) {
    text-decoration: underline;
  }

  .markdown-body :global(code) {
    padding: 1px var(--space-1);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--fg);
    font-family:
      ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-base);
  }

  .markdown-body :global(pre) {
    margin: var(--space-3) 0;
    padding: var(--space-3);
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-subtle);
  }

  .markdown-body :global(pre code) {
    padding: 0;
    background: transparent;
  }

  .markdown-body :global(blockquote) {
    margin: var(--space-3) 0;
    padding: var(--space-1) var(--space-3);
    border-left: 3px solid var(--border-strong);
    color: var(--muted-3);
  }

  .markdown-body :global(hr) {
    margin: var(--space-4) 0;
    border: 0;
    border-top: 1px solid var(--border);
  }

  .markdown-body :global(strong) {
    color: var(--fg);
    font-weight: 600;
  }

  .detail-layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 220px;
    gap: 16px;
    align-items: start;
    min-width: 0;
  }

  .detail-main {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }

  .info-card,
  .task-rail {
    min-width: 0;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-raise);
  }

  .info-card h2,
  .task-rail h2 {
    margin: 0 0 8px;
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .info-card p,
  .muted-note,
  .goal-line {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .info-list {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    margin: 12px 0 0;
  }

  .info-list div {
    min-width: 0;
  }

  .info-list dt {
    color: var(--muted-3);
    font-size: var(--text-micro);
    text-transform: uppercase;
  }

  .info-list dd {
    margin: 3px 0 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kr-list,
  .task-rail {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  .kr-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 6px 10px;
    align-items: center;
    min-width: 0;
  }

  .kr-row span:first-child,
  .rail-row span {
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kr-row strong,
  .rail-row strong {
    color: var(--fg-data);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 500;
  }

  .mini-track {
    grid-column: 1 / -1;
    height: 4px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-active);
  }

  .mini-track span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--emerald);
  }

  .rail-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
  }

  .rail-row:last-child {
    padding-bottom: 0;
    border-bottom: 0;
  }

  @media (max-width: 840px) {
    .detail-layout {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
