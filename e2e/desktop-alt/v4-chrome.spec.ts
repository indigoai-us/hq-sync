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
  it('a company row click opens the company page with the 8 sections and Overview active', () => {
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
      'Goals',
      'Projects',
      'Tasks',
      'Activity',
      'Deployments',
      'Secrets',
      'Library',
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
    // The secondary sidebar is composed conditionally; the settings surface is
    // suppressed until its in-window page (US-013) is wired, so match the guard
    // by prefix rather than the exact unconditional `{#if secondarySidebar}`.
    expect(desktopApp).toContain('{#if secondarySidebar');
    expect(desktopApp).toContain('<V4SecondarySidebar');
    expect(desktopApp).not.toContain('DesktopSidebar');
  });

  it('the sidebar overflow row clearly opens the Companies overview', () => {
    const sidebar = readRepoFile('src/desktop-alt/v4/V4Sidebar.svelte');
    const harnessMocks = readRepoFile('dev-harness/mocks/core.ts');

    expect(sidebar).toContain('data-testid="v4-more-companies"');
    expect(sidebar).toContain('aria-label={`View ${model.overflowCount} more companies`}');
    expect(sidebar).toContain('View {model.overflowCount} more companies');
    expect(sidebar).toContain("onclick={() => go('companies')}");
    expect(harnessMocks).toContain('const HARNESS_WORKSPACES');
    expect(harnessMocks).toContain("slug: 'sender-agency'");
    expect(harnessMocks).toContain("slug: 'archive-labs'");
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
});
