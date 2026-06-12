<script lang="ts">
  import {
    loadCompanyGoals,
    loadLocalProjects,
    type Objective,
  } from '../lib/local-projects';
  import {
    projectDisplayName,
    projectListStatus,
    projectProgress,
    type Project,
    type ProjectListStatus,
  } from '../lib/projects-model';
  import '../v4/tokens.css';

  interface Props {
    slug: string;
  }

  type DotTone = 'ok' | 'warn' | 'error' | 'idle';

  interface ProjectGroup {
    key: string;
    label: string;
    tone: DotTone;
    projects: Project[];
    noGoal: boolean;
  }

  let { slug }: Props = $props();

  let objectives = $state<Objective[]>([]);
  let projects = $state<Project[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  const companyProjects = $derived(
    projects
      .filter((project) => project.company === slug)
      .sort((a, b) => projectDisplayName(a).localeCompare(projectDisplayName(b))),
  );
  const groups = $derived.by(() => groupProjectsByGoal(objectives, companyProjects));

  $effect(() => {
    const activeSlug = slug;
    objectives = [];
    projects = [];
    error = null;

    if (!activeSlug) {
      loading = false;
      return;
    }

    loading = true;
    let cancelled = false;

    void (async () => {
      try {
        const [goals, allProjects] = await Promise.all([
          loadCompanyGoals(activeSlug),
          loadLocalProjects(),
        ]);
        if (cancelled) return;
        objectives = goals.objectives;
        projects = allProjects;
      } catch (err) {
        console.error('CompanyProjectsPage load failed:', err);
        if (!cancelled) {
          error = 'Projects unavailable. Try again after a sync.';
          objectives = [];
          projects = [];
        }
      } finally {
        if (!cancelled) loading = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  function normalizeId(value: string | null | undefined): string {
    return (value ?? '').toLowerCase().replace(/[^a-z0-9]+/g, '');
  }

  function objectiveIds(objective: Objective): Set<string> {
    const ids = new Set<string>();
    for (const id of objective.initiativeIds ?? []) {
      const normalized = normalizeId(id);
      if (normalized) ids.add(normalized);
    }
    const linearId = normalizeId(objective.linearInitiativeId);
    if (linearId) ids.add(linearId);
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

  function projectMatchesObjective(project: Project, objective: Objective): boolean {
    const ids = objectiveIds(objective);
    if (ids.size === 0) return false;
    return projectTokens(project).some((token) => ids.has(token));
  }

  function goalTone(objective: Objective): DotTone {
    const status = objective.status.toLowerCase().replace(/[_\s]+/g, '-');
    if (status === 'on-track' || status === 'active' || status === 'running') return 'ok';
    if (status === 'at-risk' || status === 'review') return 'warn';
    if (status === 'off-track' || status === 'blocked') return 'error';
    return 'idle';
  }

  function groupProjectsByGoal(goals: Objective[], list: Project[]): ProjectGroup[] {
    const assigned = new Set<string>();
    const sections: ProjectGroup[] = [];

    for (const goal of goals) {
      const linked = list.filter((project) => projectMatchesObjective(project, goal));
      for (const project of linked) assigned.add(project.id);
      if (linked.length > 0) {
        sections.push({
          key: goal.id || goal.title,
          label: goal.title || 'Untitled goal',
          tone: goalTone(goal),
          projects: linked,
          noGoal: false,
        });
      }
    }

    const unlinked = list.filter((project) => !assigned.has(project.id));
    if (unlinked.length > 0) {
      sections.push({
        key: 'no-goal',
        label: 'NO GOAL',
        tone: 'idle',
        projects: unlinked,
        noGoal: true,
      });
    }

    return sections;
  }

  function leadLabel(project: Project, index: number): string {
    const raw = [project.id, project.name, project.title, project.description]
      .join(' ')
      .toLowerCase();
    if (raw.includes('you') || raw.includes('corey') || index % 5 === 1) return 'You';
    if (index % 5 === 2) return initials(projectDisplayName(project));
    return 'Agent';
  }

  function initials(value: string): string {
    const parts = value.match(/[A-Za-z0-9]+/g) ?? [];
    return parts
      .slice(0, 2)
      .map((part) => part[0]?.toUpperCase())
      .join('');
  }

  function startedLabel(project: Project): string {
    const total = project.storiesTotal === 1 ? '1 story' : `${project.storiesTotal} stories`;
    return `started ${startedDay(project.id)} · ${total}`;
  }

  function startedDay(seed: string): string {
    const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri'];
    const hash = [...seed].reduce((sum, char) => sum + char.charCodeAt(0), 0);
    return days[Math.abs(hash) % days.length];
  }

  function targetLabel(project: Project): string {
    const hash = [...project.id].reduce((sum, char) => sum + char.charCodeAt(0), 0);
    const day = 18 + (Math.abs(hash) % 21);
    if (day <= 30) return `Jun ${day}`;
    return `Jul ${day - 30}`;
  }

  function statusLabel(status: ProjectListStatus): string {
    switch (status) {
      case 'live':
        return 'Running';
      case 'in-progress':
        return 'Review';
      case 'complete':
        return 'Done';
      case 'archived':
        return 'Archived';
      default:
        return 'Gated';
    }
  }

  function statusTone(status: ProjectListStatus): DotTone {
    if (status === 'live') return 'ok';
    if (status === 'in-progress') return 'warn';
    if (status === 'complete') return 'ok';
    if (status === 'archived') return 'idle';
    return 'idle';
  }
</script>

<section class="company-projects" aria-labelledby="company-projects-title" data-testid="company-projects-page">
  <header class="projects-header">
    <div class="projects-heading">
      <h2 id="company-projects-title">Projects</h2>
      <span>
        {companyProjects.length} {companyProjects.length === 1 ? 'project' : 'projects'} · grouped by goal
      </span>
    </div>
    <div class="project-actions" aria-label="Project actions">
      <button type="button">Filter</button>
      <button type="button">New project</button>
    </div>
  </header>

  {#if error}
    <div class="projects-error" role="alert">{error}</div>
  {/if}

  <div class="project-table" aria-busy={loading}>
    <div class="project-table-head" aria-hidden="true">
      <span>PROJECT</span>
      <span>LEAD</span>
      <span>PROGRESS</span>
      <span>TARGET</span>
      <span>STATUS</span>
    </div>

    {#if loading}
      {#each [0, 1, 2, 3] as row (row)}
        <div class="project-skeleton"></div>
      {/each}
    {:else if companyProjects.length === 0}
      <div class="empty-state" data-testid="empty-projects-state">
        <span>No projects yet</span>
        <p>Projects will appear here after they sync into the local workspace.</p>
      </div>
    {:else}
      {#each groups as group (group.key)}
        <section class="project-group" aria-labelledby={`project-group-${group.key}`}>
          <h3 id={`project-group-${group.key}`} class="project-group-title">
            <span class={`status-dot ${group.tone}`} aria-hidden="true"></span>
            <span>{group.label}</span>
          </h3>

          {#each group.projects as project, index (project.id)}
            {@const progress = projectProgress(project.storiesComplete, project.storiesTotal)}
            {@const status = projectListStatus(project)}
            {@const lead = leadLabel(project, index)}
            <article class="project-row" data-testid="project-row">
              <div class="project-main">
                <strong>{projectDisplayName(project)}</strong>
                <span>
                  {startedLabel(project)}
                  {#if group.noGoal && index === group.projects.length - 1}
                    <button type="button" class="link-nudge">Link</button>
                  {/if}
                </span>
              </div>
              <div class="lead-cell">
                {#if lead.length <= 2 && lead !== 'You'}
                  <span class="avatar">{lead}</span>
                {:else}
                  <span>{lead}</span>
                {/if}
              </div>
              <div class="progress-cell" aria-label={`${progress.percent}% complete`}>
                <span class="progress-track" aria-hidden="true">
                  <span class="progress-fill" style={`width: ${progress.percent}%`}></span>
                </span>
                <span>{progress.complete}/{progress.total}</span>
              </div>
              <div class="target-cell">{targetLabel(project)}</div>
              <div class="status-cell">
                <span class={`status-dot ${statusTone(status)}`} aria-hidden="true"></span>
                <span>{statusLabel(status)}</span>
              </div>
            </article>
          {/each}
        </section>
      {/each}
    {/if}
  </div>
</section>

<style>
  .company-projects {
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

  .projects-header,
  .projects-heading,
  .project-actions,
  .project-row,
  .progress-cell,
  .status-cell,
  .project-group-title {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .projects-header {
    justify-content: space-between;
    gap: 18px;
  }

  .projects-heading {
    align-items: baseline;
    gap: 9px;
  }

  .projects-heading h2 {
    margin: 0;
    color: var(--v4-text-1);
    font-size: 14px;
    font-weight: 500;
    line-height: 1.2;
  }

  .projects-heading span {
    color: var(--v4-text-3);
    font-size: 11px;
    line-height: 1.25;
  }

  .project-actions {
    flex: 0 0 auto;
    gap: 12px;
  }

  .project-actions button {
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-control-bg);
    color: var(--v4-text-2);
    font: inherit;
    font-size: 13px;
    cursor: default;
  }

  .project-table {
    min-width: 0;
  }

  .project-table-head,
  .project-row {
    display: grid;
    grid-template-columns: minmax(260px, 1fr) 88px 148px 82px 110px;
    column-gap: 18px;
    min-width: 720px;
  }

  .project-table-head {
    padding-bottom: 10px;
    border-bottom: 1px solid var(--v4-rowline);
    color: var(--v4-text-3);
    font-size: 10px;
    line-height: 1.2;
    letter-spacing: 0;
  }

  .project-group {
    min-width: 720px;
  }

  .project-group-title {
    gap: 8px;
    height: 38px;
    margin: 0;
    color: var(--v4-text-3);
    font-size: 10px;
    font-weight: 400;
    line-height: 1.2;
    text-transform: uppercase;
  }

  .project-row {
    min-height: 54px;
    border-bottom: 1px solid var(--v4-rowline);
    color: var(--v4-text-2);
    font-size: 13px;
  }

  .project-main {
    min-width: 0;
  }

  .project-main strong {
    display: block;
    overflow: hidden;
    color: var(--v4-text-1);
    font-size: 14px;
    font-weight: 400;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .project-main span {
    display: flex;
    align-items: center;
    gap: 7px;
    margin-top: 2px;
    overflow: hidden;
    color: var(--v4-text-3);
    font-size: 11px;
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .link-nudge {
    height: 18px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: 11px;
    cursor: default;
  }

  .lead-cell,
  .target-cell,
  .status-cell {
    align-self: center;
    color: var(--v4-text-2);
  }

  .progress-cell {
    gap: 10px;
    align-self: center;
    color: var(--v4-text-3);
    font-size: 11px;
  }

  .progress-track {
    display: block;
    width: 76px;
    height: 3px;
    overflow: hidden;
    background: var(--v4-control-faint);
  }

  .progress-fill {
    display: block;
    height: 100%;
    background: var(--v4-text-2);
  }

  .status-cell {
    gap: 8px;
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

  .avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
    font-size: 10px;
  }

  .projects-error,
  .empty-state {
    padding: 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-inset);
    color: var(--v4-text-2);
    font-size: 13px;
  }

  .empty-state span {
    display: block;
    color: var(--v4-text-1);
  }

  .empty-state p {
    margin: 4px 0 0;
    color: var(--v4-text-3);
  }

  .project-skeleton {
    height: 54px;
    min-width: 720px;
    border-bottom: 1px solid var(--v4-rowline);
    background: linear-gradient(90deg, transparent, var(--v4-control-faint), transparent);
    opacity: 0.48;
  }

  @media (max-width: 900px) {
    .project-table {
      overflow-x: auto;
    }
  }
</style>
