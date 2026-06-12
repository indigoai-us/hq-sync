import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-011 — per-company Board surface (CompanyBoardPanel).
 *
 * Source-contract style (matching the desktop-alt harness): the top-level
 * cross-company BoardPage was deleted; the board now lives on each company page,
 * scoped to that workspace, showing Goals + In-flight work + Projects and
 * reusing the existing project/story components. Asserts:
 *   1. The local-projects adapter wraps get_local_company_goals.
 *   2. CompanyBoardPanel loads goals, filters projects to its slug, surfaces
 *      in-flight via the classifier, and drills into the Kanban via the detail
 *      view + StoryPanel.
 *   3. CompanyPage defaults to the Overview section (V4 US-002), which hosts
 *      the board panel; the section list lives in route.ts.
 */

describe('desktop-alt company goals adapter (US-011)', () => {
  const adapter = readRepoFile('src/desktop-alt/lib/local-projects.ts');

  it('wraps get_local_company_goals with the camelCased company slug arg', () => {
    expect(adapter).toContain('export async function loadCompanyGoals');
    expect(adapter).toContain(
      "invoke<CompanyGoalsWire>('get_local_company_goals', {",
    );
    expect(adapter).toContain('companySlug: slug');
    // Returns the normalised Objective + Initiative shapes.
    expect(adapter).toContain('export interface Objective');
    expect(adapter).toContain('export interface Initiative');
    expect(adapter).toContain('export interface KeyResult');
    expect(adapter).toContain('export interface CompanyGoals');
  });
});

describe('desktop-alt CompanyBoardPanel source contract (US-011)', () => {
  const panel = readRepoFile('src/desktop-alt/panels/CompanyBoardPanel.svelte');
  const goalCard = readRepoFile('src/desktop-alt/v4/GoalCard.svelte');

  it('takes a slug prop and follows the ActivityPanel load convention', () => {
    expect(panel).toContain('slug: string');
    // Warm load with an $effect keyed on slug + a cancel flag.
    expect(panel).toContain('let cancelled = false');
    expect(panel).toContain('cancelled = true');
    // Error state for a failed load.
    expect(panel).toContain('let error = $state<string | null>(null)');
  });

  it('loads the company goals and renders objective cards through the V4 GoalCard', () => {
    expect(panel).toContain('loadCompanyGoals');
    expect(panel).toContain("import GoalCard from '../v4/GoalCard.svelte'");
    expect(panel).toContain('<GoalCard');
    expect(goalCard).toContain('data-testid="goal-card"');
    expect(goalCard).toContain('keyResultLine(objective.keyResults)');
    expect(goalCard).toContain('style={`width: ${progress}%`}');
    // Graceful empty state.
    expect(panel).toContain('No goals yet');
  });

  it('filters projects to its slug and surfaces in-flight via the classifier', () => {
    expect(panel).toContain('loadLocalProjects');
    // Scoped to ONE company.
    expect(panel).toContain('project.company === slug');
    // In-flight surfaces live / in-progress rollups first.
    expect(panel).toContain('projectListStatus');
    expect(panel).toContain("status === 'live' || status === 'in-progress'");
    // The current in-progress story title comes from the shared classifier.
    expect(panel).toContain('classifyStories');
    expect(panel).toContain("entry.state === 'in-progress'");
    expect(panel).toContain('data-testid="inflight-list"');
    expect(panel).toContain('data-testid="inflight-row"');
  });

  it('renders the V4 in-flight table with goal chips, priority, AC progress, and status', () => {
    expect(panel).toContain('data-testid="inflight-goal-chip"');
    expect(panel).toContain('goalChip(project)');
    expect(panel).toContain('priorityLabel(detail?.priority)');
    expect(panel).toContain('class="ac-track"');
    expect(panel).toContain('height: 3px');
    expect(panel).toContain('rowStatus(project, detail)');
  });

  it('feeds the company-filtered projects to ProjectListView (showCompany off)', () => {
    expect(panel).toContain("import ProjectListView from '../components/ProjectListView.svelte'");
    expect(panel).toContain('<ProjectListView');
    expect(panel).toContain('projects={companyProjects}');
  });

  it('drills into the detail view → Kanban → story detail with a back affordance', () => {
    expect(panel).toContain("import ProjectDetailView from '../pages/ProjectDetailView.svelte'");
    expect(panel).toContain("import StoryPanel from '../v4/StoryPanel.svelte'");
    expect(panel).toContain('loadLocalProjectStories');
    expect(panel).toContain('<ProjectDetailView');
    expect(panel).toContain('onback={backToList}');
    expect(panel).toContain('onselectStory={openStory}');
    expect(panel).toContain('<StoryPanel');
    expect(panel).toContain('story={selectedStory}');
    expect(panel).toContain('onclose={closeStory}');
    expect(panel).toContain('onselectDependency={selectStoryById}');
    expect(panel).toContain('{onStoryPassesChange}');
    // The embedded Kanban (reachable from the detail view) is the drill target.
    const detail = readRepoFile('src/desktop-alt/pages/ProjectDetailView.svelte');
    expect(detail).toContain('import StoryKanban');
  });

  it('stays token-driven (no hardcoded hex)', () => {
    const styleBlock = panel.split('<style>')[1] ?? '';
    expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });
});

describe('desktop-alt board is the default company section (US-011 → V4 US-002)', () => {
  const route = readRepoFile('src/desktop-alt/route.ts');
  const company = readRepoFile('src/desktop-alt/pages/CompanyPage.svelte');

  it('route.ts declares Overview FIRST among the eight company sections', () => {
    expect(route).toContain("export const DEFAULT_COMPANY_TAB: CompanyTab = 'overview'");
    const sectionsStart = route.indexOf('export const COMPANY_SECTIONS');
    const overviewIdx = route.indexOf("{ id: 'overview', label: 'Overview' }", sectionsStart);
    const goalsIdx = route.indexOf("{ id: 'goals', label: 'Goals' }", sectionsStart);
    expect(overviewIdx).toBeGreaterThan(sectionsStart);
    expect(overviewIdx).toBeLessThan(goalsIdx);
  });

  it('CompanyPage defaults to Overview, which hosts CompanyBoardPanel', () => {
    expect(company).toContain("import CompanyBoardPanel from '../panels/CompanyBoardPanel.svelte'");
    expect(company).toContain('tab = DEFAULT_COMPANY_TAB');
    // The in-page segmented control is gone — the secondary sidebar drives it.
    expect(company).not.toContain('CompanyTabs');
    // Wired as the first branch in the panel switch.
    expect(company).toContain("{#if tab === 'overview'}");
    expect(company).toContain('<CompanyBoardPanel slug={company.slug} />');
  });
});
