<script lang="ts">
  /**
   * ProjectListView — the Board project list (US-007).
   *
   * Ported from hq-desktop's ProjectListView: debounced search, status-filter
   * pills (All / Active / In Progress / Completed / Archived), a Status|Company
   * group-by toggle, collapsible sections, and ProjectRow rows that open a
   * project's story Kanban. Presentational + filtering only — the project data
   * is loaded by the parent (CompanyBoardPanel). Token-driven, monochrome glass.
   */
  import {
    filterProjectsByQuery,
    groupProjects,
    matchesStatusFilter,
    projectListStatus,
    STATUS_FILTER_OPTIONS,
    type Project,
    type ProjectGroupMode,
    type StatusFilter,
  } from '../lib/projects-model';
  import ProjectRow from './ProjectRow.svelte';

  interface Props {
    projects: Project[];
    loading?: boolean;
    error?: string | null;
    onselect?: (project: Project) => void;
  }

  let { projects, loading = false, error = null, onselect }: Props = $props();

  let searchInput = $state('');
  let debouncedQuery = $state('');
  let statusFilter = $state<StatusFilter>('active');
  let groupMode = $state<ProjectGroupMode>('status');
  let collapsed = $state<Record<string, boolean>>({});

  // Debounce the search input by 200ms (matches hq-desktop) so the list isn't
  // re-filtered on every keystroke.
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const value = searchInput;
    debounceTimer = setTimeout(() => {
      debouncedQuery = value;
    }, 200);
    return () => clearTimeout(debounceTimer);
  });

  const statusFiltered = $derived(
    projects.filter((project) =>
      matchesStatusFilter(projectListStatus(project), statusFilter),
    ),
  );
  const searched = $derived(filterProjectsByQuery(statusFiltered, debouncedQuery));
  const sections = $derived(groupProjects(searched, groupMode));
  const totalFiltered = $derived(searched.length);
  const hasProjects = $derived(projects.length > 0);
  const isFiltering = $derived(
    debouncedQuery.trim().length > 0 || statusFilter !== 'all',
  );
  const noResults = $derived(hasProjects && !loading && totalFiltered === 0);

  function setStatusFilter(value: StatusFilter) {
    statusFilter = value;
  }

  function setGroupMode(mode: ProjectGroupMode) {
    groupMode = mode;
  }

  function toggleSection(key: string) {
    collapsed = { ...collapsed, [key]: !collapsed[key] };
  }

  function clearFilters() {
    searchInput = '';
    debouncedQuery = '';
    statusFilter = 'all';
  }
</script>

<section class="project-list" aria-label="Projects">
  {#if hasProjects}
    <div class="list-controls">
      <div class="search-field">
        <input
          type="search"
          class="search-input"
          placeholder="Filter projects…"
          aria-label="Filter projects"
          data-testid="project-search"
          bind:value={searchInput}
        />
      </div>

      <div class="status-pills" role="group" aria-label="Status filter">
        {#each STATUS_FILTER_OPTIONS as option (option.value)}
          <button
            type="button"
            class="status-pill"
            class:is-active={statusFilter === option.value}
            aria-pressed={statusFilter === option.value}
            data-testid={`status-pill-${option.value}`}
            onclick={() => setStatusFilter(option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>

      <div class="group-toggle" role="group" aria-label="Group projects by">
        <button
          type="button"
          class="group-segment"
          class:is-active={groupMode === 'status'}
          aria-pressed={groupMode === 'status'}
          data-testid="group-by-status"
          onclick={() => setGroupMode('status')}
        >
          Status
        </button>
        <button
          type="button"
          class="group-segment"
          class:is-active={groupMode === 'company'}
          aria-pressed={groupMode === 'company'}
          data-testid="group-by-company"
          onclick={() => setGroupMode('company')}
        >
          Company
        </button>
      </div>
    </div>
  {/if}

  {#if error}
    <div class="list-error" role="alert">{error}</div>
  {/if}

  <div class="list-body">
    {#if loading && projects.length === 0}
      <div class="list-loading" aria-busy="true">
        {#each [0, 1, 2] as row (row)}
          <div class="skeleton-row"></div>
        {/each}
      </div>
    {:else if !hasProjects && !error}
      <div class="list-empty">
        <p class="empty-title">No projects found</p>
        <p class="empty-detail">Projects are registered in each company’s board.json.</p>
      </div>
    {:else if noResults}
      <div class="list-empty">
        <p class="empty-title">No projects match your filters</p>
        <button type="button" class="link-button" onclick={clearFilters}>
          Clear all filters
        </button>
      </div>
    {:else}
      {#each sections as section (section.key)}
        <div class="project-section">
          <button
            type="button"
            class="section-header"
            aria-expanded={!collapsed[section.key]}
            onclick={() => toggleSection(section.key)}
          >
            <span class="chevron" class:is-open={!collapsed[section.key]} aria-hidden="true">›</span>
            <span class="section-label">{section.label}</span>
            <span class="section-count">{section.projects.length}</span>
          </button>

          {#if !collapsed[section.key]}
            <div class="section-rows">
              {#each section.projects as project (`${project.company}:${project.id}`)}
                <ProjectRow
                  {project}
                  showCompany={groupMode !== 'company'}
                  {onselect}
                />
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</section>

<style>
  .project-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
    height: 100%;
  }

  .list-controls {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    gap: var(--space-3);
  }

  .search-field {
    min-width: 0;
  }

  .search-input {
    width: 100%;
    height: 32px;
    min-width: 0;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-sm);
    transition:
      background 140ms ease,
      border-color 140ms ease;
  }

  .search-input::placeholder {
    color: var(--muted-3);
  }

  .search-input:focus {
    border-color: var(--border-strong);
    background: var(--row-hover);
    outline: none;
  }

  .status-pills {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .status-pill {
    padding: var(--space-1) var(--space-3);
    border: 1px solid transparent;
    border-radius: 999px;
    background: var(--row-hover);
    color: var(--muted);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease,
      border-color 140ms ease;
  }

  .status-pill:hover {
    color: var(--fg);
    background: var(--row-active);
  }

  .status-pill.is-active {
    border-color: var(--border-strong);
    background: var(--row-active);
    color: var(--fg);
  }

  .status-pill:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .group-toggle {
    display: inline-flex;
    align-self: flex-start;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
  }

  .group-segment {
    padding: var(--space-1) var(--space-3);
    border: 0;
    border-radius: calc(var(--radius-sm) - 2px);
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 650;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .group-segment:hover {
    color: var(--fg);
  }

  .group-segment.is-active {
    background: var(--popover-primary);
    color: var(--popover-primary-text);
  }

  .group-segment:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .list-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-sm);
  }

  .list-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  .project-section {
    margin-bottom: var(--space-4);
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: left;
    cursor: pointer;
    transition: background 140ms ease;
  }

  .section-header:hover {
    background: var(--row-hover);
  }

  .section-header:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .chevron {
    flex: 0 0 auto;
    color: var(--muted-3);
    font-size: var(--text-base);
    line-height: 1;
    transition: transform 150ms ease;
  }

  .chevron.is-open {
    transform: rotate(90deg);
  }

  .section-label {
    color: var(--muted-2);
    font-size: var(--text-xs);
    font-weight: 650;
    text-transform: capitalize;
  }

  .section-count {
    padding: 0 6px;
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-3);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    line-height: 16px;
  }

  .section-rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
    padding-left: var(--space-2);
  }

  .list-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .skeleton-row {
    height: 64px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
    animation: skeleton-pulse 1.3s ease-in-out infinite;
  }

  .list-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-4);
  }

  .empty-title {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-sm);
  }

  .empty-detail {
    margin: 0;
    color: var(--muted-3);
    font-size: var(--text-xs);
  }

  .link-button {
    border: 0;
    background: transparent;
    color: var(--blue);
    font: inherit;
    font-size: var(--text-xs);
    cursor: pointer;
  }

  @keyframes skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .status-pill,
    .group-segment,
    .section-header,
    .chevron,
    .search-input {
      transition: none;
    }

    .skeleton-row {
      animation: none;
    }
  }
</style>
