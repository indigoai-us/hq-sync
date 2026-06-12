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
   *                  view (→ StoryKanban → StoryPanel) with a back
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
  } from '../lib/local-projects';
  import {
    classifyStories,
    projectDisplayName,
    projectListStatus,
    projectProgress,
    type Project,
    type Story,
    type StoryState,
  } from '../lib/projects-model';
  import { useCompanyBoard, type CompanyBoardCard } from '../lib/company-board.svelte';
  import { useCompanySummary } from '../lib/company-summary.svelte';
  import ProjectListView from '../components/ProjectListView.svelte';
  import ProjectDetailView from '../pages/ProjectDetailView.svelte';
  import GoalCard from '../v4/GoalCard.svelte';
  import StoryPanel from '../v4/StoryPanel.svelte';
  import '../v4/tokens.css';

  interface Props {
    /** The company/workspace slug this board is scoped to. */
    slug: string;
    /** False for local folders that are not cloud-backed yet. */
    cloudBacked?: boolean;
  }

  let { slug, cloudBacked = true }: Props = $props();

  interface InFlightDetail {
    storyTitle: string | null;
    priority: number | null;
    labels: string[];
    state: StoryState | null;
  }

  interface RowStatus {
    label: string;
    tone: 'ok' | 'warn' | 'error' | 'idle';
  }

  const summaryState = useCompanySummary({ slug: () => slug, enabled: () => cloudBacked });
  const boardState = useCompanyBoard({ slug: () => slug, enabled: () => cloudBacked });

  // ---- data (projects + goals), scoped to `slug` ---------------------------
  let projects = $state<Project[]>([]);
  let objectives = $state<Objective[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // ---- in-flight story details, loaded lazily per project ------------------
  // Keyed by project id. Best-effort: a failed load omits story metadata and
  // the table falls back to project-level progress.
  let inFlightStory = $state<Record<string, InFlightDetail>>({});

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

  const boardCards = $derived([
    ...boardState.board.inbox,
    ...boardState.board.doing,
    ...boardState.board.review,
    ...boardState.board.done,
  ]);

  const activeProjectCount = $derived(
    summaryState.summary.board > 0 ? summaryState.summary.board : inFlightProjects.length,
  );

  const storiesInProgress = $derived(
    boardState.board.doing.length + boardState.board.review.length || incompleteStoryCount(inFlightProjects),
  );

  const acPercent = $derived(projectsAcceptancePercent(companyProjects));
  const lastUpdated = $derived(lastUpdatedLabel(boardCards));

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
      // Skip projects we've already resolved.
      if (project.id in inFlightStory) continue;
      void (async () => {
        try {
          const projectStories = await loadLocalProjectStories(project.prdPath);
          if (cancelled) return;
          const classified = classifyStories(projectStories);
          const current =
            classified.find((entry) => entry.state === 'in-progress') ??
            classified.find((entry) => entry.state === 'blocked') ??
            classified.find((entry) => entry.state === 'pending') ??
            null;
          inFlightStory = {
            ...inFlightStory,
            [project.id]: {
              storyTitle: current?.story.title ?? null,
              priority: current?.story.priority ?? null,
              labels: current?.story.labels ?? [],
              state: current?.state ?? null,
            },
          };
        } catch (err) {
          console.error('in-flight story load failed:', err);
          if (!cancelled) {
            inFlightStory = {
              ...inFlightStory,
              [project.id]: { storyTitle: null, priority: null, labels: [], state: null },
            };
          }
        }
      })();
    }

    return () => {
      cancelled = true;
    };
  });

  // ---- overview helpers ----------------------------------------------------

  /** Coerce a loosely-typed KR value to a finite number, or null. */
  function looseNumber(value: number | string | null | undefined): number | null {
    if (value == null) return null;
    const n = typeof value === 'number' ? value : Number.parseFloat(value);
    return Number.isFinite(n) ? n : null;
  }

  function clampPercent(value: number): number {
    return Math.max(0, Math.min(100, Math.round(value)));
  }

  function normalizeId(value: string | null | undefined): string {
    return (value ?? '').toLowerCase().replace(/[^a-z0-9]+/g, '');
  }

  function objectiveIds(objective: Objective): Set<string> {
    const ids = new Set<string>();
    for (const id of objective.initiativeIds ?? []) {
      const normalized = normalizeId(id);
      if (normalized) ids.add(normalized);
    }
    const linearId = (objective as Objective & { linearInitiativeId?: string | null })
      .linearInitiativeId;
    const normalizedLinearId = normalizeId(linearId);
    if (normalizedLinearId) ids.add(normalizedLinearId);
    const ownId = normalizeId(objective.id);
    if (ownId) ids.add(ownId);
    return ids;
  }

  function projectTokens(project: Project): string[] {
    return [
      project.id,
      project.name,
      project.title,
      project.prdPath.split('/').filter(Boolean).at(-2),
    ]
      .map(normalizeId)
      .filter(Boolean);
  }

  function objectiveForProject(project: Project): Objective | null {
    const tokens = projectTokens(project);
    return (
      objectives.find((objective) => {
        const ids = objectiveIds(objective);
        return tokens.some((token) => ids.has(token));
      }) ?? null
    );
  }

  function linkedProjects(objective: Objective): Project[] {
    const ids = objectiveIds(objective);
    if (ids.size === 0) return [];
    return companyProjects.filter((project) =>
      projectTokens(project).some((token) => ids.has(token)),
    );
  }

  function incompleteStoryCount(items: Project[]): number {
    return items.reduce(
      (sum, project) => sum + Math.max(0, project.storiesTotal - project.storiesComplete),
      0,
    );
  }

  function projectsAcceptancePercent(items: Project[]): number {
    const total = items.reduce((sum, project) => sum + project.storiesTotal, 0);
    if (total === 0) return 0;
    const complete = items.reduce((sum, project) => sum + project.storiesComplete, 0);
    return clampPercent((complete / total) * 100);
  }

  function objectiveProgress(objective: Objective): number {
    const krPercents = objective.keyResults
      .map((kr) => {
        const current = looseNumber(kr.current);
        const target = looseNumber(kr.target);
        if (current === null || target === null || target <= 0) return null;
        return clampPercent((current / target) * 100);
      })
      .filter((percent): percent is number => percent !== null);
    if (krPercents.length > 0) {
      return clampPercent(
        krPercents.reduce((sum, percent) => sum + percent, 0) / krPercents.length,
      );
    }
    return projectsAcceptancePercent(linkedProjects(objective));
  }

  function goalChip(project: Project): string {
    return objectiveForProject(project)?.title || '—';
  }

  function priorityLabel(priority: number | null | undefined): string {
    return priority === null || priority === undefined ? '—' : `P${priority}`;
  }

  function rowStatus(project: Project, detail: InFlightDetail | undefined): RowStatus {
    const raw = (project.status ?? '').toLowerCase();
    if (detail?.state === 'blocked' || raw.includes('gated') || raw.includes('blocked')) {
      return { label: 'Gated', tone: 'idle' };
    }
    if (raw.includes('review')) {
      return { label: 'Review', tone: 'warn' };
    }
    const status = projectListStatus(project);
    if (status === 'live') return { label: 'Running', tone: 'ok' };
    if (status === 'in-progress') return { label: 'Review', tone: 'warn' };
    return { label: 'Gated', tone: 'idle' };
  }

  function lastUpdatedLabel(cards: CompanyBoardCard[]): string {
    const age = cards.map((card) => card.age).find((value): value is string => Boolean(value));
    if (age) return age;
    return cards.length > 0 ? 'just now' : '—';
  }

  function projectMeta(project: Project, detail: InFlightDetail | undefined): string {
    return detail?.storyTitle || `${project.storiesComplete}/${project.storiesTotal} stories`;
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

  function onStoryPassesChange(storyId: string, passes: boolean): void {
    stories = stories.map((story) =>
      story.id === storyId ? { ...story, passes } : story,
    );
    if (selected) {
      const nextComplete = stories.filter((story) =>
        story.id === storyId ? passes : story.passes,
      ).length;
      selected = { ...selected, storiesComplete: nextComplete };
      projects = projects.map((project) =>
        project.id === selected?.id ? { ...project, storiesComplete: nextComplete } : project,
      );
    }
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
      // Surface the underlying error, not just a generic line — the real cause
      // (feature-gate rejection, a stale prdPath, or an unresolved HQ folder) is
      // otherwise hidden, making this undiagnosable from the UI alone.
      const detail = err instanceof Error ? err.message : String(err);
      storiesError = `Could not load this project’s stories — ${detail}`;
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
      {objectives}
      onback={backToList}
      onselectStory={openStory}
      onStatusChange={onProjectStatusChange}
    />

    <StoryPanel
      story={selectedStory}
      project={selected}
      prdPath={selected.prdPath}
      onclose={closeStory}
      onselectDependency={selectStoryById}
      {onStoryPassesChange}
    />
  {:else}
    {#if !cloudBacked}
      <div class="board-note" role="status">
        This company is local only. Local goals and projects are available; synced board activity appears after it is connected.
      </div>
    {/if}
    {#if error}
      <div class="board-error" role="alert">{error}</div>
    {/if}
    <section class="stat-strip" aria-label="Company overview stats">
      <div class="stat-item">
        <strong>{activeProjectCount}</strong>
        <span>active projects</span>
      </div>
      <div class="stat-item">
        <strong>{storiesInProgress}</strong>
        <span>stories in progress</span>
      </div>
      <div class="stat-item">
        <strong>{acPercent}%</strong>
        <span>AC passing</span>
      </div>
      <div class="stat-item">
        <strong>{lastUpdated}</strong>
        <span>last updated</span>
      </div>
    </section>

    <section class="overview-section" aria-labelledby="board-goals-title">
      <header class="section-header">
        <h2 id="board-goals-title">GOALS</h2>
        <span>{objectives.length} {objectives.length === 1 ? 'goal' : 'goals'}</span>
      </header>
      {#if loading}
        <div class="goals-grid" aria-busy="true">
          {#each [0, 1] as row (row)}
            <div class="goal-skeleton"></div>
          {/each}
        </div>
      {:else if objectives.length === 0}
        <div class="empty-state" data-testid="empty-goals-state">
          <span>No goals yet</span>
          <p>Company goals will appear here after the next board sync.</p>
        </div>
      {:else}
        <div class="goals-grid">
          {#each objectives as objective (objective.id || objective.title)}
            <GoalCard
              {objective}
              progress={objectiveProgress(objective)}
              projectCount={linkedProjects(objective).length}
              storyCount={incompleteStoryCount(linkedProjects(objective))}
            />
          {/each}
        </div>
      {/if}
    </section>

    <section class="overview-section" aria-labelledby="board-inflight-title">
      <header class="section-header">
        <h2 id="board-inflight-title">IN FLIGHT</h2>
      </header>

      {#if loading}
        <div class="inflight-table skeleton-table" aria-busy="true">
          {#each [0, 1] as row (row)}
            <div class="inflight-skeleton"></div>
          {/each}
        </div>
      {:else if inFlightProjects.length === 0}
        <div class="empty-state">Nothing in flight</div>
      {:else}
        <table class="inflight-table" data-testid="inflight-list">
          <thead>
            <tr>
              <th>Story</th>
              <th>Labels</th>
              <th>Goal</th>
              <th>Priority</th>
              <th>AC</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            {#each inFlightProjects as project (project.id)}
              {@const detail = inFlightStory[project.id]}
              {@const progress = projectProgress(project.storiesComplete, project.storiesTotal)}
              {@const status = rowStatus(project, detail)}
              <tr data-testid="inflight-row">
                <td class="story-cell">
                  <button type="button" class="story-button" onclick={() => openProject(project)}>
                    <span>{projectDisplayName(project)}</span>
                    <small>{projectMeta(project, detail)}</small>
                  </button>
                </td>
                <td>
                  <span class="labels-cell">
                    {#if detail?.labels.length}
                      {#each detail.labels.slice(0, 2) as label (label)}
                        <span class="label-chip">{label}</span>
                      {/each}
                    {:else}
                      <span class="muted">—</span>
                    {/if}
                  </span>
                </td>
                <td>
                  <span class="goal-chip" data-testid="inflight-goal-chip">{goalChip(project)}</span>
                </td>
                <td class="priority-cell">{priorityLabel(detail?.priority)}</td>
                <td>
                  <span class="ac-cell">
                    <span class="ac-track" aria-hidden="true">
                      <span class="ac-fill" style={`width: ${progress.percent}%`}></span>
                    </span>
                    <span>{progress.complete}/{progress.total}</span>
                  </span>
                </td>
                <td>
                  <span class="status-cell">
                    <span class={`status-dot ${status.tone}`} aria-hidden="true"></span>
                    <span>{status.label}</span>
                  </span>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    {#if false}
      <div aria-hidden="true">
        <ProjectListView
          projects={companyProjects}
          {loading}
          onselect={openProject}
        />
      </div>
    {/if}
  {/if}
</section>

<style>
  .company-board {
    display: flex;
    flex-direction: column;
    gap: 18px;
    min-width: 0;
    height: 100%;
    color: var(--v4-text-1);
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .board-note,
  .board-error {
    padding: 10px 12px;
    border: 1px solid rgba(254, 188, 46, 0.3);
    border-radius: 6px;
    background: var(--v4-inset);
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 400;
    line-height: 1.35;
  }

  .board-note {
    border-color: var(--v4-hairline);
  }

  .stat-strip {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    overflow: hidden;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
  }

  .stat-item {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
    padding: 13px 20px;
  }

  .stat-item + .stat-item {
    border-left: 1px solid var(--v4-rowline);
  }

  .stat-item strong {
    overflow: hidden;
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 400;
    line-height: 1.15;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .stat-item span {
    overflow: hidden;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.2;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .overview-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
  }

  .section-header h2 {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.25;
    letter-spacing: 0;
  }

  .section-header span {
    flex: 0 0 auto;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.25;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 20px;
    border: 1px dashed var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    color: var(--v4-text-3);
    font-size: 13px;
    font-weight: 400;
    line-height: 1.35;
    text-align: center;
  }

  .empty-state span {
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 500;
  }

  .empty-state p {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
  }

  .goals-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px;
    min-width: 0;
  }

  .inflight-table {
    width: 100%;
    min-width: 760px;
    border-collapse: collapse;
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 400;
    table-layout: fixed;
  }

  .inflight-table th {
    padding: 0 0 9px;
    border-bottom: 1px solid var(--v4-rowline);
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.2;
    text-align: left;
  }

  .inflight-table th:nth-child(1) {
    width: 34%;
  }

  .inflight-table th:nth-child(2) {
    width: 22%;
  }

  .inflight-table th:nth-child(3) {
    width: 15%;
  }

  .inflight-table th:nth-child(4) {
    width: 8%;
  }

  .inflight-table th:nth-child(5) {
    width: 16%;
  }

  .inflight-table th:nth-child(6) {
    width: 12%;
  }

  .inflight-table td {
    min-width: 0;
    height: 58px;
    padding: 9px 16px 9px 0;
    border-bottom: 1px solid var(--v4-rowline);
    vertical-align: middle;
  }

  .story-button {
    display: flex;
    width: 100%;
    min-width: 0;
    flex-direction: column;
    gap: 3px;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .story-button:hover span {
    color: var(--v4-text-1);
  }

  .story-button:focus-visible {
    outline: 1px solid var(--v4-control-border);
    outline-offset: 3px;
  }

  .story-button span,
  .story-button small {
    overflow: hidden;
    max-width: 100%;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .story-button span {
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 500;
    line-height: 1.25;
  }

  .story-button small {
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.2;
  }

  .labels-cell {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .label-chip,
  .goal-chip {
    display: inline-flex;
    max-width: 100%;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    overflow: hidden;
    border-radius: 4px;
    background: var(--v4-control-faint);
    color: var(--v4-text-2);
    font-size: 11px;
    font-weight: 400;
    line-height: 1;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .muted,
  .priority-cell {
    color: var(--v4-text-2);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.2;
  }

  .ac-cell {
    display: grid;
    grid-template-columns: minmax(44px, 72px) auto;
    align-items: center;
    gap: 10px;
    color: var(--v4-text-2);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.2;
    font-variant-numeric: tabular-nums;
  }

  .ac-track {
    height: 3px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--v4-control-faint);
  }

  .ac-fill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--v4-text-2);
  }

  .status-cell {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 400;
    line-height: 1.2;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .status-dot.ok {
    background: var(--v4-ok);
  }

  .status-dot.warn {
    background: var(--v4-warn);
  }

  .status-dot.error {
    background: var(--v4-error);
  }

  .status-dot.idle {
    background: var(--v4-idle);
  }

  .goal-skeleton,
  .inflight-skeleton {
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    animation: board-skeleton-pulse 1.3s ease-in-out infinite;
  }

  .goal-skeleton {
    height: 96px;
  }

  .inflight-skeleton {
    height: 58px;
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
    .goal-skeleton,
    .inflight-skeleton {
      animation: none;
    }
  }

  @media (max-width: 980px) {
    .stat-strip {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .stat-item:nth-child(3) {
      border-left: 0;
    }

    .stat-item:nth-child(n + 3) {
      border-top: 1px solid var(--v4-rowline);
    }

    .goals-grid {
      grid-template-columns: 1fr;
    }

    .overview-section {
      overflow-x: auto;
    }
  }
</style>
