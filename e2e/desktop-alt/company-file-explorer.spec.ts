import { describe, expect, it } from 'vitest';
import {
  COMPANY_SECTIONS,
  getDesktopSecondarySidebar,
} from '../../src/desktop-alt/route';
import type { Workspace } from '../../src/lib/workspaces';
import { readRepoFile } from './harness';

/**
 * US-009 — Top-level Files mode: explorer sidebar + company switcher.
 *
 * Source-contract harness (same style as file-preview.spec.ts). Does NOT mount
 * components — asserts on source text to lock down the Files-mode wiring so the
 * explorer sidebar, the connected-first company switcher, and the
 * select→preview data flow into the MAIN content area don't regress. Also
 * verifies the per-company Files secondary-sidebar tab is GONE.
 *
 * Covers the three US-009 e2eTests:
 *   1. Given Files mode, when opened, the sidebar shows a connected-first
 *      company list + a tree (not the company nav).
 *   2. Given a company is picked in Files mode, when a file is selected, the
 *      preview renders in the main area.
 *   3. Given a company view, when its secondary sidebar renders, there is NO
 *      'Files' item.
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

describe('desktop-alt Files mode — explorer sidebar + company switcher (US-009)', () => {
  const sidebar = readRepoFile('src/desktop-alt/v4/FilesModeSidebar.svelte');
  const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
  const route = readRepoFile('src/desktop-alt/route.ts');
  const model = readRepoFile('src/desktop-alt/v4/model.ts');

  // -------------------------------------------------------------------------
  // e2eTest 1: Files mode shows a connected-first company list + a tree
  // -------------------------------------------------------------------------
  it('FilesModeSidebar renders a connected-first company list and fetches the company tree', () => {
    // Reuses the presentational tree component (28px fixed rows).
    expect(sidebar).toContain(
      "import CompanyFileTree from '../components/CompanyFileTree.svelte'",
    );
    // Fetches the (noise-filtered) tree for the active company slug.
    expect(sidebar).toContain("invoke<FileNode>('get_company_file_tree', { slug })");
    // Uses the SHARED connected-first sort so ordering matches the primary
    // sidebar (US-007), not a duplicated sort.
    expect(sidebar).toContain(
      "import { sortV4CompaniesConnectedFirst } from './model'",
    );
    expect(sidebar).toContain('sortV4CompaniesConnectedFirst(companies, activeSlug)');
    // The mini company list rows fire onselectcompany; the tree is rendered
    // only when it has children, with loading/empty states otherwise.
    expect(sidebar).toContain('onselectcompany?.(row.slug)');
    expect(sidebar).toContain('<CompanyFileTree');
    expect(sidebar).toContain('tree && tree.children.length > 0');
    expect(sidebar).toContain('No files yet');
  });

  it('the model exports the shared connected-first sort used by both sidebars', () => {
    expect(model).toContain('export function sortV4CompaniesConnectedFirst(');
    // The primary sidebar model delegates to it (no duplicated sort logic).
    expect(model).toContain('sortV4CompaniesConnectedFirst(');
  });

  it('DesktopApp swaps the primary sidebar for FilesModeSidebar in Files mode', () => {
    expect(desktopApp).toContain(
      "import FilesModeSidebar from './v4/FilesModeSidebar.svelte'",
    );
    expect(desktopApp).toContain("{#if route.kind === 'files'}");
    expect(desktopApp).toContain('<FilesModeSidebar');
    expect(desktopApp).toContain('activeSlug={filesActiveSlug}');
    // Picking a company navigates to a files route for that company (clearing
    // any selected file — no path on the company switch).
    expect(desktopApp).toContain("onselectcompany={(slug) => navigate({ kind: 'files', slug })}");
  });

  // -------------------------------------------------------------------------
  // e2eTest 2: selecting a file renders the preview in the MAIN content area
  // -------------------------------------------------------------------------
  it('a file select drives the FilePreviewPane in the main content area', () => {
    // The sidebar fires onselectfile when a tree file row is selected.
    expect(sidebar).toContain('onselectfile?: (path: string) => void');
    expect(sidebar).toContain('onselect={handleSelectFile}');
    // The shell turns a file select into a files route carrying the path.
    expect(desktopApp).toContain('onselectfile={(path) =>');
    expect(desktopApp).toContain("navigate({ kind: 'files', slug: filesActiveSlug ?? undefined, path })");
    // The main area renders FilePreviewPane driven by the selected path, with a
    // friendly prompt before any selection.
    expect(desktopApp).toContain("import FilePreviewPane from './components/FilePreviewPane.svelte'");
    expect(desktopApp).toContain('<FilePreviewPane path={filesSelectedPath}');
    expect(desktopApp).toContain('{#if filesSelectedPath}');
    expect(desktopApp).toContain('Select a file to preview it');
  });

  it('the files route + Files nav destination exist in the IA', () => {
    // route.ts declares the files route kind.
    expect(route).toContain("kind: 'files'");
    expect(route).toMatch(/kind === 'files'/);
    // model.ts / V4_NAV_ITEMS includes Files as a primary destination.
    expect(model).toContain("{ id: 'files', label: 'Files' }");
    expect(model).toContain("| 'files'");
  });

  it('Files mode survives a desktop-alt window reload via persisted route state', () => {
    expect(desktopApp).toContain('hq-sync.desktop.route.v1');
    expect(desktopApp).toContain('readStoredFilesRoute');
  });

  // -------------------------------------------------------------------------
  // e2eTest 3: the company secondary sidebar has NO 'Files' item
  // -------------------------------------------------------------------------
  it("the company secondary sidebar no longer exposes a 'Files' item", () => {
    const companies = [workspace({})];
    const secondary = getDesktopSecondarySidebar({ kind: 'company', slug: 'indigo' }, companies);
    expect(secondary?.surface).toBe('company');
    expect(secondary?.items.some((item) => item.label === 'Files')).toBe(false);
    // COMPANY_SECTIONS no longer carries the 'files' tab.
    expect(COMPANY_SECTIONS.some((section) => (section.id as string) === 'files')).toBe(false);
    // route.ts no longer declares the company Files section.
    expect(route).not.toContain("{ id: 'files', label: 'Files' }");
  });
});
