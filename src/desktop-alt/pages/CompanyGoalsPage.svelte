<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { buildClaudeCodeUrl } from '../../lib/claude-code-link';
  import {
    loadCompanyGoals,
    loadLocalProjects,
    loadLocalProjectStories,
    type KeyResult,
    type Objective,
  } from '../lib/local-projects';
  import {
    projectDisplayName,
    type Project,
    type Story,
  } from '../lib/projects-model';
  import ProjectDetailView from './ProjectDetailView.svelte';
  import StoryPanel from '../v4/StoryPanel.svelte';
  import '../v4/tokens.css';

  interface Props {
    slug: string;
  }

  type DotTone = 'ok' | 'warn' | 'error' | 'idle';

  interface GoalStatus {
    label: string;
    tone: DotTone;
    atRisk: boolean;
  }

  let { slug }: Props = $props();

  let objectives = $state<Objective[]>([]);
  let projects = $state<Project[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  let selected = $state<Project | null>(null);
  let stories = $state<Story[]>([]);
  let storiesLoading = $state(false);
  let storiesError = $state<string | null>(null);
  let selectedStoryId = $state<string | null>(null);
  let actionBusy = $state<string | null>(null);
  let actionMessage = $state<string | null>(null);

  const companyProjects = $derived(projects.filter((project) => project.company === slug));
  const linkedProjectCount = $derived.by(() => {
    const ids = new Set<string>();
    for (const objective of objectives) {
      for (const project of linkedProjects(objective)) ids.add(project.id);
    }
    return ids.size;
  });
  const targetQuarter = $derived.by(() => {
    const value = objectives.map((objective) => quarterLabel(objective.timeframe)).find(Boolean);
    return value ?? 'No target';
  });
  const selectedStory = $derived(
    selectedStoryId === null
      ? null
      : (stories.find((story) => story.id === selectedStoryId) ?? null),
  );

  $effect(() => {
    const activeSlug = slug;
    objectives = [];
    projects = [];
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
        const [goals, allProjects] = await Promise.all([
          loadCompanyGoals(activeSlug),
          loadLocalProjects(),
        ]);
        if (cancelled) return;
        objectives = goals.objectives;
        projects = allProjects;
      } catch (err) {
        console.error('CompanyGoalsPage load failed:', err);
        if (!cancelled) {
          error = 'Goals unavailable. Try again after a sync.';
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

  function goalStatus(raw: string): GoalStatus {
    const normalized = raw.toLowerCase().replace(/[_\s]+/g, '-').trim();
    if (normalized === 'on-track' || normalized === 'active' || normalized === 'running') {
      return { label: 'ON TRACK', tone: 'ok', atRisk: false };
    }
    if (normalized === 'at-risk' || normalized === 'review') {
      return { label: 'AT RISK', tone: 'warn', atRisk: true };
    }
    if (normalized === 'off-track' || normalized === 'blocked') {
      return { label: 'OFF TRACK', tone: 'error', atRisk: true };
    }
    return {
      label: normalized ? normalized.replace(/-/g, ' ').toUpperCase() : 'NO STATUS',
      tone: 'idle',
      atRisk: false,
    };
  }

  function looseNumber(value: number | string | null | undefined): number | null {
    if (value == null || value === '') return null;
    const raw = typeof value === 'number' ? value : value.replace(/,/g, '');
    const parsed = typeof raw === 'number' ? raw : Number.parseFloat(raw);
    return Number.isFinite(parsed) ? parsed : null;
  }

  function clampPercent(value: number): number {
    return Math.max(0, Math.min(100, Math.round(value)));
  }

  function krProgress(kr: KeyResult): number {
    const current = looseNumber(kr.current);
    const target = looseNumber(kr.target);
    if (current === null || target === null || target === 0) return 0;
    if (target < current && current > 0) return clampPercent((target / current) * 100);
    return clampPercent((current / target) * 100);
  }

  function formatValue(value: number | string | null | undefined, unit?: string): string {
    if (value == null || value === '') return '—';
    const text = String(value);
    if (!unit) return text;
    if (unit === '$' || unit.toLowerCase() === 'usd') return `$${text}`;
    if (unit === '%' || unit.startsWith('/')) return `${text}${unit}`;
    return `${text} ${unit}`;
  }

  function keyResultName(kr: KeyResult): string {
    return kr.title || kr.metric || 'Key result';
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

  function linkedProjects(objective: Objective): Project[] {
    const ids = objectiveIds(objective);
    if (ids.size === 0) return [];
    return companyProjects.filter((project) =>
      projectTokens(project).some((token) => ids.has(token)),
    );
  }

  function overflowCount(items: Project[]): number {
    return Math.max(0, items.length - 3);
  }

  function quarterLabel(value: string | null | undefined): string | null {
    const raw = (value ?? '').trim();
    if (!raw) return null;
    const quarter = raw.match(/Q[1-4]/i)?.[0]?.toUpperCase();
    const year = raw.match(/\b20\d{2}\b/)?.[0];
    if (quarter && year) return `${quarter} ${year}`;
    return quarter ?? raw;
  }

  function ownerLabel(value: string | null | undefined): string {
    const raw = (value ?? '').trim();
    if (!raw) return 'Agent';
    if (raw.toLowerCase() === 'you' || raw.toLowerCase() === 'me') return 'You';
    if (raw.toLowerCase() === 'agent') return 'Agent';
    return raw;
  }

  function riskNote(objective: Objective): string {
    // The at-risk basis is real — derived from a flat/at-risk/blocked KR status
    // (or the objective's own description). We deliberately do NOT append a
    // fabricated proposal-count tail: no such store exists, so claiming the
    // agent already drafted N projects would invent activity that never happened.
    const flat = objective.keyResults.find((kr) => {
      const status = (kr.status ?? '').toLowerCase().replace(/[_\s]+/g, '-');
      return status === 'at-risk' || status === 'flat' || status === 'blocked';
    });
    return `at risk — ${flat ? `${keyResultName(flat)} KR flat` : objective.description || 'progress needs attention'}`;
  }

  async function openHqPrompt(kind: string, prompt: string) {
    if (actionBusy) return;
    actionBusy = kind;
    actionMessage = null;
    try {
      const config: { hqFolderPath?: string } = await invoke<{ hqFolderPath?: string }>(
        'get_config',
      ).catch(() => ({}));
      const url = buildClaudeCodeUrl({ folder: config.hqFolderPath ?? '', prompt });
      await invoke('open_claude_code_link', { url });
      actionMessage = 'Opened in Claude Code.';
    } catch (err) {
      console.error('open_claude_code_link failed:', err);
      try {
        await navigator.clipboard.writeText(prompt);
        actionMessage = 'Prompt copied.';
      } catch {
        actionMessage = 'Could not open Claude Code.';
      }
    } finally {
      actionBusy = null;
    }
  }

  function newGoal() {
    const prompt = [
      `/goals ${slug}`,
      '',
      `Create a new measurable company goal for ${slug}.`,
      'Interview me for the missing objective, owner, target quarter, and key results, then update the local company goals source so it appears in HQ Sync.',
    ].join('\n');
    void openHqPrompt('new-goal', prompt);
  }

  function reviewProposal(objective: Objective) {
    const prompt = [
      `/goals ${slug}`,
      '',
      `This goal is flagged at risk: ${objective.title || 'Untitled goal'}.`,
      `Current note: ${riskNote(objective)}`,
      objective.description ? `Goal description: ${objective.description}` : null,
      'Recommend the projects or story changes that should bring the goal back on track, then write the approved updates into the local HQ project/goal files.',
    ]
      .filter((line): line is string => Boolean(line))
      .join('\n');
    void openHqPrompt(`review-${objective.id || objective.title}`, prompt);
  }

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
      const detail = err instanceof Error ? err.message : String(err);
      storiesError = `Could not load this project’s stories — ${detail}`;
      stories = [];
    } finally {
      storiesLoading = false;
    }
  }

  function backToGoals(): void {
    selected = null;
    stories = [];
    storiesError = null;
    selectedStoryId = null;
  }

  function onProjectStatusChange(projectId: string, status: string): void {
    if (selected && selected.id === projectId) {
      selected = { ...selected, status };
    }
    projects = projects.map((project) =>
      project.id === projectId ? { ...project, status } : project,
    );
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
</script>

<section class="company-goals" aria-labelledby="company-goals-title" data-testid="company-goals-page">
  {#if selected}
    <ProjectDetailView
      project={selected}
      {stories}
      {storiesLoading}
      {storiesError}
      objectives={objectives}
      onback={backToGoals}
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
    <header class="goals-header">
      <div class="goals-heading">
        <h2 id="company-goals-title">Goals</h2>
        <span>
          {objectives.length} {objectives.length === 1 ? 'goal' : 'goals'} · {targetQuarter}
          · linked to {linkedProjectCount} {linkedProjectCount === 1 ? 'project' : 'projects'}
        </span>
      </div>
      <div class="goal-actions">
        {#if actionMessage}
          <span class="action-status" role="status">{actionMessage}</span>
        {/if}
        <button
          type="button"
          class="new-goal-button"
          onclick={newGoal}
          disabled={actionBusy !== null}
        >
          {actionBusy === 'new-goal' ? 'Opening…' : 'New goal'}
        </button>
      </div>
    </header>

    {#if error}
      <div class="goals-error" role="alert">{error}</div>
    {/if}

    {#if loading}
      <div class="goal-stack" aria-busy="true">
        {#each [0, 1] as row (row)}
          <div class="goal-skeleton"></div>
        {/each}
      </div>
    {:else if objectives.length === 0}
      <div class="empty-state" data-testid="empty-goals-state">
        <span>No goals yet</span>
        <p>Company goals will appear here after goals sync into the local workspace.</p>
      </div>
    {:else}
      <div class="goal-stack">
        {#each objectives as objective (objective.id || objective.title)}
          {@const status = goalStatus(objective.status)}
          {@const linked = linkedProjects(objective)}
          <article class:at-risk={status.atRisk} class="goal-card" data-testid="goal-card">
            <header class="goal-card-header">
              <div class="goal-title-row">
                <h3>{objective.title || 'Untitled goal'}</h3>
                <span class="goal-status">
                  <span class={`status-dot ${status.tone}`} aria-hidden="true"></span>
                  <span>{status.label}</span>
                </span>
              </div>
              <span class="goal-meta">
                owner: {ownerLabel(objective.owner)} · target {quarterLabel(objective.timeframe) ?? '—'}
              </span>
            </header>

            <table class="kr-table" data-testid="kr-table">
              <thead>
                <tr>
                  <th>Key result</th>
                  <th>Current → target</th>
                  <th>Progress</th>
                </tr>
              </thead>
              <tbody>
                {#if objective.keyResults.length === 0}
                  <tr>
                    <td class="kr-empty" colspan="3">No key results yet</td>
                  </tr>
                {:else}
                  {#each objective.keyResults as kr, index (kr.id || `${objective.id}-${index}`)}
                    {@const progress = krProgress(kr)}
                    <tr data-testid="kr-row">
                      <td>{keyResultName(kr)}</td>
                      <td class="kr-value">
                        {formatValue(kr.current, kr.unit)} → {formatValue(kr.target, kr.unit)}
                      </td>
                      <td>
                        <span class="kr-progress" aria-label={`${progress}% progress`}>
                          <span class="progress-track" aria-hidden="true">
                            <span class="progress-fill" style={`width: ${progress}%`}></span>
                          </span>
                          <span>{progress}%</span>
                        </span>
                      </td>
                    </tr>
                  {/each}
                {/if}
              </tbody>
            </table>

            {#if status.atRisk}
              <div class="risk-row" data-testid="at-risk-note">
                <span class="status-dot warn" aria-hidden="true"></span>
                <span>{riskNote(objective)}</span>
                <button
                  type="button"
                  onclick={() => reviewProposal(objective)}
                  disabled={actionBusy !== null}
                >
                  {actionBusy === `review-${objective.id || objective.title}` ? 'Opening…' : 'Review proposal'}
                </button>
              </div>
            {/if}

            <footer class="linked-row">
              <span>Linked projects</span>
              <div class="project-chips" data-testid="linked-projects">
                {#if linked.length === 0}
                  <span class="muted-chip">None</span>
                {:else}
                  {#each linked.slice(0, 3) as project (project.id)}
                    <button
                      type="button"
                      class="project-chip"
                      data-testid="linked-project-chip"
                      onclick={() => openProject(project)}
                    >
                      {projectDisplayName(project)}
                    </button>
                  {/each}
                  {#if overflowCount(linked) > 0}
                    <span class="muted-chip">+{overflowCount(linked)}</span>
                  {/if}
                {/if}
              </div>
            </footer>
          </article>
        {/each}
      </div>
    {/if}

    <p class="goals-footnote">
      Goals are objectives with measurable key results. Projects ladder up to goals; progress rolls up automatically.
    </p>
  {/if}
</section>

<style>
  .company-goals {
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

  .goals-header,
  .goal-card-header,
  .goal-title-row,
  .goal-actions,
  .risk-row,
  .linked-row,
  .project-chips,
  .goal-status {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .goals-header {
    justify-content: space-between;
    gap: 18px;
  }

  .goal-actions {
    flex: 0 0 auto;
    gap: 10px;
  }

  .action-status {
    max-width: 160px;
    overflow: hidden;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .goals-heading {
    display: flex;
    align-items: baseline;
    gap: 9px;
    min-width: 0;
  }

  .goals-heading h2 {
    margin: 0;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.2;
  }

  .goals-heading span,
  .goal-meta,
  .goals-footnote {
    overflow: hidden;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .new-goal-button {
    flex: 0 0 auto;
    height: 30px;
    padding: 0 12px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 30px;
    cursor: default;
  }

  .new-goal-button:disabled,
  .risk-row button:disabled {
    opacity: 0.52;
  }

  .goals-error {
    padding: 10px 12px;
    border: 1px solid rgba(254, 188, 46, 0.3);
    border-radius: 6px;
    background: var(--v4-inset);
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.35;
  }

  .goal-stack {
    display: flex;
    flex-direction: column;
    gap: 18px;
    min-width: 0;
  }

  .goal-card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
    padding: 18px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
  }

  .goal-card.at-risk {
    border-color: rgba(254, 188, 46, 0.3);
  }

  .goal-card-header {
    justify-content: space-between;
    gap: 18px;
  }

  .goal-title-row {
    gap: 9px;
  }

  .goal-title-row h3 {
    margin: 0;
    overflow: hidden;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .goal-status {
    flex: 0 0 auto;
    gap: 7px;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.2;
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

  .kr-table {
    width: 100%;
    min-width: 620px;
    border-collapse: collapse;
    table-layout: fixed;
  }

  .kr-table th {
    padding: 0 0 9px;
    border-bottom: 1px solid var(--v4-rowline);
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.2;
    text-align: left;
    text-transform: uppercase;
  }

  .kr-table th:nth-child(1) {
    width: 55%;
  }

  .kr-table th:nth-child(2) {
    width: 20%;
  }

  .kr-table th:nth-child(3) {
    width: 25%;
  }

  .kr-table td {
    height: 35px;
    padding: 8px 16px 8px 0;
    border-bottom: 1px solid var(--v4-rowline);
    overflow: hidden;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
    vertical-align: middle;
  }

  .kr-table td.kr-value {
    color: var(--v4-text-1);
  }

  .kr-table td.kr-empty {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    text-align: center;
  }

  .kr-progress {
    display: grid;
    grid-template-columns: minmax(54px, 120px) 34px;
    align-items: center;
    gap: 10px;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.2;
    font-variant-numeric: tabular-nums;
  }

  .progress-track {
    height: 3px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--v4-control-faint);
  }

  .progress-fill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--v4-text-2);
  }

  .risk-row {
    gap: 8px;
    min-height: 25px;
    padding-top: 1px;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.25;
  }

  .risk-row span:nth-child(2) {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .risk-row button {
    flex: 0 0 auto;
    margin-left: auto;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 400;
    cursor: default;
  }

  .linked-row {
    gap: 12px;
  }

  .linked-row > span {
    flex: 0 0 auto;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.2;
    text-transform: uppercase;
  }

  .project-chips {
    gap: 8px;
    flex-wrap: wrap;
  }

  .project-chip,
  .muted-chip {
    display: inline-flex;
    max-width: 220px;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    overflow: hidden;
    border: 0;
    border-radius: 4px;
    background: var(--v4-control-faint);
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .project-chip {
    cursor: pointer;
  }

  .project-chip:hover,
  .project-chip:focus-visible {
    color: var(--v4-text-1);
    outline: none;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 28px 20px;
    border: 1px dashed var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.35;
    text-align: center;
  }

  .empty-state span {
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .empty-state p,
  .goals-footnote {
    margin: 0;
  }

  .goal-skeleton {
    height: 160px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    animation: goals-skeleton-pulse 1.3s ease-in-out infinite;
  }

  @keyframes goals-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (max-width: 980px) {
    .goals-heading {
      flex-direction: column;
      align-items: flex-start;
      gap: 4px;
    }

    .goal-card {
      overflow-x: auto;
    }
  }

  @media (max-width: 720px) {
    .goals-header,
    .goal-card-header,
    .goal-actions,
    .linked-row {
      align-items: flex-start;
      flex-direction: column;
    }

    .new-goal-button,
    .goal-actions {
      width: 100%;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .goal-skeleton {
      animation: none;
    }
  }
</style>
