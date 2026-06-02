import { describe, expect, it } from 'vitest';
import {
  filterProjectsByQuery,
  groupProjects,
  matchesStatusFilter,
  projectListStatus,
  projectProgress,
  STATUS_FILTER_OPTIONS,
  type Project,
  type StatusFilter,
} from '../../src/desktop-alt/lib/projects-model';
import {
  getDesktopHotkeyRoute,
  getDesktopSidebarRows,
  type DesktopRoute,
} from '../../src/desktop-alt/route';
import type { Workspace } from '../../src/lib/workspaces';
import { readRepoFile } from './harness';

/**
 * US-007 — top-level Board surface.
 *
 * Source-contract (non-render) harness, matching the existing desktop-alt spec
 * style. Asserts at two levels:
 *   1. Logic — the Board route lives in the sidebar + hotkeys, and the pure
 *      list-filter/group/status helpers behave over a fixture project set.
 *   2. Source contract — BoardPage + ProjectListView + ProjectRow wire the
 *      search, status pills, group-by toggle, progress bars, the StoryKanban
 *      drill-in, and the back affordance, all token-driven; and the old flat
 *      per-company Board tab is unwired from the company page.
 */

function workspace(overrides: Partial<Workspace>): Workspace {
  return {
    slug: 'indigo',
    displayName: 'Indigo',
    kind: 'company',
    state: 'synced',
    cloudUid: 'cmp_1',
    bucketName: 'bucket',
    hasLocalFolder: true,
    localPath: '/tmp/HQ/companies/indigo',
    membershipStatus: 'active',
    role: 'member',
    lastSyncedAt: null,
    brokenReason: null,
    ...overrides,
  };
}

function project(overrides: Partial<Project>): Project {
  return {
    id: 'proj',
    title: 'Project',
    description: '',
    company: 'indigo',
    status: 'active',
    prdPath: 'companies/indigo/projects/proj/prd.json',
    storiesTotal: 4,
    storiesComplete: 2,
    ...overrides,
  };
}

const FIXTURE_PROJECTS: Project[] = [
  // Live: active board + in-flight stories → emphasised.
  project({ id: 'flagship', title: 'Flagship', company: 'indigo', status: 'active', storiesTotal: 10, storiesComplete: 4 }),
  // Completed: every story passes.
  project({ id: 'done', title: 'Done Deal', company: 'acme', status: 'active', storiesTotal: 3, storiesComplete: 3 }),
  // Archived: terminal.
  project({ id: 'old', title: 'Old Thing', company: 'acme', status: 'archived', storiesTotal: 5, storiesComplete: 1 }),
  // Pending: planned board, nothing done.
  project({ id: 'plan', title: 'Planned Work', company: 'indigo', status: 'planned', storiesTotal: 6, storiesComplete: 0 }),
];

describe('desktop-alt Board surface (US-007)', () => {
  it('adds a top-level Board route to the sidebar with its own hotkey', () => {
    const companies = [workspace({ slug: 'indigo', displayName: 'Indigo' })];
    const rows = getDesktopSidebarRows({ kind: 'board' }, companies);

    const board = rows.find((row) => row.label === 'Board');
    expect(board).toBeDefined();
    expect(board?.route).toEqual({ kind: 'board' });
    expect(board?.shortcut).toBe('⌘1');
    expect(board?.active).toBe(true);

    // Sync + Meetings stay as top-level destinations, renumbered after Board.
    expect(rows.find((r) => r.label === 'Sync')?.shortcut).toBe('⌘2');
    expect(rows.find((r) => r.label === 'Meetings')?.shortcut).toBe('⌘3');

    // ⌘1 maps to the Board route.
    expect(
      getDesktopHotkeyRoute({ key: '1', metaKey: true, ctrlKey: false }, companies),
    ).toEqual({ kind: 'board' } satisfies DesktopRoute);
  });

  it('classifies projects to effective list status and honours the status pills', () => {
    expect(projectListStatus(FIXTURE_PROJECTS[0])).toBe('live');
    expect(projectListStatus(FIXTURE_PROJECTS[1])).toBe('complete');
    expect(projectListStatus(FIXTURE_PROJECTS[2])).toBe('archived');
    expect(projectListStatus(FIXTURE_PROJECTS[3])).toBe('pending');

    const byFilter = (filter: StatusFilter) =>
      FIXTURE_PROJECTS.filter((p) => matchesStatusFilter(projectListStatus(p), filter)).map(
        (p) => p.id,
      );

    expect(byFilter('all').sort()).toEqual(['done', 'flagship', 'old', 'plan']);
    // "Active" hides completed + archived.
    expect(byFilter('active').sort()).toEqual(['flagship', 'plan']);
    expect(byFilter('in-progress')).toEqual(['flagship']);
    expect(byFilter('complete')).toEqual(['done']);
    expect(byFilter('archived')).toEqual(['old']);

    // All five pills are present.
    expect(STATUS_FILTER_OPTIONS.map((o) => o.value)).toEqual([
      'all',
      'active',
      'in-progress',
      'complete',
      'archived',
    ]);
  });

  it('groups by status (live first) and by company, and searches by query', () => {
    const byStatus = groupProjects(FIXTURE_PROJECTS, 'status');
    // Live section sorts ahead of the others.
    expect(byStatus[0].key).toBe('live');
    expect(byStatus[0].projects.map((p) => p.id)).toEqual(['flagship']);

    const byCompany = groupProjects(FIXTURE_PROJECTS, 'company');
    expect(byCompany.map((s) => s.key)).toEqual(['acme', 'indigo']);

    // Search matches title + company.
    expect(filterProjectsByQuery(FIXTURE_PROJECTS, 'flag').map((p) => p.id)).toEqual(['flagship']);
    expect(
      filterProjectsByQuery(FIXTURE_PROJECTS, 'acme').map((p) => p.id).sort(),
    ).toEqual(['done', 'old']);
    expect(filterProjectsByQuery(FIXTURE_PROJECTS, '').length).toBe(FIXTURE_PROJECTS.length);
  });

  it('computes per-project progress for the progress bar', () => {
    const live = projectProgress(4, 10);
    expect(live).toMatchObject({ complete: 4, total: 10, percent: 40 });
    const done = projectProgress(3, 3);
    expect(done).toMatchObject({ complete: 3, total: 3, percent: 100 });
  });

  it('wires the Board route into DesktopApp and the sidebar', () => {
    const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
    const route = readRepoFile('src/desktop-alt/route.ts');
    const sidebar = readRepoFile('src/desktop-alt/DesktopSidebar.svelte');

    // Route kind union extended + page switch + page import.
    expect(route).toContain("'board' | 'sync' | 'meetings' | 'company'");
    expect(desktopApp).toContain("import BoardPage from './pages/BoardPage.svelte'");
    expect(desktopApp).toContain("route.kind === 'board'");
    expect(desktopApp).toContain('<BoardPage companySlug={route.slug ?? null} />');

    // Sidebar carries the Board row first; companies fall after the 3 primaries.
    expect(route).toContain("label: 'Board'");
    expect(sidebar).toContain('rows.slice(0, 3)');
    expect(sidebar).toContain('rows.slice(3)');
  });

  it('wires the project list: search, pills, group-by, rows, progress, drill-in', () => {
    const list = readRepoFile('src/desktop-alt/components/ProjectListView.svelte');
    const row = readRepoFile('src/desktop-alt/components/ProjectRow.svelte');
    const page = readRepoFile('src/desktop-alt/pages/BoardPage.svelte');

    // Debounced search.
    expect(list).toContain('data-testid="project-search"');
    expect(list).toContain('debouncedQuery');
    expect(list).toContain('setTimeout');

    // Status pills (all five) + group-by toggle.
    expect(list).toContain('STATUS_FILTER_OPTIONS');
    expect(list).toContain('data-testid="group-by-status"');
    expect(list).toContain('data-testid="group-by-company"');
    expect(list).toContain('groupProjects');

    // Rows render progress + live emphasis.
    expect(list).toContain('import ProjectRow');
    expect(row).toContain('projectProgress');
    expect(row).toContain('progress-fill');
    expect(row).toContain('is-live');
    expect(row).toContain("onselect?.(project)");

    // BoardPage loads projects, then drills into the ProjectDetailView (US-009),
    // which embeds the StoryKanban via its Board tab and owns the back affordance.
    // (Superseded US-007's straight-to-Kanban contract: the StoryKanban import +
    // the back button now live in ProjectDetailView.svelte, not BoardPage.)
    expect(page).toContain('loadLocalProjects');
    expect(page).toContain('loadLocalProjectStories');
    expect(page).toContain('import ProjectDetailView');
    expect(page).toContain('<ProjectDetailView');
    const detail = readRepoFile('src/desktop-alt/pages/ProjectDetailView.svelte');
    expect(detail).toContain('import StoryKanban');
    expect(detail).toContain('<StoryKanban {stories}');
    expect(detail).toContain('data-testid="detail-back"');
    // Best-effort company pre-filter.
    expect(page).toContain('companySlug');
  });

  it('keeps the Board surface token-driven (no hardcoded hex)', () => {
    for (const path of [
      'src/desktop-alt/components/ProjectListView.svelte',
      'src/desktop-alt/components/ProjectRow.svelte',
      'src/desktop-alt/pages/BoardPage.svelte',
    ]) {
      const styleBlock = readRepoFile(path).split('<style>')[1] ?? '';
      expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    }
  });

  it('replaces the flat per-company Board tab — company page keeps Activity/Deployments/Secrets', () => {
    const tabs = readRepoFile('src/desktop-alt/components/CompanyTabs.svelte');
    const company = readRepoFile('src/desktop-alt/pages/CompanyPage.svelte');

    // CompanyTab type no longer includes 'board'.
    expect(tabs).toContain("export type CompanyTab = 'activity' | 'deployments' | 'secrets'");
    expect(tabs).not.toContain("{ id: 'board' as const");

    // The company page no longer renders BoardPanel and opens on Activity.
    expect(company).not.toContain('<BoardPanel slug={company.slug} />');
    expect(company).toContain("let activeTab = $state<CompanyTab>('activity')");
    expect(company).toContain('<ActivityPanel slug={company.slug} />');
    expect(company).toContain('<DeploymentsPanel slug={company.slug} />');
    expect(company).toContain('<SecretsPanel slug={company.slug} />');
  });
});
