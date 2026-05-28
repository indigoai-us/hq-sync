import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const companyPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompanyPage.svelte'),
  'utf8',
);
const deploymentsPanel = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/panels/DeploymentsPanel.svelte'),
  'utf8',
);
const deploymentRow = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/DeploymentRow.svelte'),
  'utf8',
);
const desktopAltCommand = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/commands/desktop_alt.rs'),
  'utf8',
);
const tauriMain = readFileSync(resolve(process.cwd(), 'src-tauri/src/main.rs'), 'utf8');
const desktopAltCapability = readFileSync(
  resolve(process.cwd(), 'src-tauri/capabilities/desktop-alt.json'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-011: Deployments panel reads hq-deploy subdomains via Tauri command', () => {
  it('wires the deployments tab to get_company_deployments with the selected company slug', () => {
    const page = normalize(companyPage);
    const panel = normalize(deploymentsPanel);

    expect(page).toContain("import DeploymentsPanel from '../panels/DeploymentsPanel.svelte'");
    expect(page).toContain('<DeploymentsPanel slug={company.slug} />');
    expect(panel).toContain("void invoke<Partial<DeploymentEntry>[]>('get_company_deployments', { slug })");
    expect(panel).toContain('return () => { cancelled = true; };');
    expect(panel).toContain('function retry() { reloadToken += 1; }');
    expect(panel).toContain("console.error('get_company_deployments failed:', err)");
    expect(tauriMain).toContain('commands::desktop_alt::get_company_deployments');
  });

  it('returns the prototype DeploymentEntry shape from hq-deploy reuse-as-is with Cognito bearer auth', () => {
    const command = normalize(desktopAltCommand);

    expect(command).toContain('pub struct DeploymentEntry { pub sub: String, pub url: String, pub state: String, pub last_deploy: String, pub size: String, pub ver: String, pub pwd: bool, }');
    expect(command).toContain('pub async fn get_company_deployments(slug: String) -> Result<Vec<DeploymentEntry>, String>');
    expect(command).toContain('let _slug = normalize_slug(&slug)?;');
    expect(command).toContain('const HQ_DEPLOY_API_BASE: &str = "https://api.indigo-hq.com";');
    expect(command).toContain('let url = deployments_url(HQ_DEPLOY_API_BASE);');
    expect(command).toContain('format!("{}/api/apps/me", base.trim_end_matches(\'/\'))');
    expect(command).toContain('let token = cognito::get_valid_access_token() .await .map_err(|e| format!("auth: {e}"))?;');
    expect(command).toContain('.header("authorization", format!("Bearer {token}"))');
    expect(command).toContain('parse_deployments_response(status, &text)');
  });

  it('normalizes hq-deploy app rows into visible url, version, state, and password-lock fields', () => {
    const panel = normalize(deploymentsPanel);
    const row = normalize(deploymentRow);
    const command = normalize(desktopAltCommand);

    expect(command).toContain('let url = string_field(value, &["url"]) .and_then(|url| normalize_deployment_host(&url)) .unwrap_or_else(|| format!("{sub}.{HQ_DEPLOY_APP_DOMAIN}"));');
    expect(command).toContain('state: normalize_deployment_state(value)');
    expect(command).toContain('last_deploy: deployment_last_deploy(value)');
    expect(command).toContain('size: deployment_size(value)');
    expect(command).toContain('ver: deployment_version(value)');
    expect(command).toContain('pwd: bool_field( value, &["pwd", "passwordProtected", "passwordLocked", "locked"], ) .unwrap_or(false)');
    expect(panel).toContain('deployments = Array.isArray(result) ? result.map(normalizeDeployment) : [];');
    expect(panel).toContain('ver: stringOrFallback(entry.ver, \'-\')');
    expect(panel).toContain('pwd: entry.pwd === true');
    expect(row).toContain('<span class="url" title={deployment.url}>{deployment.url}</span>');
    expect(row).toContain('<span class="version" title={deployment.ver}>{deployment.ver}</span>');
    expect(row).toContain('<span class={`status-dot ${deployment.state}`} title={stateLabel} aria-label={stateLabel}></span>');
    expect(row).toContain('{#if deployment.pwd}');
    expect(row).toContain('<span class="lock-icon" title="Password locked" aria-label="Password locked"></span>');
  });

  it('renders toolbar state counts, disabled deploy affordance, and deployment rows with the prototype grid', () => {
    const panel = normalize(deploymentsPanel);
    const row = normalize(deploymentRow);

    expect(panel).toContain('const activeCount = $derived(countByState(\'active\'))');
    expect(panel).toContain('const deployingCount = $derived(countByState(\'deploying\'))');
    expect(panel).toContain('const pausedCount = $derived(countByState(\'paused\'))');
    expect(panel).toContain('<span><strong>{activeCount}</strong> active</span>');
    expect(panel).toContain('<span><strong>{deployingCount}</strong> deploying</span>');
    expect(panel).toContain('<span><strong>{pausedCount}</strong> paused</span>');
    expect(panel).toContain('title="Deploy from terminal: /deploy"');
    expect(panel).toContain('aria-label="Deploy from terminal: /deploy"');
    expect(panel).toContain('<button class="toolbar-button" type="button" disabled title="Find deployments">');
    expect(panel).toContain('{#each deployments as deployment, index (`${deployment.url}:${index}`)}');
    expect(panel).toContain('<DeploymentRow {deployment} />');
    expect(row).toContain('grid-template-columns: 14px 1.4fr 1fr auto auto auto;');
  });

  it('pulses deploying rows blue and counts deploying deployments in the toolbar', () => {
    const panel = normalize(deploymentsPanel);
    const row = normalize(deploymentRow);

    expect(panel).toContain("return deployments.filter((deployment) => deployment.state === state).length;");
    expect(panel).toContain("<span><strong>{deployingCount}</strong> deploying</span>");
    expect(row).toContain(".status-dot.deploying { background: #2563eb; animation: pulse 1.4s ease-in-out infinite; }");
    expect(row).toContain('@keyframes pulse');
    expect(row).toContain('@media (prefers-reduced-motion: reduce) { .status-dot.deploying { animation: none; } }');
  });

  it('opens deployment URLs in the external browser through tauri-plugin-shell', () => {
    const row = normalize(deploymentRow);

    expect(row).toContain("import { open } from '@tauri-apps/plugin-shell';");
    expect(row).toContain('async function openDeployment() { await open(`https://${deployment.url}`); }');
    expect(row).toContain('title="Open in browser"');
    expect(row).toContain('onclick={openDeployment}');
    expect(desktopAltCapability).toContain('"shell:allow-open"');
  });

  it('matches US-009-style loading, error, retry, and empty-state handling', () => {
    const panel = normalize(deploymentsPanel);

    expect(panel).toContain('{#if error}');
    expect(panel).toContain('<div class="deployments-error" role="alert">');
    expect(panel).toContain('<strong>Deployments unavailable</strong>');
    expect(panel).toContain('<button type="button" onclick={retry}>Retry</button>');
    expect(panel).toContain('<section class="deployments-card" aria-labelledby="deployments-list-title" aria-busy={loading}>');
    expect(panel).toContain('<div class="deployment-skeleton" aria-label="Loading deployments">');
    expect(panel).toContain('{:else if deployments.length > 0}');
    expect(panel).toContain('<div class="empty-state">No provisioned subdomains for this company.</div>');
  });
});
