<script lang="ts">
  /**
   * BoardPage — the top-level Board surface (US-007).
   *
   * Lists every local project across companies (via US-003's get_local_projects,
   * adapted in local-projects.ts) through ProjectListView, and drills into a
   * single project's story Kanban (US-006 StoryKanban) on click — loading that
   * project's prd.json via get_local_project_prd and classifying with US-004.
   *
   * Company pre-filter (AC4): the desktop window has no dedicated "entered in a
   * company context" signal — the only backend-queued route hint is `meetings`.
   * So pre-filter is best-effort: when the Board route carries a `companySlug`
   * (e.g. a future company → Board hand-off), the list is scoped to it; with no
   * such signal we default to all companies and note it in the subtitle.
   */
  import { onMount } from 'svelte';
  import {
    loadLocalProjects,
    loadLocalProjectStories,
  } from '../lib/local-projects';
  import {
    type Project,
    type Story,
  } from '../lib/projects-model';
  import ProjectListView from '../components/ProjectListView.svelte';
  import ProjectDetailView from './ProjectDetailView.svelte';
  import StoryDetailPanel from '../components/StoryDetailPanel.svelte';

  interface Props {
    /** Best-effort company pre-filter — scopes the list to one company slug. */
    companySlug?: string | null;
  }

  let { companySlug = null }: Props = $props();

  let projects = $state<Project[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Drill-in state. `selected` is the open project; its stories load lazily.
  let selected = $state<Project | null>(null);
  let stories = $state<Story[]>([]);
  let storiesLoading = $state(false);
  let storiesError = $state<string | null>(null);

  // Story detail slide-over (US-008). `selectedStoryId` is the open story within
  // the current project; we key off the id (not the object) so a dependency-chip
  // click can reselect by id and so the panel survives a stories refresh.
  let selectedStoryId = $state<string | null>(null);
  const selectedStory = $derived(
    selectedStoryId === null
      ? null
      : (stories.find((story) => story.id === selectedStoryId) ?? null),
  );

  function openStory(story: Story): void {
    selectedStoryId = story.id;
  }

  function closeStory(): void {
    selectedStoryId = null;
  }

  // Dependency-chip click: reselect that dependency story if it exists in this
  // project; if the id isn't present, leave the current selection untouched.
  function selectStoryById(storyId: string): void {
    if (stories.some((story) => story.id === storyId)) {
      selectedStoryId = storyId;
    }
  }

  // Apply the best-effort company pre-filter. If the slug matches no project we
  // fall back to showing everything rather than an empty board.
  const visibleProjects = $derived.by(() => {
    if (!companySlug) return projects;
    const scoped = projects.filter((project) => project.company === companySlug);
    return scoped.length > 0 ? scoped : projects;
  });

  const prefilterActive = $derived(
    Boolean(companySlug) &&
      projects.some((project) => project.company === companySlug),
  );

  const subtitle = $derived.by(() => {
    if (loading) return 'Scanning projects…';
    const count = visibleProjects.length;
    const noun = count === 1 ? 'project' : 'projects';
    if (prefilterActive) return `${count} ${noun} in ${companySlug}`;
    return `${count} ${noun} across all companies`;
  });

  async function loadProjects() {
    loading = true;
    error = null;
    try {
      projects = await loadLocalProjects();
    } catch (err) {
      console.error('get_local_projects failed:', err);
      error = 'Projects unavailable. Try again after a sync.';
      projects = [];
    } finally {
      loading = false;
    }
  }

  async function openProject(project: Project) {
    selected = project;
    stories = [];
    storiesError = null;
    selectedStoryId = null;

    if (!project.prdPath) {
      // A board project with no linked prd has no stories to drill into.
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

  function backToList() {
    selected = null;
    stories = [];
    storiesError = null;
    selectedStoryId = null;
  }

  // US-010: a persisted status change updates the open project + its list row so
  // the new status survives a back-navigation without a full reload.
  function onProjectStatusChange(projectId: string, status: string) {
    if (selected && selected.id === projectId) {
      selected = { ...selected, status };
    }
    projects = projects.map((project) =>
      project.id === projectId ? { ...project, status } : project,
    );
  }

  onMount(() => {
    void loadProjects();
  });
</script>

<section class="board-page" aria-labelledby="board-page-title" aria-label="Board">
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
    <header class="page-header">
      <h1 id="board-page-title">Board</h1>
      <p class="page-subtitle">{subtitle}</p>
    </header>

    <div class="board-list-wrap">
      <ProjectListView
        projects={visibleProjects}
        {loading}
        {error}
        onselect={openProject}
      />
    </div>
  {/if}
</section>

<style>
  .board-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    height: 100%;
  }

  .page-header {
    flex-shrink: 0;
    min-width: 0;
  }

  .page-header h1 {
    margin: 0;
    color: var(--fg);
    font-size: 22px;
    font-weight: 680;
    line-height: 29px;
  }

  .page-subtitle {
    margin: var(--space-1) 0 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .board-list-wrap {
    flex: 1 1 auto;
    min-height: 0;
  }
</style>
