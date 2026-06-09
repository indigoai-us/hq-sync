import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * US-009 (hq-pack-porter) — Unify the Packages surface into the Marketplace /
 * Library area.
 *
 * The standalone Packages window (a separate Tauri window opened from Settings)
 * is removed as a distinct destination, and its function is merged into the
 * desktop-alt **Library** area as a new **Installed** tab — so installed packs
 * AND browsable/marketplace packs now live in ONE coherent surface, with no
 * duplicate package UIs. This consolidates (does NOT regress) the proj-119
 * Library/Marketplace work; the attribution byline + creator-profile link still
 * work (covered by US-019), and the install / scope-select flows are untouched.
 *
 * This repo has no DOM/component harness (vitest `environment: "node"`), so —
 * like the other US-0xx story tests — these are SOURCE-CONTRACT tests over the
 * relevant sources plus on-disk presence/absence checks.
 */

const root = process.cwd();
const read = (rel: string): string => readFileSync(resolve(root, rel), 'utf8');
const normalize = (s: string): string => s.replace(/\s+/g, ' ');

const libraryBrowser = read('src/desktop-alt/components/LibraryBrowser.svelte');
const installedPanel = read('src/desktop-alt/panels/InstalledPacksPanel.svelte');

describe('US-009: installed + marketplace packs live in one Library surface', () => {
  it('exposes an "Installed" tab alongside the existing Marketplace tab in the Library browser', () => {
    const src = normalize(libraryBrowser);
    // Both destinations are tabs of the SAME LibraryBrowser toolbar — one place.
    expect(src).toContain("{ id: 'installed', label: 'Installed' }");
    expect(src).toContain("{ id: 'marketplace', label: 'Marketplace' }");
  });

  it('renders the unified InstalledPacksPanel for the Installed tab', () => {
    const src = normalize(libraryBrowser);
    expect(src).toContain("import InstalledPacksPanel from '../panels/InstalledPacksPanel.svelte'");
    // The Installed tab is a self-contained body (its own fetch + actions), so it
    // is part of the standalone-tab set and renders the panel.
    expect(src).toContain("const isInstalled = $derived(filter === 'installed');");
    expect(src).toContain('isInstalled || isMarketplace');
    expect(src).toContain('{#if isInstalled}');
    expect(src).toContain('<InstalledPacksPanel />');
  });

  it('the Installed panel reuses the SAME package commands the old window used (no behaviour change)', () => {
    const src = normalize(installedPanel);
    // The pack lifecycle commands are byte-for-byte the ones the standalone
    // window invoked — only the host surface changed.
    expect(src).toContain("invoke<PackagesView>('list_packages')");
    expect(src).toContain("invoke('install_package', { source, registry })");
    expect(src).toContain("invoke('update_package', { name })");
    expect(src).toContain("invoke('uninstall_package', { name })");
    expect(src).toContain("invoke('check_package_updates')");
  });

  it('shows BOTH installed and available/marketplace-sourced packs in the one panel', () => {
    const src = normalize(installedPanel);
    // Installed packs group + an available-to-install group in the same surface.
    expect(src).toContain('const installed = $derived(view?.packs?.installed ?? []);');
    expect(src).toContain('const available = $derived(view?.packs?.available ?? []);');
    expect(src).toContain('data-testid="installed-group"');
  });
});

describe('US-009: the standalone Packages destination is removed', () => {
  it('no longer ships the standalone Packages window app or its HTML entry', () => {
    // The dedicated window's Svelte app, its mount entry, and its HTML are gone.
    expect(existsSync(resolve(root, 'packages.html'))).toBe(false);
    expect(existsSync(resolve(root, 'src/packages/PackagesApp.svelte'))).toBe(false);
    expect(existsSync(resolve(root, 'src/packages/main.ts'))).toBe(false);
    // And its capability file (windows: ["packages"]) is removed too.
    expect(existsSync(resolve(root, 'src-tauri/capabilities/packages.json'))).toBe(false);
  });

  it('drops the packages window from the Vite build inputs and the Tauri window config', () => {
    const vite = read('vite.config.ts');
    expect(vite).not.toContain('packages.html');

    const conf = read('src-tauri/tauri.conf.json');
    expect(conf).not.toContain('packages.html');
    // No window labelled "packages" remains in the app window list.
    const windows = JSON.parse(conf).app.windows as Array<{ label?: string }>;
    expect(windows.some((w) => w.label === 'packages')).toBe(false);
  });

  it('removes the window-lifecycle commands + handshake state from the Rust surface', () => {
    const rust = read('src-tauri/src/commands/packages.rs');
    // The lifecycle commands and the ready-handshake managed state are gone …
    expect(rust).not.toContain('pub async fn open_packages_window');
    expect(rust).not.toContain('pub fn packages_window_ready');
    expect(rust).not.toContain('pub struct PendingPackages');

    const main = read('src-tauri/src/main.rs');
    // … and they are no longer registered / managed in main.rs.
    expect(main).not.toContain('open_packages_window');
    expect(main).not.toContain('packages_window_ready');
    expect(main).not.toContain('PendingPackages');
    // The data commands that now back the Library tab are still registered.
    expect(main).toContain('commands::packages::list_packages');
    expect(main).toContain('commands::packages::uninstall_package');
  });

  it('removes the Settings "Manage packages" entry that opened the standalone window', () => {
    const settings = read('src/components/Settings.svelte');
    expect(settings).not.toContain('open_packages_window');
    expect(settings).not.toContain('handleManagePackages');
  });
});

describe('US-009: no regression to design tokens or the install / scope flows', () => {
  it('the Installed panel uses design tokens only (no new hardcoded hex colors in its own rules)', () => {
    // Pull just the <style> block and assert it carries no raw #rrggbb / #rgb
    // color literals — everything routes through the desktop-alt CSS variables.
    // (#fff on a filled primary button mirrors MarketplacePanel's install button
    // and is the single allowed exception, matching the existing convention.)
    const styleMatch = installedPanel.match(/<style>([\s\S]*)<\/style>/);
    expect(styleMatch).not.toBeNull();
    const style = (styleMatch?.[1] ?? '').replace(/#fff\b/g, '');
    expect(style).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    // It honors the reduced-motion contract like its sibling panels.
    expect(style).toContain('@media (prefers-reduced-motion: reduce)');
  });

  it('does NOT touch the Marketplace install / scope-select flow (left intact for US-019/US-009-119)', () => {
    const market = read('src/desktop-alt/panels/MarketplacePanel.svelte');
    // The marketplace install + scope picker that proj-119 owns is untouched.
    expect(market).toContain('data-testid="marketplace-install-button"');
    expect(market).toContain('data-testid="marketplace-scope-select"');
    expect(market).toContain('installMarketplacePack(');
  });
});
