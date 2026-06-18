import { describe, expect, it } from 'vitest';
import {
  fromV4Route,
  getDesktopSecondarySidebar,
  type DesktopRoute,
} from '../../src/desktop-alt/route';
import type { Workspace } from '../../src/lib/workspaces';
import { readRepoFile } from './harness';

/**
 * US-002 — V4 chrome composition (route restructure).
 *
 * Source-contract + model harness, matching the existing desktop-alt spec
 * style. The story's E2E scenario: given a company row click in the sidebar,
 * when the company page opens, then the secondary sidebar shows the 8 company
 * sections with Overview active.
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
    role: 'owner',
    lastSyncedAt: null,
    brokenReason: null,
    invitedBy: null,
    invitedAt: null,
    ...overrides,
  };
}

describe('desktop-alt V4 chrome (US-002)', () => {
  it('a company row click opens the company page with the 10 sections and Overview active', () => {
    const companies = [workspace({})];

    // The V4Sidebar company row emits { kind: 'company', slug } — the shell
    // narrows it onto the DesktopRoute union with no section, i.e. Overview.
    const clicked = fromV4Route({ kind: 'company', slug: 'indigo' });
    expect(clicked).toEqual({ kind: 'company', slug: 'indigo' } satisfies DesktopRoute);

    const secondary = getDesktopSecondarySidebar(clicked, companies);
    expect(secondary?.surface).toBe('company');
    expect(secondary?.header).toBe('Indigo');
    expect(secondary?.items.map((item) => item.label)).toEqual([
      'Overview',
      'Accounts',
      'Goals',
      'Projects',
      'Tasks',
      'Activity',
      'Deployments',
      'Secrets',
      'Library',
      'Files',
    ]);
    expect(secondary?.activeId).toBe('overview');
  });

  it('shows the secondary sidebar only on company / library / settings surfaces', () => {
    const companies = [workspace({})];
    for (const route of [
      { kind: 'home' },
      { kind: 'companies' },
      { kind: 'messages' },
      { kind: 'meetings' },
      { kind: 'moderation' },
    ] satisfies DesktopRoute[]) {
      expect(getDesktopSecondarySidebar(route, companies)).toBeNull();
    }
    expect(getDesktopSecondarySidebar({ kind: 'library' }, companies)).not.toBeNull();
    expect(getDesktopSecondarySidebar({ kind: 'settings' }, companies)).not.toBeNull();
    expect(
      getDesktopSecondarySidebar({ kind: 'company', slug: 'indigo' }, companies),
    ).not.toBeNull();
  });

  it('DesktopApp composes the V4 chrome (title bar + both sidebars) and drops the old chrome', () => {
    const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');

    expect(desktopApp).toContain('<V4TitleBar');
    expect(desktopApp).toContain('<V4Sidebar');
    expect(desktopApp).toContain('let companies = $state<Workspace[]>(cachedCompanies)');
    expect(desktopApp).toContain('const nextCompanies = getDesktopCompanies(result.workspaces)');
    expect(desktopApp).toContain('companies = nextCompanies');
    expect(desktopApp).toContain('const shellCompanies = $derived');
    expect(desktopApp).toContain('const watchedWorkspaceCount = $derived(shellCompanies.length)');
    expect(desktopApp).toContain(
      'let renderCompanies = $state<Workspace[]>(cachedCompanies)',
    );
    expect(desktopApp).toContain('let renderWorkspaceCount = $state(cachedCompanies.length)');
    expect(desktopApp).toContain('renderCompanies = nextCompanies');
    expect(desktopApp).toContain('renderWorkspaceCount = nextCompanies.length');
    expect(desktopApp).toContain('writeCachedWorkspaces(result.workspaces)');
    // The chrome refreshes reactively from renderCompanies / renderWorkspaceCount,
    // so the desktop must NOT hard-reload the document or remount the chrome on a
    // workspace-list change — that mid-paint reload was the blank/freeze
    // (see desktop-render-stability.spec.ts).
    expect(desktopApp).not.toContain('window.location.reload()');
    expect(desktopApp).toContain('companies={renderCompanies}');
    expect(desktopApp).toContain('workspaceCount={renderWorkspaceCount}');
    expect(desktopApp).not.toContain('{#key renderWorkspaceCount}');
    expect(desktopApp).not.toContain('chromeReady');
    expect(desktopApp).not.toContain('companies={workspaces}');
    // The secondary sidebar is composed conditionally; the settings surface is
    // suppressed until its in-window page (US-013) is wired, so match the guard
    // by prefix rather than the exact unconditional `{#if secondarySidebar}`.
    expect(desktopApp).toContain('{#if secondarySidebar');
    expect(desktopApp).toContain('<V4SecondarySidebar');
    expect(desktopApp).not.toContain('DesktopSidebar');
  });

  it('the sidebar renders all companies directly instead of using an overflow row', () => {
    const sidebar = readRepoFile('src/desktop-alt/v4/V4Sidebar.svelte');
    const harnessMocks = readRepoFile('dev-harness/mocks/core.ts');

    expect(sidebar).toContain('class="v4-nav v4-company-nav"');
    expect(sidebar).toContain('flex: 1 1 auto');
    expect(sidebar).toContain('overflow-y: auto');
    expect(sidebar).toContain('companies,');
    expect(sidebar).toContain('companies && companies.length > 0 ? companies : fetched');
    expect(sidebar).toContain('if (companies && companies.length > 0) return');
    expect(sidebar).not.toContain('companies = null');
    expect(sidebar).not.toContain('data-testid="v4-more-companies"');
    expect(sidebar).not.toContain('model.overflowCount');
    expect(sidebar).not.toContain('View {model.overflowCount} more companies');
    expect(harnessMocks).toContain('const HARNESS_WORKSPACES');
    expect(harnessMocks).toContain("slug: 'sender-agency'");
    expect(harnessMocks).toContain("slug: 'archive-labs'");
  });

  it('the status bar derives live fallback values without defaulting dynamic props', () => {
    const statusBar = readRepoFile('src/desktop-alt/DesktopStatusBar.svelte');

    expect(statusBar).toContain('workspaceCount,');
    expect(statusBar).toContain('const currentWorkspaceCount = $derived(workspaceCount ?? 0)');
    expect(statusBar).toContain('{currentWorkspaceCount}</span> workspace');
    expect(statusBar).not.toContain('workspaceCount = 0');
  });

  it('the old segmented-control navigation is gone from company and library pages', () => {
    const company = readRepoFile('src/desktop-alt/pages/CompanyPage.svelte');
    const library = readRepoFile('src/desktop-alt/pages/LibraryPage.svelte');

    expect(company).not.toContain('CompanyTabs');
    expect(company).not.toContain('role="tablist"');
    // The library body forces its tab from the route, which hides
    // LibraryBrowser's in-body segmented control.
    expect(library).toContain('forcedFilter={tab}');
  });

  it('settings uses the V4 secondary sidebar instead of rendering a second in-page index', () => {
    const settings = readRepoFile('src/desktop-alt/pages/SettingsPage.svelte');

    expect(settings).not.toContain('class="settings-index"');
    expect(settings).not.toContain('grid-template-columns: 180px minmax');
  });
});
