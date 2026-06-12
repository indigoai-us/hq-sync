import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../src/lib/workspaces';
import {
  COMPANY_SECTIONS,
  getDesktopActiveCompany,
  getDesktopCompanies,
  getDesktopSecondarySidebar,
} from '../../src/desktop-alt/route';
import { emptyCompanySummary } from '../../src/desktop-alt/lib/company-summary.svelte';

const companyPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompanyPage.svelte'),
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
    invitedBy: null,
    invitedAt: null,
    ...overrides,
  };
}

describe('US-007: Company page shell — V4 sections + crumb (sections moved to the secondary sidebar in US-002)', () => {
  it('renders the company shell with the crumb and the 8 sections when a sidebar company is selected', () => {
    const workspaces: Workspace[] = [
      workspace({ slug: 'personal', displayName: 'Personal', kind: 'personal', role: null }),
      workspace({ slug: 'acme', displayName: 'Acme Corp', role: 'admin' }),
    ];
    const companies = getDesktopCompanies(workspaces);

    expect(getDesktopActiveCompany({ kind: 'company', slug: 'acme' }, companies)).toMatchObject({
      slug: 'acme',
      role: 'admin',
    });

    const page = normalize(companyPage);
    expect(page).toContain('<h1 id="company-page-title" class="visually-hidden">{company.displayName}</h1>');
    expect(page).toContain('<header class="company-actions-row">');
    // Company actions are wired to the HQ web console, in-desktop Settings route,
    // and the Claude Code /plan workflow.
    expect(page).toContain("import { open as openExternal } from '@tauri-apps/plugin-shell';");
    expect(page).toContain('<button type="button" onclick={openInvite}>Invite</button>');
    expect(page).toContain('<button type="button" onclick={openCompanySettings}>Settings</button>');
    expect(page).toContain('onclick={() => void startNewProject()}');

    // The sections live in the V4 secondary sidebar — 8 of them, Overview
    // first/default, role surfaced in the header meta line.
    expect(COMPANY_SECTIONS.map((section) => section.id)).toEqual([
      'overview',
      'goals',
      'projects',
      'tasks',
      'activity',
      'deployments',
      'secrets',
      'library',
    ]);
    const secondary = getDesktopSecondarySidebar({ kind: 'company', slug: 'acme' }, companies);
    expect(secondary?.header).toBe('Acme Corp');
    expect(secondary?.meta).toContain('Admin');
    expect(secondary?.activeId).toBe('overview');
  });

  it('swaps the selected panel when a section is selected from the secondary sidebar', () => {
    const page = normalize(companyPage);
    const desktop = normalize(desktopApp);

    // The route drives the section; the in-page segmented control is gone.
    expect(page).toContain('tab = DEFAULT_COMPANY_TAB');
    expect(page).not.toContain('CompanyTabs');
    expect(desktop).toContain("navigate({ kind: 'company', slug: route.slug, tab: id as CompanyTab })");
    expect(desktop).toContain('<CompanyPage');
    expect(desktop).toContain('company={activeCompany}');
    expect(desktop).toContain('tab={companyTab}');

    // The company page opens on the Overview board (company-scoped goals/
    // projects/in-flight via CompanyBoardPanel). The old flat vault BoardPanel
    // stays retired.
    expect(page).not.toContain('<BoardPanel slug={company.slug} />');
    expect(page).toContain("import SecretsPanel from '../panels/SecretsPanel.svelte'");
    expect(page).toContain('<CompanyBoardPanel slug={company.slug} {cloudBacked} />');
    expect(page).toContain('<ActivityPanel slug={company.slug} {cloudBacked} />');
    expect(page).toContain('<DeploymentsPanel slug={company.slug} {cloudBacked} />');
    expect(page).toContain('<SecretsPanel slug={company.slug} {cloudBacked} />');
  });

  it('wires company metadata plus workspace role propagation', () => {
    const page = normalize(companyPage);
    const summary = normalize(companySummary);
    const desktop = normalize(desktopApp);
    const workspaceSrc = normalize(workspaceTypes);
    const rustWorkspaces = normalize(workspaceCommand);
    const rustDesktopAlt = normalize(desktopAltCommand);
    const rustMain = normalize(tauriMain);

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
    expect(page).toContain('onopencompanysettings?: () => void;');
    expect(page).toContain('onopenprojects?: () => void;');
    expect(page).toContain("const settings = await invoke<SettingsWire>('get_settings').catch(() => ({ hqPath: null }));");

    expect(desktop).toContain("invoke<WorkspacesResult>('list_syncable_workspaces')");
    expect(workspaceSrc).toContain('role: string | null;');
    expect(rustWorkspaces).toContain('pub role: Option<String>');
    // US-004 (V4) hoisted the per-slug membership lookup so role + invite
    // metadata derive from one find — same propagation, new contract.
    expect(rustWorkspaces).toMatch(
      /let membership_for_slug = cloud_entity_for_slug[\s\S]*let role = membership_for_slug\.and_then\(\|m\| m\.role\.clone\(\)\)/,
    );
  });
});
