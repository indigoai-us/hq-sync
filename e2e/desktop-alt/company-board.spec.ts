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
 *      view + StoryDetailPanel.
 *   3. CompanyTabs + CompanyPage have the Board tab as the first/default tab.
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

  it('takes a slug prop and follows the ActivityPanel load convention', () => {
    expect(panel).toContain('slug: string');
    // Warm load with an $effect keyed on slug + a cancel flag.
    expect(panel).toContain('let cancelled = false');
    expect(panel).toContain('cancelled = true');
    // Error state for a failed load.
    expect(panel).toContain('let error = $state<string | null>(null)');
  });

  it('loads the company goals and renders objectives with optional KR bars', () => {
    expect(panel).toContain('loadCompanyGoals');
    expect(panel).toContain('data-testid="goal-card"');
    // KRs render only when present (board data is [] today → empty state).
    expect(panel).toContain('objective.keyResults.length > 0');
    expect(panel).toContain('data-testid="goal-key-results"');
    expect(panel).toContain('krPercent');
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

  it('feeds the company-filtered projects to ProjectListView (showCompany off)', () => {
    expect(panel).toContain("import ProjectListView from '../components/ProjectListView.svelte'");
    expect(panel).toContain('<ProjectListView');
    expect(panel).toContain('projects={companyProjects}');
  });

  it('drills into the detail view → Kanban → story detail with a back affordance', () => {
    expect(panel).toContain("import ProjectDetailView from '../pages/ProjectDetailView.svelte'");
    expect(panel).toContain("import StoryDetailPanel from '../components/StoryDetailPanel.svelte'");
    expect(panel).toContain('loadLocalProjectStories');
    expect(panel).toContain('<ProjectDetailView');
    expect(panel).toContain('onback={backToList}');
    expect(panel).toContain('onselectStory={openStory}');
    expect(panel).toContain('<StoryDetailPanel');
    expect(panel).toContain('story={selectedStory}');
    expect(panel).toContain('onclose={closeStory}');
    expect(panel).toContain('onselectDependency={selectStoryById}');
    // The embedded Kanban (reachable from the detail view) is the drill target.
    const detail = readRepoFile('src/desktop-alt/pages/ProjectDetailView.svelte');
    expect(detail).toContain('import StoryKanban');
  });

  it('stays token-driven (no hardcoded hex)', () => {
    const styleBlock = panel.split('<style>')[1] ?? '';
    expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });
});

describe('desktop-alt Board tab is first + default (US-011)', () => {
  const tabs = readRepoFile('src/desktop-alt/components/CompanyTabs.svelte');
  const company = readRepoFile('src/desktop-alt/pages/CompanyPage.svelte');

  it('CompanyTabs declares board in the union and as the FIRST tab', () => {
    expect(tabs).toContain(
      "export type CompanyTab = 'board' | 'activity' | 'deployments' | 'secrets'",
    );
    // Board is the first entry in the derived tabs array.
    const tabsArrayStart = tabs.indexOf('const tabs = $derived([');
    const boardIdx = tabs.indexOf("{ id: 'board' as const", tabsArrayStart);
    const activityIdx = tabs.indexOf("{ id: 'activity' as const", tabsArrayStart);
    expect(boardIdx).toBeGreaterThan(tabsArrayStart);
    expect(boardIdx).toBeLessThan(activityIdx);
    expect(tabs).toContain("{ id: 'board' as const, label: 'Board', count: summary.board }");
  });

  it('CompanyPage defaults to the Board tab on init and on slug change', () => {
    expect(company).toContain("import CompanyBoardPanel from '../panels/CompanyBoardPanel.svelte'");
    expect(company).toContain("let activeTab = $state<CompanyTab>('board')");
    // The slug-change reset also returns to Board.
    expect(company).toContain("activeTab = 'board'");
    // Wired as the first branch in the panel switch.
    expect(company).toContain("{#if activeTab === 'board'}");
    expect(company).toContain('<CompanyBoardPanel slug={company.slug} />');
  });
});
