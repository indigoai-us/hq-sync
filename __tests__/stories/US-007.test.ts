import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../src/lib/workspaces';
import {
  getDesktopActiveCompany,
  getDesktopCompanies,
  getDesktopSidebarRows,
  initialDesktopRoute,
} from '../../src/desktop-alt/route';
import { emptyCompanySummary } from '../../src/desktop-alt/lib/company-summary.svelte';

const companyPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompanyPage.svelte'),
  'utf8',
);
const companyTabs = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/CompanyTabs.svelte'),
  'utf8',
);
const companySummary = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/lib/company-summary.svelte.ts'),
  'utf8',
);
const desktopApp = readFileSync(resolve(process.cwd(), 'src/desktop-alt/DesktopApp.svelte'), 'utf8');
const workspaceTypes = readFileSync(resolve(process.cwd(), 'src/lib/workspaces.ts'), 'utf8');
const workspaceCommand = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/commands/workspaces.rs'),
  'utf8',
);
const desktopAltCommand = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/commands/desktop_alt.rs'),
  'utf8',
);
const tauriMain = readFileSync(resolve(process.cwd(), 'src-tauri/src/main.rs'), 'utf8');

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

function workspace(overrides: Partial<Workspace>): Workspace {
  return {
    slug: 'acme',
    displayName: 'Acme Corp',
    kind: 'company',
    state: 'synced',
    cloudUid: 'cloud-acme',
    bucketName: 'hq-acme',
    hasLocalFolder: true,
    localPath: '/Users/test/HQ/companies/acme',
    membershipStatus: 'active',
    role: 'admin',
    lastSyncedAt: null,
    brokenReason: null,
    ...overrides,
  };
}

describe('US-007: Company page shell — tabs + crumb + role pill', () => {
  it('renders the company shell with the 4 tabs and the crumb when a sidebar company is selected', () => {
    const workspaces: Workspace[] = [
      workspace({ slug: 'personal', displayName: 'Personal', kind: 'personal', role: null }),
      workspace({ slug: 'acme', displayName: 'Acme Corp', role: 'admin' }),
    ];
    const companies = getDesktopCompanies(workspaces);
    const acmeRow = getDesktopSidebarRows(initialDesktopRoute, companies).find(
      (row) => row.label === 'Acme Corp',
    );

    expect(acmeRow?.route).toEqual({ kind: 'company', slug: 'acme' });
    expect(getDesktopActiveCompany(acmeRow!.route, companies)).toMatchObject({
      slug: 'acme',
      role: 'admin',
    });

    const page = normalize(companyPage);
    const tabs = normalize(companyTabs);
    expect(page).toContain('<span>Companies</span> <span aria-hidden="true">›</span> <span>{company.displayName}</span>');
    expect(page).toContain('<h1 id="company-page-title">{company.displayName}</h1>');
    expect(page).toContain('board cards ·');
    expect(page).toContain('activity this week ·');
    expect(page).toContain('deployments ·');
    expect(page).toContain('secrets');
    // US-001 wired these to the Tauri shell opener (HQ web console + invite).
    expect(page).toContain("import { open as openExternal } from '@tauri-apps/plugin-shell';");
    expect(page).toContain('<button type="button" onclick={openInBrowser}>Open in browser</button>');
    expect(page).toContain('<button type="button" onclick={openInvite}>Invite</button>');
    // The board lives on the company page again (company-scoped): Board is the
    // first/default tab, ahead of Activity / Deployments / Secrets.
    expect(tabs).toContain("{ id: 'board' as const, label: 'Board', count: summary.board }");
    expect(tabs).toContain("{ id: 'activity' as const, label: 'Activity', count: summary.activity.last7d }");
    expect(tabs).toContain("{ id: 'deployments' as const, label: 'Deployments', count: summary.deployments }");
    expect(tabs).toContain("{ id: 'secrets' as const, label: 'Secrets', count: summary.secrets }");
  });

  it('moves the active tab indicator and swaps the selected panel when a tab is selected', () => {
    const page = normalize(companyPage);
    const tabs = normalize(companyTabs);

    expect(page).toContain("let activeTab = $state<CompanyTab>('board')");
    expect(page).toContain('function selectTab(tab: CompanyTab) { activeTab = tab; }');
    expect(page).toContain("import SecretsPanel from '../panels/SecretsPanel.svelte'");
    expect(page).toContain('<CompanyTabs {activeTab} summary={summaryState.summary} role={company.role} onselect={selectTab} />');
    expect(tabs).toContain('aria-selected={activeTab === tab.id}');
    expect(tabs).toContain('class:active={activeTab === tab.id}');
    expect(tabs).toContain('onclick={() => onselect(tab.id)}');
    expect(tabs).toContain('.company-tabs button.active::after');
    // The company page opens on the Board (company-scoped goals/projects/in-flight
    // via CompanyBoardPanel). The old flat vault BoardPanel is retired.
    expect(page).not.toContain('<BoardPanel slug={company.slug} />');
    expect(page).toContain('<CompanyBoardPanel slug={company.slug} />');
    expect(page).toContain('<ActivityPanel slug={company.slug} />');
    expect(page).toContain('<DeploymentsPanel slug={company.slug} />');
    expect(page).toContain('<SecretsPanel slug={company.slug} />');
  });

  it('resets to the board tab on company navigation and wires summary counts plus workspace role propagation', () => {
    const page = normalize(companyPage);
    const tabs = normalize(companyTabs);
    const summary = normalize(companySummary);
    const desktop = normalize(desktopApp);
    const workspace = normalize(workspaceTypes);
    const rustWorkspaces = normalize(workspaceCommand);
    const rustDesktopAlt = normalize(desktopAltCommand);
    const rustMain = normalize(tauriMain);

    expect(page).toContain('let previousSlug = $state<string | null>(null)');
    expect(page).toContain('if (company.slug !== previousSlug) { previousSlug = company.slug; activeTab = \'board\'; }');
    expect(page).toContain('const summaryState = useCompanySummary({ slug: () => company.slug })');

    expect(emptyCompanySummary()).toEqual({
      board: 0,
      activity: { last7d: 0 },
      deployments: 0,
      secrets: 0,
    });
    expect(summary).toContain("void invoke<CompanySummary>('get_company_summary', { slug })");
    expect(summary).toContain('summary = emptyCompanySummary();');
    // company-summary was refactored from an effect-cleanup `cancelled` flag to
    // a monotonic request id that discards out-of-order completions.
    expect(summary).toContain('const myRequest = ++requestId;');
    expect(summary).toContain('if (myRequest === requestId) {');
    expect(rustDesktopAlt).toContain('pub struct CompanySummary');
    expect(rustDesktopAlt).toContain('pub async fn get_company_summary(slug: String) -> Result<CompanySummary, String>');
    expect(rustMain).toContain('commands::desktop_alt::get_company_summary');

    expect(desktop).toContain("invoke<WorkspacesResult>('list_syncable_workspaces')");
    expect(workspace).toContain('role: string | null;');
    expect(rustWorkspaces).toContain('pub role: Option<String>');
    expect(rustWorkspaces).toMatch(/let role = cloud_entity_for_slug[\s\S]*\.and_then\(\|m\| m\.role\.clone\(\)\)/);
    expect(page).toContain('role={company.role}');
    expect(tabs).toContain("const roleLabel = $derived(role ? role : 'No role')");
    expect(tabs).toContain('<span class="role-pill" title="Workspace role">{roleLabel}</span>');
  });
});
