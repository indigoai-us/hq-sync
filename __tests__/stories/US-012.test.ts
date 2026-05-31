import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const companyPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompanyPage.svelte'),
  'utf8',
);
const secretsPanel = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/panels/SecretsPanel.svelte'),
  'utf8',
);
const secretEnvRow = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/SecretEnvRow.svelte'),
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

function blockFrom(source: string, start: string, end: string): string {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);

  expect(startIndex).toBeGreaterThanOrEqual(0);
  expect(endIndex).toBeGreaterThan(startIndex);

  return source.slice(startIndex, endIndex);
}

describe('US-012: Secrets panel reads metadata only with no plaintext values', () => {
  it('wires the secrets tab to get_company_secrets with the selected company slug', () => {
    const page = normalize(companyPage);
    const panel = normalize(secretsPanel);

    expect(page).toContain("import SecretsPanel from '../panels/SecretsPanel.svelte'");
    expect(page).toContain('<SecretsPanel slug={company.slug} />');
    expect(panel).toContain("void invoke<Partial<SecretEnv>[]>('get_company_secrets', { slug })");
    expect(panel).toContain('return () => { cancelled = true; };');
    expect(panel).toContain('function retry() { reloadToken += 1; }');
    expect(panel).toContain("console.error('get_company_secrets failed:', err)");
    expect(tauriMain).toContain('commands::desktop_alt::get_company_secrets');
  });

  it('returns only env/count/items metadata from the Tauri command and registers no plaintext DTO fields', () => {
    const command = normalize(desktopAltCommand);

    expect(command).toContain('pub struct SecretItem { pub key: String, pub upd: String, pub rot: String, }');
    expect(command).toContain('pub struct SecretEnv { pub env: String, pub count: usize, pub items: Vec<SecretItem>, }');
    expect(command).toContain('pub async fn get_company_secrets(slug: String) -> Result<Vec<SecretEnv>, String>');
    expect(command).toContain('SecretEnv { env, count: items.len(), items, }');
    expect(command).toContain('grouped.entry(env).or_default().push(SecretItem { key, upd: secret_updated_at(row), rot: secret_rotation(row), });');
    expect(command).toContain('let serialized = serde_json::to_value(&envs).unwrap();');
    expect(command).toContain('assert!(!serialized_text.contains("\\\"value\\\""));');
    expect(command).toContain('assert!(!serialized_text.contains("\\\"secret\\\""));');
    expect(command).toContain('assert!(serialized.get(0).unwrap().get("value").is_none());');
  });

  it('uses a metadata-list GET endpoint and does not call a fetch-secret/value endpoint', () => {
    const getCompanySecrets = normalize(
      blockFrom(
        desktopAltCommand,
        'pub async fn get_company_secrets(slug: String) -> Result<Vec<SecretEnv>, String>',
        '/// Open or focus the Indigo-only alternate desktop UX window.',
      ),
    );
    const urlBuilder = normalize(
      blockFrom(desktopAltCommand, 'fn secrets_url(base: &str, company_uid: &str)', 'fn parse_board_response'),
    );

    expect(getCompanySecrets).toContain('let url = secrets_url(&vault_base()?, &company_uid)?;');
    expect(getCompanySecrets).toContain('build_client() .get(url)');
    expect(getCompanySecrets).toContain('parse_secrets_response(status, &text)');
    expect(urlBuilder).toContain('format!( "{}/secrets/{}", base.trim_end_matches(\'/\'), company_uid )');
    expect(getCompanySecrets).not.toMatch(/\.(post|put|patch)\s*\(/);
    expect(getCompanySecrets).not.toMatch(/fetch[_-]?secret|read[_-]?secret|get[_-]?secret[_-]?value/i);
    expect(urlBuilder).not.toMatch(/\/secret\/|\/value|\/reveal|\/decrypt/i);
  });

  it('renders collapsed-by-default environment rows with production sealed and non-production open pills', () => {
    const row = normalize(secretEnvRow);

    expect(row).toContain('let expanded = $state(false);');
    expect(row).toContain("return ['prod', 'production'].includes(env.trim().toLowerCase());");
    expect(row).toContain("const pill = $derived(isSealedSecretEnv(secretEnv.env) ? 'sealed' : 'open')");
    expect(row).toContain('<button class="env-button" type="button" aria-expanded={expanded} aria-controls={rowId} onclick={toggleExpanded} >');
    expect(row).toContain('<span class="env-name" title={secretEnv.env}>{secretEnv.env}</span>');
    expect(row).toContain('<span class={`env-pill ${pill}`}>{pill}</span>');
    expect(row).toContain('<span class="env-count">{secretEnv.count} keys</span>');
    expect(row).toContain('{#if expanded}');
    expect(row).toContain('.env-pill.sealed');
    expect(row).toContain('.env-pill.open');
  });

  it('expands an environment tree to key names and last-updated/rotation metadata with no value field rendered', () => {
    const row = normalize(secretEnvRow);
    const itemMarkup = blockFrom(secretEnvRow, '<div class="secret-list">', '{/each}');

    expect(row).toContain('<span>Key</span> <span>Updated</span> <span>Rotated</span>');
    expect(row).toContain('{#each secretEnv.items as item, index (`${secretEnv.env}:${item.key}:${index}`)}');
    expect(row).toContain('<span class="secret-key" title={item.key}>{item.key}</span>');
    expect(row).toContain('<time title={item.upd}>{item.upd}</time>');
    expect(row).toContain('<time title={item.rot}>{item.rot}</time>');
    expect(itemMarkup).not.toContain('item.value');
    expect(itemMarkup).not.toContain('item.secret');
    expect(itemMarkup).not.toMatch(/<span[^>]*>\s*Value\s*<\/span>|<input|<textarea/i);
  });

  it('renders read-only toolbar affordances, exact doc note, and exact empty state', () => {
    const panel = normalize(secretsPanel);

    expect(panel).toContain('Read-only metadata. Values are never sent to the client — use /hq-secrets to fetch a value.');
    expect(panel).toContain('title="Export not available — use /hq-secrets exec"');
    expect(panel).toContain('aria-label="Export not available — use /hq-secrets exec"');
    expect(panel).toContain('> Export .env </button>');
    expect(panel).toContain('title="Create from CLI: hq secrets set"');
    expect(panel).toContain('aria-label="Create from CLI: hq secrets set"');
    expect(panel).toContain('> New key </button>');
    expect(panel.match(/type="button" disabled/g)?.length).toBeGreaterThanOrEqual(2);
    expect(panel).toContain('<div class="empty-state">No secrets yet</div>');
  });

  it('normalizes and renders env rows from production/staging/preview metadata with count fallbacks', () => {
    const panel = normalize(secretsPanel);
    const row = normalize(secretEnvRow);

    expect(panel).toContain('secrets = Array.isArray(result) ? result.map(normalizeSecretEnv) : [];');
    expect(panel).toContain('const items = Array.isArray(entry.items) ? entry.items.map(normalizeSecretItem) : [];');
    expect(panel).toContain('env: stringOrFallback(entry.env, \'unknown\')');
    expect(panel).toContain('count: numberOrFallback(entry.count, items.length)');
    expect(panel).toContain('key: stringOrFallback(item.key, \'UNTITLED_KEY\')');
    expect(panel).toContain('upd: stringOrFallback(item.upd, \'-\')');
    expect(panel).toContain('rot: stringOrFallback(item.rot, \'-\')');
    expect(panel).toContain('{#each secrets as secretEnv, index (`${secretEnv.env}:${index}`)}');
    expect(panel).toContain('<SecretEnvRow {secretEnv} />');
    expect(row).toContain('<span class="env-name" title={secretEnv.env}>{secretEnv.env}</span>');
    expect(row).toContain('<span class="env-count">{secretEnv.count} keys</span>');
  });
});
