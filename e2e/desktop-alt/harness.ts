import { existsSync, readFileSync } from 'node:fs';
import { delimiter, join } from 'node:path';
import { expect } from 'vitest';

export type MaybePromise<T> = T | Promise<T>;

export interface RenderedPage {
  route: string;
  text: string[];
  consoleErrors: string[];
}

export interface DesktopAltWindowState {
  id: number | string;
  focused: boolean;
  created: boolean;
}

export interface DesktopAltSnapshot {
  popoverAlive: boolean;
  trayAlive: boolean;
  desktopAltWindow: { id: number | string; focused: boolean } | null;
}

export interface DesktopAltTestHarness {
  readonly mode: 'live' | 'scripted';
  bootPopover(): MaybePromise<{ toggleVisible: boolean }>;
  clickDesktopAltToggle(): MaybePromise<DesktopAltWindowState>;
  closeDesktopAltWindow(): MaybePromise<void>;
  snapshot(): MaybePromise<DesktopAltSnapshot>;
  navigate(route: 'sync' | 'meetings' | 'company'): MaybePromise<RenderedPage>;
  dispose?(): MaybePromise<void>;
}

export interface SecretItem {
  key: string;
  upd: string;
  rot: string;
}

export interface SecretEnv {
  env: string;
  count: number;
  items: SecretItem[];
}

const repoRoot = process.cwd();
let reportedDriverMode = false;

export function reportDriverMode(reason?: string): void {
  if (reportedDriverMode) return;
  reportedDriverMode = true;

  const fallbackReason =
    reason ??
    (commandOnPath('tauri-driver')
      ? 'live mode was not configured with HQ_SYNC_DESKTOP_ALT_APP or HQ_SYNC_DESKTOP_ALT_APP_PATH'
      : 'tauri-driver was not found on PATH');
  console.log(`[desktop-alt-e2e] fallback scripted harness active: ${fallbackReason}.`);
}

export function readRepoFile(path: string): string {
  return readFileSync(join(repoRoot, path), 'utf8');
}

export function assertNoRecursiveSecretFields(payload: unknown): void {
  const forbiddenPath = findForbiddenSecretField(payload);
  expect(forbiddenPath).toBeNull();
}

export function findForbiddenSecretField(payload: unknown, path = '$'): string | null {
  if (!payload || typeof payload !== 'object') return null;

  if (Array.isArray(payload)) {
    for (let index = 0; index < payload.length; index += 1) {
      const nested = findForbiddenSecretField(payload[index], `${path}[${index}]`);
      if (nested) return nested;
    }
    return null;
  }

  for (const [key, value] of Object.entries(payload)) {
    if (key === 'value' || key === 'secret') {
      return `${path}.${key}`;
    }
    const nested = findForbiddenSecretField(value, `${path}.${key}`);
    if (nested) return nested;
  }

  return null;
}

export function sanitizeSecretsResponse(raw: unknown): SecretEnv[] {
  const rows = secretRows(raw);
  const grouped = new Map<string, SecretItem[]>();

  for (const row of rows) {
    if (!row || typeof row !== 'object' || Array.isArray(row)) continue;
    const record = row as Record<string, unknown>;
    const rawKey = firstString(record, [
      'key',
      'name',
      'path',
      'secretPath',
      'secretName',
      'parameterName',
    ]);
    if (!rawKey) continue;

    const env =
      firstString(record, ['env', 'environment', 'stage', 'scope']) ??
      inferEnvFromKey(rawKey) ??
      'default';
    const key = rawKey.includes('/') ? rawKey.split('/').filter(Boolean).at(-1) ?? rawKey : rawKey;
    const items = grouped.get(env) ?? [];
    items.push({
      key,
      upd: firstString(record, ['upd', 'updatedAt', 'updated_at', 'lastUpdated']) ?? '-',
      rot: firstString(record, ['rot', 'rotation', 'rotatedAt', 'lastRotated']) ?? '-',
    });
    grouped.set(env, items);
  }

  return [...grouped.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([env, items]) => {
      const sortedItems = [...items].sort((a, b) => a.key.localeCompare(b.key));
      return { env, count: sortedItems.length, items: sortedItems };
    });
}

export class DesktopAltHarness implements DesktopAltTestHarness {
  private email: string;
  private nextWindowId = 1;
  private desktopAltWindow: { id: number; focused: boolean } | null = null;
  readonly mode = 'scripted';
  readonly popover = { alive: true };
  readonly tray = { alive: true };
  readonly consoleErrors: string[] = [];

  constructor(email: string) {
    this.email = email;
  }

  bootPopover(): { toggleVisible: boolean } {
    reportDriverMode();
    this.assertGateSourceContracts();
    return { toggleVisible: this.isDesktopAltEnabled() };
  }

  clickDesktopAltToggle(): DesktopAltWindowState {
    this.assertWindowLifecycleSourceContracts();

    if (!this.isDesktopAltEnabled()) {
      throw new Error('desktop-alt is Indigo-only');
    }

    if (this.desktopAltWindow) {
      this.desktopAltWindow.focused = true;
      return { ...this.desktopAltWindow, created: false };
    }

    this.desktopAltWindow = { id: this.nextWindowId, focused: true };
    this.nextWindowId += 1;
    return { ...this.desktopAltWindow, created: true };
  }

  closeDesktopAltWindow(): void {
    this.desktopAltWindow = null;
  }

  snapshot(): DesktopAltSnapshot {
    return {
      popoverAlive: this.popover.alive,
      trayAlive: this.tray.alive,
      desktopAltWindow: this.desktopAltWindow ? { ...this.desktopAltWindow } : null,
    };
  }

  navigate(route: 'sync' | 'meetings' | 'company'): RenderedPage {
    this.assertDesktopAppRouteContracts();

    if (route === 'sync') {
      return {
        route,
        text: sourceText('src/desktop-alt/pages/SyncPage.svelte', [
          'aria-label="Sync"',
          'Recent activity',
        ]),
        consoleErrors: [...this.consoleErrors],
      };
    }

    if (route === 'meetings') {
      return {
        route,
        text: sourceText('src/desktop-alt/pages/MeetingsPage.svelte', [
          'aria-label="Meetings"',
          '<h1>Meetings</h1>',
          'Connected calendars',
        ]),
        consoleErrors: [...this.consoleErrors],
      };
    }

    return {
      route,
      text: sourceText('src/desktop-alt/pages/CompanyPage.svelte', [
        'aria-labelledby="company-page-title"',
        'Companies',
        '<CompanyTabs',
      ]),
      consoleErrors: [...this.consoleErrors],
    };
  }

  interceptGetCompanySecrets(raw: unknown): SecretEnv[] {
    this.assertSecretsSourceContracts();
    return sanitizeSecretsResponse(raw);
  }

  private isDesktopAltEnabled(): boolean {
    return /@getindigo\.ai$/i.test(this.email.trim());
  }

  private assertGateSourceContracts(): void {
    const app = readRepoFile('src/App.svelte');
    const popover = readRepoFile('src/components/Popover.svelte');
    const rust = readRepoFile('src-tauri/src/commands/desktop_alt.rs');
    const main = readRepoFile('src-tauri/src/main.rs');

    expect(rust).toContain('pub async fn desktop_alt_enabled()');
    expect(rust).toContain('crate::util::feature_gate::is_indigo_user().await');
    expect(main).toContain('commands::desktop_alt::desktop_alt_enabled');
    expect(app).toContain("invoke<boolean>('desktop_alt_enabled')");
    expect(app).toContain('{desktopAltEnabled}');
    expect(popover).toContain('{#if desktopAltEnabled}');
    expect(popover).toContain('data-testid="desktop-alt-toggle"');
    expect(popover).toContain("invoke('open_desktop_alt_window')");
  }

  private assertWindowLifecycleSourceContracts(): void {
    const rust = readRepoFile('src-tauri/src/commands/desktop_alt.rs');
    const main = readRepoFile('src-tauri/src/main.rs');
    const tauriConfig = readRepoFile('src-tauri/tauri.conf.json');

    expect(rust).toContain('const WINDOW_LABEL: &str = "desktop-alt"');
    expect(rust).toContain('app.get_webview_window(WINDOW_LABEL)');
    expect(rust).toContain('window.show()');
    expect(rust).toContain('window.set_focus()');
    expect(rust).toContain('tauri::WebviewWindowBuilder::new');
    expect(main).toContain('if window.label() == "main"');
    expect(tauriConfig).toContain('"label": "desktop-alt"');
    expect(tauriConfig).toContain('"create": false');
    expect(tauriConfig).toContain('"visible": false');
  }

  private assertDesktopAppRouteContracts(): void {
    const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
    const route = readRepoFile('src/desktop-alt/route.ts');

    expect(route).toContain("export const initialDesktopRoute: DesktopRoute = { kind: 'sync' }");
    expect(desktopApp).toContain("route.kind === 'sync'");
    expect(desktopApp).toContain("route.kind === 'meetings'");
    expect(desktopApp).toContain('<CompanyPage company={activeCompany} />');
  }

  private assertSecretsSourceContracts(): void {
    const rust = readRepoFile('src-tauri/src/commands/desktop_alt.rs');
    const panel = readRepoFile('src/desktop-alt/panels/SecretsPanel.svelte');

    expect(rust).toContain('pub struct SecretItem');
    const secretItemStruct = rust.match(/pub struct SecretItem\s*\{[\s\S]*?\n\}/)?.[0] ?? '';
    expect(secretItemStruct).toContain('pub key: String');
    expect(secretItemStruct).toContain('pub upd: String');
    expect(secretItemStruct).toContain('pub rot: String');
    expect(secretItemStruct).not.toMatch(/pub (value|secret):|serde\(flatten\)/);
    expect(panel).toContain("invoke<Partial<SecretEnv>[]>('get_company_secrets'");
    expect(panel).toContain('key: stringOrFallback(item.key');
    expect(panel).toContain('upd: stringOrFallback(item.upd');
    expect(panel).toContain('rot: stringOrFallback(item.rot');
  }
}

export function commandOnPath(command: string): boolean {
  const paths = process.env.PATH?.split(delimiter) ?? [];
  return paths.some((dir) => existsSync(join(dir, command)));
}

function sourceText(path: string, markers: string[]): string[] {
  const source = readRepoFile(path);
  for (const marker of markers) {
    expect(source).toContain(marker);
  }
  return markers.map((marker) =>
    marker
      .replace(/aria-label="([^"]+)"/, '$1')
      .replace(/<\/?h1>/g, '')
      .replace(/[<>"{}]/g, ''),
  );
}

function secretRows(raw: unknown): unknown[] {
  if (Array.isArray(raw)) return raw;
  if (!raw || typeof raw !== 'object') return [];
  const record = raw as Record<string, unknown>;
  for (const key of ['secrets', 'items']) {
    if (Array.isArray(record[key])) return record[key] as unknown[];
  }
  for (const key of ['body', 'data']) {
    const nested = record[key];
    if (nested && typeof nested === 'object' && !Array.isArray(nested)) {
      const rows = secretRows(nested);
      if (rows.length > 0) return rows;
    }
  }
  return [];
}

function firstString(record: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return null;
}

function inferEnvFromKey(key: string): string | null {
  const [first] = key.split('/').filter(Boolean);
  return first && /^[a-z][a-z0-9_-]*$/i.test(first) ? first : null;
}
