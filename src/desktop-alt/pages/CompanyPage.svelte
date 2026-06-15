<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open as openExternal } from '@tauri-apps/plugin-shell';
  import type { Workspace } from '../../lib/workspaces';
  import { buildClaudeCodeUrl } from '../../lib/claude-code-link';
  import { companyInviteUrl, companySettingsUrl } from '../lib/hq-console';
  import ActivityPanel from '../panels/ActivityPanel.svelte';
  import CompanyBoardPanel from '../panels/CompanyBoardPanel.svelte';
  import CompanyGoalsPage from './CompanyGoalsPage.svelte';
  import CompanyProjectsPage from './CompanyProjectsPage.svelte';
  import CompanyTasksPage from './CompanyTasksPage.svelte';
  import DeploymentsPanel from '../panels/DeploymentsPanel.svelte';
  import SecretsPanel from '../panels/SecretsPanel.svelte';
  import CompanyLibraryPanel from '../panels/CompanyLibraryPanel.svelte';
  import { DEFAULT_COMPANY_TAB, type CompanyTab } from '../route';

  interface Props {
    company: Workspace;
    /**
     * Which of the eight company sections to show — driven by the V4 secondary
     * sidebar (US-002); the in-page segmented control is gone. Defaults to
     * Overview.
     */
    tab?: CompanyTab;
    /** Switch to the Projects section before handing project creation to HQ. */
    onopenprojects?: () => void;
  }

  let {
    company,
    tab = DEFAULT_COMPANY_TAB,
    onopenprojects,
  }: Props = $props();

  interface SettingsWire {
    hqPath?: string | null;
  }

  let actionError = $state<string | null>(null);
  let newProjectBusy = $state(false);

  const cloudBacked = $derived(
    company.state === 'synced' ||
      company.state === 'cloud-only' ||
      (company.kind === 'company' && Boolean(company.cloudUid)),
  );

  function openInvite() {
    void openExternal(companyInviteUrl(company.slug));
  }

  // Company settings (sync rules, members, roles) live in the HQ web console,
  // not the in-app Settings route — open the company's console page in the
  // system browser.
  function openCompanySettings() {
    actionError = null;
    void openExternal(companySettingsUrl(company.slug));
  }

  async function startNewProject() {
    if (newProjectBusy) return;
    actionError = null;
    newProjectBusy = true;
    onopenprojects?.();

    const prompt = [
      `/plan ${company.slug} new project`,
      '',
      `Start a new HQ project for ${company.displayName}.`,
      `Use company slug: ${company.slug}.`,
      'Interview me only for the missing product decisions, then create the project PRD under this company and make sure it appears in the HQ Sync desktop Projects screen.',
    ].join('\n');

    try {
      const settings = await invoke<SettingsWire>('get_settings').catch(() => ({ hqPath: null }));
      const folder = settings.hqPath ?? company.localPath ?? '';
      const url = buildClaudeCodeUrl({ folder, prompt });
      await invoke('open_claude_code_link', { url });
    } catch (err) {
      console.error('open_claude_code_link for new project failed:', err);
      try {
        await navigator.clipboard.writeText(prompt);
        actionError = 'Project prompt copied. Paste it into Claude Code to continue.';
      } catch {
        actionError = 'Could not open Claude Code. The Projects screen is open.';
      }
    } finally {
      newProjectBusy = false;
    }
  }
</script>

<section class="company-page" aria-labelledby="company-page-title">
  <h1 id="company-page-title" class="visually-hidden">{company.displayName}</h1>

  <header class="company-actions-row">
    <div></div>
    <div class="company-actions" aria-label="Company actions">
      <button type="button" onclick={openInvite}>Invite</button>
      <button type="button" onclick={openCompanySettings}>Settings</button>
      <button type="button" class="primary" onclick={() => void startNewProject()} disabled={newProjectBusy}>
        {newProjectBusy ? 'Opening…' : 'New project'}
      </button>
    </div>
  </header>

  {#if actionError}
    <p class="company-action-error" role="status">{actionError}</p>
  {/if}

  {#key `${company.slug}:${tab}`}
    <div class="company-panel">
      {#if tab === 'overview'}
        <CompanyBoardPanel slug={company.slug} {cloudBacked} />
      {:else if tab === 'goals'}
        <CompanyGoalsPage slug={company.slug} />
      {:else if tab === 'projects'}
        <CompanyProjectsPage slug={company.slug} onnewproject={startNewProject} />
      {:else if tab === 'tasks'}
        <CompanyTasksPage slug={company.slug} />
      {:else if tab === 'activity'}
        <ActivityPanel slug={company.slug} {cloudBacked} />
      {:else if tab === 'deployments'}
        <DeploymentsPanel slug={company.slug} {cloudBacked} />
      {:else if tab === 'library'}
        <CompanyLibraryPanel slug={company.slug} />
      {:else if tab === 'secrets'}
        <SecretsPanel slug={company.slug} {cloudBacked} />
      {/if}
    </div>
  {/key}
</section>

<style>
  .company-page {
    display: grid;
    gap: 18px;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .company-actions-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 30px;
    min-width: 0;
  }

  .company-actions {
    display: flex;
    flex: 0 0 auto;
    gap: 8px;
  }

  .company-actions button {
    max-width: 160px;
    height: 30px;
    overflow: hidden;
    padding: 0 11px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 400;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: default;
    transition: opacity 120ms cubic-bezier(0.33, 1, 0.68, 1),
      transform 120ms cubic-bezier(0.33, 1, 0.68, 1);
  }

  .company-actions button:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    transform: translateY(-1px);
  }

  .company-actions button.primary {
    border-color: transparent;
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  .company-actions button:active {
    transform: translateY(0);
    opacity: 0.72;
  }

  .company-actions button:disabled {
    transform: none;
    opacity: 0.58;
  }

  .company-action-error {
    margin: -10px 0 0;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    line-height: 1.3;
  }

  .company-panel {
    min-width: 0;
    animation: panel-enter 220ms cubic-bezier(0.33, 1, 0.68, 1);
  }

  @keyframes panel-enter {
    from {
      opacity: 0;
      transform: translateY(6px);
    }

    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @media (max-width: 720px) {
    .company-actions {
      width: 100%;
    }

    .company-actions button {
      min-width: 0;
      max-width: none;
      flex: 1 1 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .company-actions button {
      transition: none;
    }

    .company-actions button:hover,
    .company-actions button:active {
      transform: none;
    }

    .company-panel {
      animation: none;
      will-change: auto;
    }
  }
</style>
