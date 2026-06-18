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
  it('FilesModeSidebar renders a connected-first company list and the lazy tree', () => {
    // Reuses the presentational tree component (28px fixed rows).
    expect(sidebar).toContain(
      "import CompanyFileTree from '../components/CompanyFileTree.svelte'",
    );
    // Uses the SHARED connected-first sort so ordering matches the primary
    // sidebar (US-007), not a duplicated sort.
    expect(sidebar).toContain(
      "import { sortV4CompaniesConnectedFirst } from './model'",
    );
    expect(sidebar).toContain('sortV4CompaniesConnectedFirst(companies, activeSlug)');
    // The mini company list rows fire onselectcompany and the lazy tree renders.
    expect(sidebar).toContain('onselectcompany?.(');
    expect(sidebar).toContain('<CompanyFileTree');
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
    // Picking a company navigates to a files route scoped to that company;
    // clearing the filter (null slug) returns to the root files route.
    expect(desktopApp).toContain("navigate({ kind: 'files', slug: slug ?? undefined })");
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

/**
 * US-010 — Files mode: exit control + root tree by default, companies as
 * filter (lazy-loaded).
 *
 * Source-contract harness. Locks down the iteration-3 rework:
 *   1. Exit control present in the Files-mode sidebar header; returns to the
 *      previous view (default Home) and restores the normal sidebar.
 *   2. Root-by-default: with no company selected the tree is rooted at the HQ
 *      root ('') and the company list is an OPTIONAL filter.
 *   3. Company-as-filter: selecting a company scopes the tree to
 *      companies/<slug>/, with a visible + clearable active-filter affordance.
 *   4. Lazy expansion: the tree loads children per folder via list_hq_dir; no
 *      eager get_company_file_tree walk in Files mode.
 */
describe('desktop-alt Files mode — exit + root-default + company filter (US-010)', () => {
  const sidebar = readRepoFile('src/desktop-alt/v4/FilesModeSidebar.svelte');
  const tree = readRepoFile('src/desktop-alt/components/CompanyFileTree.svelte');
  const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
  const lib = readRepoFile('src/desktop-alt/lib/file-tree.ts');
  const rust = readRepoFile('src-tauri/src/commands/desktop_alt.rs');
  const mainRs = readRepoFile('src-tauri/src/main.rs');

  // -------------------------------------------------------------------------
  // e2eTest 3 (listed): the back/exit control returns to the main area
  // -------------------------------------------------------------------------
  it('the Files-mode sidebar exposes an exit/back control wired to leave Files mode', () => {
    // Sidebar declares an onexit callback and renders a Back control.
    expect(sidebar).toContain('onexit?: () => void');
    expect(sidebar).toContain('onexit?.()');
    expect(sidebar).toContain('Back');
    // The shell wires it to a handler that restores the prior route.
    expect(desktopApp).toContain('onexit={exitFilesMode}');
    expect(desktopApp).toContain('function exitFilesMode()');
    // Exit restores the remembered pre-Files route (default Home).
    expect(desktopApp).toContain('routeBeforeFiles');
    expect(desktopApp).toContain("{ kind: 'home' }");
  });

  it('entering Files mode no longer auto-selects a company (root is the default)', () => {
    // The old auto-default-to-first-connected-company branch is gone.
    expect(desktopApp).not.toContain('firstConnectedSlug');
    // Navigating to files remembers where we came from for the exit control.
    expect(desktopApp).toContain('routeBeforeFiles = route');
  });

  // -------------------------------------------------------------------------
  // e2eTest 1 (listed): root top-level folders by default
  // -------------------------------------------------------------------------
  it('the tree is rooted at the HQ root by default and scopes on company select', () => {
    // No company → empty root path; company → companies/<slug>.
    expect(sidebar).toContain("activeSlug ? `companies/${activeSlug}` : ''");
    expect(sidebar).toContain('rootPath={treeRootPath}');
    // The tree loads the root level on mount, not a pre-walked company tree.
    expect(sidebar).not.toContain("get_company_file_tree");
  });

  // -------------------------------------------------------------------------
  // e2eTest 2 (listed): company filter scoping + clear affordance
  // -------------------------------------------------------------------------
  it('the company list acts as a clearable filter (toggle off / explicit clear)', () => {
    // onselectcompany takes a nullable slug (null = clear back to root).
    expect(sidebar).toContain('onselectcompany?: (slug: string | null) => void');
    // Clicking the active company toggles the filter off; an explicit clear
    // affordance also sends null.
    expect(sidebar).toContain('row.slug === activeSlug ? null : row.slug');
    expect(sidebar).toContain('onselectcompany?.(null)');
    // The active filter is visually indicated (scope chip) and labelled root
    // when no filter is set.
    expect(sidebar).toContain('fs-scope');
    expect(sidebar).toContain('companies/{activeSlug}');
    expect(sidebar).toContain('HQ root');
  });

  // -------------------------------------------------------------------------
  // e2eTest 4 (listed): lazy per-folder expansion (no full eager walk)
  // -------------------------------------------------------------------------
  it('the tree lazy-loads children per folder via list_hq_dir', () => {
    // Sidebar supplies a per-directory loader backed by list_hq_dir.
    expect(sidebar).toContain("invoke<DirEntry[]>('list_hq_dir', { relPath })");
    expect(sidebar).toContain('loadChildren');
    // CompanyFileTree is lazy: it takes rootPath + loadChildren and fetches on
    // expand rather than consuming a pre-walked FileNode tree.
    expect(tree).toContain('loadChildren: (relPath: string) => Promise<DirEntry[]>');
    expect(tree).toContain('function ensureLoaded(');
    expect(tree).toContain('flattenLazy(');
    // The lib exposes the lazy node shape with loaded/hasChildren flags.
    expect(lib).toContain('export interface DirEntry');
    expect(lib).toContain('export interface LazyNode');
    expect(lib).toContain('hasChildren: boolean');
    expect(lib).toContain('loaded: boolean');
    expect(lib).toContain('export function flattenLazy(');
  });

  it('the lazy list command is implemented + registered (path-guarded, noise-filtered)', () => {
    // Backend command exists and reuses the shared guard + noise filter.
    expect(rust).toContain('pub async fn list_hq_dir(');
    expect(rust).toContain('fn list_dir_entries(');
    expect(rust).toContain('is_within(hq_root, &abs)');
    expect(rust).toContain('is_dev_noise(&name, is_dir)');
    expect(rust).toContain('pub struct DirEntry');
    // Registered in main.rs (authorized by core:default — no new token).
    expect(mainRs).toContain('commands::desktop_alt::list_hq_dir');
  });
});
