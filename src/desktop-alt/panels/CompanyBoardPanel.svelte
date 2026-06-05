<script lang="ts">
  /**
   * CompanyBoardPanel — the per-company Board surface (US-011).
   *
   * Replaces the old top-level cross-company BoardPage (deleted) with a board
   * scoped to ONE company/workspace. Top to bottom it shows three sections:
   *
   *   1. Goals     — the company's objectives (via get_local_company_goals),
   *                  rendered as compact monochrome-glass cards: title, a
   *                  status pill, timeframe, owner, and (only when present) a
   *                  small per-KR progress bar.
   *   2. In flight — the company's projects whose rollup is in-progress/live,
   *                  surfaced first with their current in-progress story title
   *                  (loaded lazily per project). A short "what's moving" list.
   *   3. Projects  — the full company-filtered project list (ProjectListView,
   *                  showCompany=false). Selecting a project opens the detail
   *                  view (→ StoryKanban → StoryDetailPanel) with a back
   *                  affordance, exactly like the old BoardPage flow.
   *
   * Load convention follows ActivityPanel: warm-read from the company store,
   * $effect keyed on slug with a cancel flag, error state. Drill-in selection
   * (selected project + selected story) is owned here, mirroring BoardPage —
   * when a project is open the goals/in-flight/list sections hide behind the
   * detail view, and Back returns to the board.
   */
  import {
    loadLocalProjects,
    loadLocalProjectStories,
    loadCompanyGoals,
    type Objective,
    type KeyResult,
  } from '../lib/local-projects';
  import {
    classifyStories,
    projectDisplayName,
    projectListStatus,
    type Project,
    type Story,
  } from '../lib/projects-model';
  import ProjectListView from '../components/ProjectListView.svelte';
  import ProjectDetailView from '../pages/ProjectDetailView.svelte';
  import StoryDetailPanel from '../components/StoryDetailPanel.svelte';

  interface Props {
    /** The company/workspace slug this board is scoped to. */
    slug: string;
  }

  let { slug }: Props = $props();

  // ---- data (projects + goals), scoped to `slug` ---------------------------
  let projects = $state<Project[]>([]);
  let objectives = $state<Objective[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // ---- in-flight current-story titles, loaded lazily per project -----------
  // Keyed by project id → the current in-progress story's title (or null while
  // loading / when none). Best-effort: a failed load simply omits the title and
  // the row falls back to project-level progress.
  let inFlightStory = $state<Record<string, string | null>>({});

  // ---- drill-in state (mirrors the old BoardPage) --------------------------
  let selected = $state<Project | null>(null);
  let stories = $state<Story[]>([]);
  let storiesLoading = $state(false);
  let storiesError = $state<string | null>(null);

  let selectedStoryId = $state<string | null>(null);
  const selectedStory = $derived(
    selectedStoryId === null
      ? null
      : (stories.find((story) => story.id === selectedStoryId) ?? null),
  );

  // Projects already scoped to this company (the Rust command returns ALL
  // companies; we filter by slug here, exactly as the task specifies).
  const companyProjects = $derived(
    projects.filter((project) => project.company === slug),
  );

  // In-flight projects: rollup is in-progress or live, emphasised (live first).
  const inFlightProjects = $derived(
    companyProjects
      .filter((project) => {
        const status = projectListStatus(project);
        return status === 'live' || status === 'in-progress';
      })
      .sort((a, b) => {
        // live before in-progress, then by name.
        const rank = (p: Project) => (projectListStatus(p) === 'live' ? 0 : 1);
        return rank(a) - rank(b) || projectDisplayName(a).localeCompare(projectDisplayName(b));
      }),
  );

  // Load goals + projects whenever the company slug changes. Cancel-flag guards
  // against an out-of-order completion when the user switches companies fast.
  $effect(() => {
    const activeSlug = slug;
    projects = [];
    objectives = [];
    inFlightStory = {};
    error = null;
    selected = null;
    stories = [];
    storiesError = null;
    selectedStoryId = null;

    if (!activeSlug) {
      loading = false;
      return;
    }

    loading = true;
    let cancelled = false;

    void (async () => {
      try {
        const [allProjects, goals] = await Promise.all([
          loadLocalProjects(),
          loadCompanyGoals(activeSlug),
        ]);
        if (cancelled) return;
        projects = allProjects;
        objectives = goals.objectives;
      } catch (err) {
        console.error('CompanyBoardPanel load failed:', err);
        if (!cancelled) {
          error = 'Board unavailable. Try again after a sync.';
          projects = [];
          objectives = [];
        }
      } finally {
        if (!cancelled) loading = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  // Lazily load the in-flight projects' current story titles. Keyed on the set
  // of in-flight project prdPaths so it re-runs when the project set changes.
  $effect(() => {
    const targets = inFlightProjects.filter((project) => project.prdPath);
    if (targets.length === 0) return;

    let cancelled = false;
    for (const project of targets) {
      // Skip projects we've already resolved (title or explicit null).
      if (project.id in inFlightStory) continue;
      void (async () => {
        try {
          const projectStories = await loadLocalProjectStories(project.prdPath);
          if (cancelled) return;
          const current = classifyStories(projectStories).find(
            (entry) => entry.state === 'in-progress',
          );
          inFlightStory = {
            ...inFlightStory,
            [project.id]: current?.story.title ?? null,
          };
        } catch (err) {
          console.error('in-flight story load failed:', err);
          if (!cancelled) {
            inFlightStory = { ...inFlightStory, [project.id]: null };
          }
        }
      })();
    }

    return () => {
      cancelled = true;
    };
  });

  // ---- goals helpers -------------------------------------------------------

  /** Human label for an objective status (snake/space → Title Case). */
  function objectiveStatusLabel(status: string): string {
    const clean = status.replace(/[_-]+/g, ' ').trim();
    if (!clean) return 'No status';
    return clean.replace(/\b\w/g, (ch) => ch.toUpperCase());
  }

  /** Coerce a loosely-typed KR value to a finite number, or null. */
  function krNumber(value: number | string | null | undefined): number | null {
    if (value == null) return null;
    const n = typeof value === 'number' ? value : Number.parseFloat(value);
    return Number.isFinite(n) ? n : null;
  }

  /**
   * A KR's progress percent (0–100), or null when it can't be computed. Only
   * rendered when both current + target resolve to numbers and target > 0.
   */
  function krPercent(kr: KeyResult): number | null {
    const current = krNumber(kr.current);
    const target = krNumber(kr.target);
    if (current === null || target === null || target <= 0) return null;
    return Math.max(0, Math.min(100, Math.round((current / target) * 100)));
  }

  // ---- drill-in handlers (mirror BoardPage) --------------------------------

  function selectStoryById(storyId: string): void {
    if (stories.some((story) => story.id === storyId)) {
      selectedStoryId = storyId;
    }
  }

  function openStory(story: Story): void {
    selectedStoryId = story.id;
  }

  function closeStory(): void {
    selectedStoryId = null;
  }

  async function openProject(project: Project): Promise<void> {
    selected = project;
    stories = [];
    storiesError = null;
    selectedStoryId = null;

    if (!project.prdPath) {
      storiesLoading = false;
      return;
    }

    storiesLoading = true;
    try {
      stories = await loadLocalProjectStories(project.prdPath);
    } catch (err) {
      console.error('get_local_project_prd failed:', err);
      storiesError = 'Could not load this project’s stories.';
      stories = [];
    } finally {
      storiesLoading = false;
    }
  }

  function backToList(): void {
    selected = null;
    stories = [];
    storiesError = null;
    selectedStoryId = null;
  }

  // A persisted status change updates the open project + its list row so the
  // new status survives a back-navigation without a full reload.
  function onProjectStatusChange(projectId: string, status: string): void {
    if (selected && selected.id === projectId) {
      selected = { ...selected, status };
    }
    projects = projects.map((project) =>
      project.id === projectId ? { ...project, status } : project,
    );
    // The in-flight set may change; drop the cached story so it reloads.
    if (projectId in inFlightStory) {
      const next = { ...inFlightStory };
      delete next[projectId];
      inFlightStory = next;
    }
  }
</script>

<section class="company-board" aria-label="Board" data-testid="company-board-panel">
  {#if selected}
    <ProjectDetailView
      project={selected}
      {stories}
      {storiesLoading}
      {storiesError}
      onback={backToList}
      onselectStory={openStory}
      onStatusChange={onProjectStatusChange}
    />

    <StoryDetailPanel
      story={selectedStory}
      onclose={closeStory}
      onselectDependency={selectStoryById}
    />
  {:else}
    {#if error}
      <div class="board-error" role="alert">{error}</div>
    {/if}

    <!-- Goals ------------------------------------------------------------- -->
    <section class="board-section" aria-labelledby="board-goals-title">
      <header class="section-header">
        <h2 id="board-goals-title">Goals</h2>
        <span>{objectives.length} {objectives.length === 1 ? 'objective' : 'objectives'}</span>
      </header>

      {#if loading}
        <div class="goals-grid" aria-busy="true">
          {#each [0, 1] as row (row)}
            <div class="goal-skeleton"></div>
          {/each}
        </div>
      {:else if objectives.length === 0}
        <div class="empty-state">No goals yet</div>
      {:else}
        <div class="goals-grid">
          {#each objectives as objective (objective.id || objective.title)}
            <article class="goal-card" data-testid="goal-card">
              <div class="goal-head">
                <span class="goal-title">{objective.title || 'Untitled objective'}</span>
                <span class="goal-pill goal-status">
                  {objectiveStatusLabel(objective.status)}
                </span>
                {#if objective.timeframe}
                  <span class="goal-pill goal-meta">{objective.timeframe}</span>
                {/if}
                {#if objective.owner}
                  <span class="goal-pill goal-meta">{objective.owner}</span>
                {/if}
              </div>

              {#if objective.description}
                <p class="goal-desc">{objective.description}</p>
              {/if}

              {#if objective.keyResults.length > 0}
                <ul class="kr-list" data-testid="goal-key-results">
                  {#each objective.keyResults as kr, krIndex (kr.id || krIndex)}
                    <li class="kr-row">
                      <span class="kr-title">{kr.title || kr.metric || 'Key result'}</span>
                      {#if krPercent(kr) !== null}
                        <span class="kr-track" aria-hidden="true">
                          <span class="kr-fill" style={`width: ${krPercent(kr)}%`}></span>
                        </span>
                        <span class="kr-percent">{krPercent(kr)}%</span>
                      {/if}
                    </li>
                  {/each}
                </ul>
              {/if}
            </article>
          {/each}
        </div>
      {/if}
    </section>

    <!-- In flight --------------------------------------------------------- -->
    <section class="board-section" aria-labelledby="board-inflight-title">
      <header class="section-header">
        <h2 id="board-inflight-title">In flight</h2>
        <span>{inFlightProjects.length} active</span>
      </header>

      {#if loading}
        <div class="inflight-list" aria-busy="true">
          {#each [0, 1] as row (row)}
            <div class="inflight-skeleton"></div>
          {/each}
        </div>
      {:else if inFlightProjects.length === 0}
        <div class="empty-state">Nothing in flight</div>
      {:else}
        <div class="inflight-list" data-testid="inflight-list">
          {#each inFlightProjects as project (project.id)}
            <button
              type="button"
              class="inflight-row"
              class:is-live={projectListStatus(project) === 'live'}
              data-testid="inflight-row"
              onclick={() => openProject(project)}
            >
              <span class="inflight-dot" aria-hidden="true"></span>
              <span class="inflight-body">
                <span class="inflight-title">{projectDisplayName(project)}</span>
                <span class="inflight-current">
                  {#if inFlightStory[project.id]}
                    {inFlightStory[project.id]}
                  {:else}
                    {project.storiesComplete}/{project.storiesTotal} stories
                  {/if}
                </span>
              </span>
              {#if projectListStatus(project) === 'live'}
                <span class="inflight-badge">Running</span>
              {/if}
            </button>
          {/each}
        </div>
      {/if}
    </section>

    <!-- Projects ---------------------------------------------------------- -->
    <section class="board-section projects-section" aria-labelledby="board-projects-title">
      <header class="section-header">
        <h2 id="board-projects-title">Projects</h2>
        <span>{companyProjects.length} {companyProjects.length === 1 ? 'project' : 'projects'}</span>
      </header>

      <div class="projects-wrap">
        <ProjectListView
          projects={companyProjects}
          {loading}
          onselect={openProject}
        />
      </div>
    </section>
  {/if}
</section>

<style>
  .company-board {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
    height: 100%;
  }

  .board-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-base);
  }

  .board-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .projects-section {
    flex: 1 1 auto;
    min-height: 0;
  }

  .section-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    min-width: 0;
  }

  .section-header h2 {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 22px;
  }

  .section-header span {
    flex: 0 0 auto;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  .empty-state {
    padding: var(--space-4);
    border: 1px dashed var(--border);
    border-radius: var(--radius-md);
    color: var(--muted-3);
    font-size: var(--text-base);
    text-align: center;
  }

  /* ---- Goals -------------------------------------------------------------- */
  .goals-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-3);
    min-width: 0;
  }

  .goal-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
  }

  .goal-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .goal-title {
    min-width: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 18px;
  }

  .goal-pill {
    display: inline-flex;
    align-items: center;
    flex: 0 0 auto;
    padding: 1px var(--space-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--row-hover);
    color: var(--muted-2);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 16px;
  }

  .goal-status {
    color: var(--fg);
  }

  .goal-meta {
    color: var(--muted-3);
    font-weight: 500;
  }

  .goal-desc {
    margin: 0;
    overflow-wrap: anywhere;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  .kr-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .kr-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 64px auto;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .kr-title {
    min-width: 0;
    overflow: hidden;
    color: var(--muted-2);
    font-size: var(--text-base);
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kr-track {
    width: 64px;
    height: 4px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-hover);
  }

  .kr-fill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--fg);
    opacity: 0.55;
  }

  .kr-percent {
    color: var(--muted-3);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
    line-height: 16px;
  }

  /* ---- In flight --------------------------------------------------------- */
  .inflight-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .inflight-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease;
  }

  .inflight-row:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .inflight-row:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .inflight-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--muted-2);
  }

  .inflight-row.is-live .inflight-dot {
    background: var(--emerald);
  }

  .inflight-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }

  .inflight-title {
    min-width: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .inflight-current {
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .inflight-badge {
    flex: 0 0 auto;
    padding: 1px var(--space-2);
    border-radius: 999px;
    background: var(--row-hover);
    color: var(--muted-2);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 16px;
  }

  /* ---- Projects ---------------------------------------------------------- */
  .projects-wrap {
    flex: 1 1 auto;
    min-height: 0;
  }

  /* ---- Skeletons --------------------------------------------------------- */
  .goal-skeleton,
  .inflight-skeleton {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
    animation: board-skeleton-pulse 1.3s ease-in-out infinite;
  }

  .goal-skeleton {
    height: 96px;
  }

  .inflight-skeleton {
    height: 52px;
  }

  @keyframes board-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .inflight-row,
    .kr-fill {
      transition: none;
    }

    .goal-skeleton,
    .inflight-skeleton {
      animation: none;
    }
  }
</style>
