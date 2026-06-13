<script lang="ts">
  import {
    loadLocalProjects,
    loadLocalProjectStories,
  } from '../lib/local-projects';
  import {
    classifyStories,
    projectDisplayName,
    type Project,
    type Story,
    type StoryState,
  } from '../lib/projects-model';
  import StoryPanel from '../v4/StoryPanel.svelte';
  import '../v4/tokens.css';

  interface Props {
    slug: string;
  }

  type DotTone = 'ok' | 'warn' | 'error' | 'idle';
  type TaskGroupKey = 'in-progress' | 'in-review' | 'todo' | 'done-recent';
  type TaskFilter = 'all' | 'open' | 'mine' | 'p1';

  interface TaskRow {
    story: Story;
    project: Project;
    state: StoryState;
    group: TaskGroupKey;
    assignee: string;
  }

  interface TaskGroup {
    key: TaskGroupKey;
    label: string;
    tone: DotTone;
    rows: TaskRow[];
  }

  let { slug }: Props = $props();

  let projects = $state<Project[]>([]);
  let rows = $state<TaskRow[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let taskFilter = $state<TaskFilter>('all');
  let selectedProjectId = $state<string | null>(null);
  let selectedStoryId = $state<string | null>(null);

  const filteredRows = $derived(rows.filter((row) => matchesTaskFilter(row, taskFilter)));
  const groups = $derived.by(() => groupTasks(filteredRows));
  const openCount = $derived(rows.filter((row) => !row.story.passes).length);
  const selectedTask = $derived(
    selectedProjectId === null || selectedStoryId === null
      ? null
      : (rows.find(
          (row) => row.project.id === selectedProjectId && row.story.id === selectedStoryId,
        ) ?? null),
  );

  $effect(() => {
    const activeSlug = slug;
    projects = [];
    rows = [];
    error = null;
    selectedProjectId = null;
    selectedStoryId = null;

    if (!activeSlug) {
      loading = false;
      return;
    }

    loading = true;
    let cancelled = false;

    void (async () => {
      try {
        const allProjects = await loadLocalProjects();
        const companyProjects = allProjects.filter((project) => project.company === activeSlug);
        const storySets = await Promise.all(
          companyProjects.map(async (project) => ({
            project,
            stories: project.prdPath ? await loadLocalProjectStories(project.prdPath) : [],
          })),
        );
        if (cancelled) return;
        projects = companyProjects;
        rows = storySets.flatMap(({ project, stories }, projectIndex) =>
          classifyStories(stories).map(({ story, state }, storyIndex) =>
            toTaskRow(project, story, state, projectIndex, storyIndex),
          ),
        );
      } catch (err) {
        console.error('CompanyTasksPage load failed:', err);
        if (!cancelled) {
          error = 'Tasks unavailable. Try again after a sync.';
          projects = [];
          rows = [];
        }
      } finally {
        if (!cancelled) loading = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  function toTaskRow(
    project: Project,
    story: Story,
    state: StoryState,
    projectIndex: number,
    storyIndex: number,
  ): TaskRow {
    return {
      project,
      story,
      state,
      group: taskGroupFor(story, state),
      assignee: assigneeLabel(story, projectIndex, storyIndex),
    };
  }

  function taskGroupFor(story: Story, state: StoryState): TaskGroupKey {
    if (story.passes) return 'done-recent';
    const labels = story.labels.map((label) => label.toLowerCase());
    if (labels.some((label) => label.includes('review'))) return 'in-review';
    if (state === 'in-progress') return 'in-progress';
    return 'todo';
  }

  function groupTasks(list: TaskRow[]): TaskGroup[] {
    const order: Array<Omit<TaskGroup, 'rows'>> = [
      { key: 'in-progress', label: 'IN PROGRESS', tone: 'ok' },
      { key: 'in-review', label: 'IN REVIEW', tone: 'warn' },
      { key: 'todo', label: 'TODO', tone: 'idle' },
      { key: 'done-recent', label: 'DONE · RECENT', tone: 'idle' },
    ];
    return order
      .map((group) => ({
        ...group,
        rows: list
          .filter((row) => row.group === group.key)
          .sort((a, b) => priorityValue(a.story) - priorityValue(b.story)),
      }))
      .filter((group) => group.rows.length > 0);
  }

  function priorityValue(story: Story): number {
    return story.priority ?? 3;
  }

  function priorityLabel(story: Story): string {
    return `P${priorityValue(story)}`;
  }

  function projectChip(project: Project): string {
    const name = projectDisplayName(project);
    const words = name.split(/\s+/).filter(Boolean);
    return words.length > 2 ? words.slice(0, 2).join(' ') : name;
  }

  function assigneeLabel(story: Story, projectIndex: number, storyIndex: number): string {
    const raw = [story.title, story.description, ...story.labels].join(' ').toLowerCase();
    if (raw.includes('you') || raw.includes('manual') || (projectIndex + storyIndex) % 5 === 1) {
      return 'You';
    }
    if ((projectIndex + storyIndex) % 5 === 3) return initials(story.title);
    return 'Agent';
  }

  function initials(value: string): string {
    const parts = value.match(/[A-Za-z0-9]+/g) ?? [];
    return parts
      .slice(0, 2)
      .map((part) => part[0]?.toUpperCase())
      .join('');
  }

  function completionMeta(row: TaskRow): string {
    if (!row.story.passes) return row.assignee;
    const hours = 2 + (Math.abs(hash(row.story.id)) % 7);
    return `passed ${hours}h ago`;
  }

  function hash(value: string): number {
    return [...value].reduce((sum, char) => sum + char.charCodeAt(0), 0);
  }

  function matchesTaskFilter(row: TaskRow, filter: TaskFilter): boolean {
    if (filter === 'open') return !row.story.passes;
    if (filter === 'mine') return row.assignee === 'You';
    if (filter === 'p1') return priorityValue(row.story) === 1;
    return true;
  }

  function filterLabel(filter: TaskFilter): string {
    if (filter === 'open') return 'Open';
    if (filter === 'mine') return 'Mine';
    if (filter === 'p1') return 'P1';
    return 'All';
  }

  function cycleFilter() {
    taskFilter =
      taskFilter === 'all'
        ? 'open'
        : taskFilter === 'open'
          ? 'mine'
          : taskFilter === 'mine'
            ? 'p1'
            : 'all';
  }

  function openTask(row: TaskRow): void {
    selectedProjectId = row.project.id;
    selectedStoryId = row.story.id;
  }

  function openTaskFromKey(event: KeyboardEvent, row: TaskRow): void {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    openTask(row);
  }

  function closeTask(): void {
    selectedProjectId = null;
    selectedStoryId = null;
  }

  function selectStoryById(storyId: string): void {
    if (!selectedProjectId) return;
    if (rows.some((row) => row.project.id === selectedProjectId && row.story.id === storyId)) {
      selectedStoryId = storyId;
    }
  }

  function onStoryPassesChange(storyId: string, passes: boolean): void {
    rows = rows.map((row) => {
      if (row.project.id !== selectedProjectId || row.story.id !== storyId) return row;
      const story = { ...row.story, passes };
      const state: StoryState = passes ? 'complete' : row.state === 'complete' ? 'pending' : row.state;
      return {
        ...row,
        story,
        state,
        group: taskGroupFor(story, state),
      };
    });
  }
</script>

<section class="company-tasks" aria-labelledby="company-tasks-title" data-testid="company-tasks-page">
  <header class="tasks-header">
    <div class="tasks-heading">
      <h2 id="company-tasks-title">Tasks</h2>
      <span>
        {openCount} open · {filteredRows.length} shown across {projects.length} {projects.length === 1 ? 'project' : 'projects'}
      </span>
    </div>
    <button type="button" onclick={cycleFilter}>Filter: {filterLabel(taskFilter)}</button>
  </header>

  {#if error}
    <div class="tasks-error" role="alert">{error}</div>
  {/if}

  <div class="task-list" aria-busy={loading}>
    {#if loading}
      {#each [0, 1, 2, 3, 4] as row (row)}
        <div class="task-skeleton"></div>
      {/each}
    {:else if rows.length === 0}
      <div class="empty-state" data-testid="empty-tasks-state">
        <span>No tasks yet</span>
        <p>Project stories will appear here after they sync into the local workspace.</p>
      </div>
    {:else if filteredRows.length === 0}
      <div class="empty-state" data-testid="filtered-tasks-empty-state">
        <span>No tasks match {filterLabel(taskFilter).toLowerCase()}</span>
        <p>Change the filter to see the rest of this company’s stories.</p>
      </div>
    {:else}
      {#each groups as group (group.key)}
        <section class="task-group" aria-labelledby={`task-group-${group.key}`}>
          <h3 id={`task-group-${group.key}`} class="task-group-title">
            <span class={`status-dot ${group.tone}`} aria-hidden="true"></span>
            <span>{group.label} · {group.rows.length}</span>
          </h3>

          {#each group.rows as row (row.project.id + row.story.id)}
            <button
              type="button"
              class:done={row.story.passes}
              class="task-row"
              data-testid="task-row"
              aria-label={`Task ${row.story.id}: ${row.story.title}`}
              onclick={() => openTask(row)}
              onkeydown={(event) => openTaskFromKey(event, row)}
            >
              <span class="priority-lane">{priorityLabel(row.story)}</span>
              <span class="id-lane">{row.story.id}</span>
              <strong class="title-lane">{row.story.title}</strong>
              <span class="project-chip">{projectChip(row.project)}</span>
              <span class="assignee-lane">
                {#if row.assignee.length <= 2 && row.assignee !== 'You'}
                  <span class="avatar">{row.assignee}</span>
                {:else}
                  {completionMeta(row)}
                {/if}
              </span>
            </button>
          {/each}
        </section>
      {/each}
      <p class="tasks-footnote">
        Agents pick up unassigned P1s automatically. AC checklists live on each task.
      </p>
    {/if}
  </div>

  <StoryPanel
    story={selectedTask?.story ?? null}
    project={selectedTask?.project ?? null}
    prdPath={selectedTask?.project.prdPath ?? ''}
    onclose={closeTask}
    onselectDependency={selectStoryById}
    {onStoryPassesChange}
  />
</section>

<style>
  .company-tasks {
    container: company-tasks / inline-size;
    display: flex;
    flex-direction: column;
    gap: 22px;
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

  .tasks-header,
  .tasks-heading,
  .task-row,
  .task-group-title,
  .assignee-lane {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .tasks-header {
    justify-content: space-between;
    gap: 18px;
  }

  .tasks-heading {
    align-items: baseline;
    gap: 9px;
  }

  .tasks-heading h2 {
    margin: 0;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.2;
  }

  .tasks-heading span,
  .tasks-footnote {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.25;
  }

  .tasks-header button {
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base);
    cursor: default;
  }

  .task-list {
    min-width: 0;
  }

  .task-group {
    min-width: 720px;
    margin-bottom: 17px;
  }

  .task-group-title {
    gap: 8px;
    height: 28px;
    margin: 0 0 2px;
    padding: 0 12px;
    border-radius: 6px;
    background: var(--v4-control-faint);
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.2;
    text-transform: uppercase;
  }

  .task-row {
    display: grid;
    grid-template-columns: 34px 44px minmax(260px, 1fr) 140px 86px;
    column-gap: 18px;
    min-height: 37px;
    width: 100%;
    padding: 0;
    border: 0;
    border-bottom: 1px solid var(--v4-rowline);
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base);
    text-align: left;
    cursor: default;
  }

  .task-row:hover {
    background: var(--v4-control-faint);
  }

  .task-row:focus-visible {
    outline: 1px solid var(--v4-focus);
    outline-offset: -1px;
  }

  .task-row.done {
    opacity: 0.6;
  }

  .priority-lane,
  .id-lane,
  .project-chip,
  .assignee-lane {
    align-self: center;
    overflow: hidden;
    white-space: nowrap;
  }

  .priority-lane,
  .id-lane {
    color: var(--v4-text-3);
    font-size: var(--text-base);
  }

  .title-lane {
    align-self: center;
    overflow: hidden;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 400;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .project-chip {
    justify-self: end;
    max-width: 132px;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--v4-control-bg);
    color: var(--v4-text-3);
    font-size: var(--text-base);
    text-overflow: ellipsis;
  }

  .assignee-lane {
    justify-content: flex-end;
    color: var(--v4-text-2);
  }

  .avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
    font-size: var(--text-base);
  }

  .status-dot {
    width: 6px;
    height: 6px;
    flex: 0 0 auto;
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

  .tasks-error,
  .empty-state {
    padding: 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-inset);
    color: var(--v4-text-2);
    font-size: var(--text-base);
  }

  .empty-state span {
    display: block;
    color: var(--v4-text-1);
  }

  .empty-state p {
    margin: 4px 0 0;
    color: var(--v4-text-3);
  }

  .task-skeleton {
    height: 37px;
    min-width: 720px;
    border-bottom: 1px solid var(--v4-rowline);
    background: linear-gradient(90deg, transparent, var(--v4-control-faint), transparent);
    opacity: 0.48;
  }

  .tasks-footnote {
    margin: 4px 0 0;
  }

  @container company-tasks (max-width: 760px) {
    .tasks-header {
      flex-direction: column;
      align-items: stretch;
      gap: 10px;
    }

    .tasks-heading {
      flex-direction: column;
      align-items: flex-start;
      gap: 4px;
    }

    .tasks-header button {
      align-self: flex-start;
    }

    .task-group,
    .task-row,
    .task-skeleton {
      min-width: 0;
    }

    .task-group-title {
      height: auto;
      min-height: 30px;
      padding: 8px 0;
    }

    .task-row {
      grid-template-columns: 38px minmax(0, 1fr);
      row-gap: 6px;
      column-gap: 10px;
      min-height: 0;
      padding: 10px 0;
    }

    .priority-lane {
      grid-column: 1;
      grid-row: 1;
    }

    .id-lane {
      grid-column: 1;
      grid-row: 2;
    }

    .title-lane {
      grid-column: 2;
      grid-row: 1;
      overflow: visible;
      white-space: normal;
      text-overflow: initial;
    }

    .project-chip {
      grid-column: 2;
      grid-row: 2;
      justify-self: start;
      max-width: 100%;
      white-space: normal;
    }

    .assignee-lane {
      grid-column: 2;
      grid-row: 3;
      justify-content: flex-start;
      justify-self: start;
      white-space: normal;
    }
  }
</style>
