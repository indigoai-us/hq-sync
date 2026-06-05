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
  import { loadLocalProjectReadme } from '../lib/local-projects';
  import { setProjectStatus } from '../lib/projects-store.svelte';
  import { renderMarkdown } from '../lib/markdown';
  import {
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
    onStatusChange,
  }: Props = $props();

  // ---- README load (sibling of the prd.json) -------------------------------
  let readme = $state<string | null>(null);
  let readmeLoading = $state(false);

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

  const hasPrd = $derived(Boolean(project.prdPath));
  const hasReadme = $derived(readme !== null && readme.trim() !== '');
  const readmeHtml = $derived(hasReadme ? renderMarkdown(readme as string) : '');

  const progress = $derived(
    projectProgress(project.storiesComplete, project.storiesTotal),
  );

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
  type Tab = 'overview' | 'board';
  let tab = $state<Tab>('overview');

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
</script>

<section
  class="project-detail"
  aria-labelledby="project-detail-title"
  data-testid="project-detail-view"
>
  <header class="detail-header">
    <button
      type="button"
      class="back-button"
      data-testid="detail-back"
      onclick={onback}
    >
      <span class="back-chevron" aria-hidden="true">‹</span>
      <span>All Projects</span>
    </button>

    <div class="header-main">
      <div class="header-icon" aria-hidden="true">⬗</div>
      <div class="header-body">
        <h1 id="project-detail-title">{projectDisplayName(project)}</h1>
        {#if project.description}
          <p class="detail-description">{project.description}</p>
        {/if}

        <div class="badges">
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

          {#if hasPrd && progress.total > 0}
            <span class="progress" data-testid="detail-progress">
              <span class="progress-track">
                <span class="progress-fill" style="width: {progress.percent}%"></span>
              </span>
              <span class="progress-label">{progress.complete}/{progress.total}</span>
            </span>
          {/if}
        </div>
      </div>
    </div>

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
      <div class="overview" data-testid="detail-overview">
        {#if readmeLoading}
          <p class="muted-note">Loading README…</p>
        {:else if hasReadme}
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          <article
            class="markdown-body"
            data-testid="readme-markdown"
          >{@html readmeHtml}</article>
        {:else}
          <div class="no-readme" data-testid="no-readme">
            {#if project.description}
              <section class="info-card">
                <h2>Description</h2>
                <p>{project.description}</p>
              </section>
            {/if}
            <section class="info-card">
              <h2>Project Info</h2>
              <dl class="info-list">
                <div>
                  <dt>Status</dt>
                  <dd>{EDITABLE_PROJECT_STATUS_LABEL[currentStatus]}</dd>
                </div>
                {#if project.company}
                  <div>
                    <dt>Company</dt>
                    <dd>{project.company}</dd>
                  </div>
                {/if}
                <div>
                  <dt>PRD</dt>
                  <dd>{hasPrd ? 'Yes' : 'Not yet created'}</dd>
                </div>
              </dl>
            </section>
          </div>
        {/if}
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

  .back-button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-bottom: var(--space-2);
    margin-left: calc(-1 * var(--space-2));
    padding: var(--space-1) var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
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

  .header-main {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    min-width: 0;
  }

  .header-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-md);
    background: var(--row-active);
    color: var(--blue);
    font-size: var(--text-base);
  }

  .header-body {
    min-width: 0;
    flex: 1 1 auto;
  }

  .header-body h1 {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 29px;
  }

  .detail-description {
    margin: var(--space-1) 0 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .badge,
  .status-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted);
    font-size: var(--text-base);
    font-weight: 600;
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
    font-size: var(--text-base);
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
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-base);
    font-weight: 600;
  }

  .indicator {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--muted-3);
    font-size: var(--text-base);
  }

  .progress {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .progress-track {
    width: 64px;
    height: 6px;
    overflow: hidden;
    border-radius: var(--radius-sm);
    background: var(--row-active);
  }

  .progress-fill {
    display: block;
    height: 100%;
    border-radius: var(--radius-sm);
    background: var(--emerald);
  }

  .progress-label {
    color: var(--muted-3);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
  }

  .tabs {
    display: inline-flex;
    gap: var(--space-1);
    margin-top: var(--space-3);
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
    font-size: var(--text-base);
    font-weight: 600;
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
    font-size: 0.92em;
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
</style>
