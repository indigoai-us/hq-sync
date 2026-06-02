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
    projectDisplayName,
    type Project,
    type Story,
  } from '../lib/projects-model';
  import ProjectListView from '../components/ProjectListView.svelte';
  import StoryKanban from '../components/StoryKanban.svelte';

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
  }

  onMount(() => {
    void loadProjects();
  });
</script>

<section class="board-page" aria-labelledby="board-page-title" aria-label="Board">
  {#if selected}
    <header class="page-header">
      <button
        type="button"
        class="back-button"
        data-testid="board-back"
        onclick={backToList}
      >
        <span class="back-chevron" aria-hidden="true">‹</span>
        <span>Projects</span>
      </button>
      <h1 id="board-page-title">{projectDisplayName(selected)}</h1>
      {#if selected.company}
        <p class="page-subtitle">{selected.company}</p>
      {/if}
    </header>

    <div class="board-drill">
      {#if storiesError}
        <div class="drill-error" role="alert">{storiesError}</div>
      {:else if !selected.prdPath}
        <div class="drill-empty">
          <p>This project has no linked PRD yet, so there are no stories to show.</p>
        </div>
      {:else}
        <StoryKanban {stories} loading={storiesLoading} />
      {/if}
    </div>
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

  .back-button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-bottom: var(--space-2);
    padding: var(--space-1) var(--space-2);
    margin-left: calc(-1 * var(--space-2));
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-sm);
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
    font-size: var(--text-lg);
    line-height: 1;
  }

  .board-list-wrap,
  .board-drill {
    flex: 1 1 auto;
    min-height: 0;
  }

  .drill-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-sm);
  }

  .drill-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-6);
    border: 1px dashed var(--border);
    border-radius: var(--radius-md);
    color: var(--muted-3);
    font-size: var(--text-sm);
  }

  .drill-empty p {
    margin: 0;
  }
</style>
