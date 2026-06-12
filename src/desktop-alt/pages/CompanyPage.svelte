<script lang="ts">
  import { open as openExternal } from '@tauri-apps/plugin-shell';
  import type { Workspace } from '../../lib/workspaces';
  import ActivityPanel from '../panels/ActivityPanel.svelte';
  import CompanyBoardPanel from '../panels/CompanyBoardPanel.svelte';
  import CompanyGoalsPage from './CompanyGoalsPage.svelte';
  import CompanyProjectsPage from './CompanyProjectsPage.svelte';
  import CompanyTasksPage from './CompanyTasksPage.svelte';
  import DeploymentsPanel from '../panels/DeploymentsPanel.svelte';
  import SecretsPanel from '../panels/SecretsPanel.svelte';
  import CompanyLibraryPanel from '../panels/CompanyLibraryPanel.svelte';
  import { useCompanySummary } from '../lib/company-summary.svelte';
  import { DEFAULT_COMPANY_TAB, type CompanyTab } from '../route';

  interface Props {
    company: Workspace;
    /**
     * Which of the eight company sections to show — driven by the V4 secondary
     * sidebar (US-002); the in-page segmented control is gone. Defaults to
     * Overview.
     */
    tab?: CompanyTab;
  }

  let { company, tab = DEFAULT_COMPANY_TAB }: Props = $props();

  const summaryState = useCompanySummary({ slug: () => company.slug });

  const subtitle = $derived(
    `${summaryState.summary.board} board cards · ${summaryState.summary.activity.last7d} activity this week · ${summaryState.summary.deployments} deployments · ${summaryState.summary.secrets} secrets`,
  );

  // HQ web console base. Same host the Meetings page links to for
  // "Open HQ Console Integrations" — the company console lives at /{slug}.
  const HQ_CONSOLE_BASE = 'https://hq.getindigo.ai';

  function companyConsoleUrl(): string {
    return `${HQ_CONSOLE_BASE}/${encodeURIComponent(company.slug)}`;
  }

  function openInBrowser() {
    void openExternal(companyConsoleUrl());
  }

  function openInvite() {
    void openExternal(`${companyConsoleUrl()}/invite`);
  }
</script>

<section class="company-page" aria-labelledby="company-page-title">
  {#if tab === 'goals'}
    <h1 id="company-page-title" class="visually-hidden">{company.displayName}</h1>
  {:else}
    <header class="company-header">
      <div class="company-heading">
        <nav class="company-crumb" aria-label="Breadcrumb">
          <span>Companies</span>
          <span aria-hidden="true">›</span>
          <span>{company.displayName}</span>
        </nav>
        <h1 id="company-page-title">{company.displayName}</h1>
        <p>{subtitle}</p>
        {#if summaryState.error}
          <span class="summary-error">Summary unavailable. Showing zeros.</span>
        {/if}
      </div>

      <div class="company-actions" aria-label="Company actions">
        <button type="button" onclick={openInBrowser}>Open in browser</button>
        <button type="button" onclick={openInvite}>Invite</button>
      </div>
    </header>
  {/if}

  {#key `${company.slug}:${tab}`}
    <div class="company-panel">
      {#if tab === 'overview'}
        <CompanyBoardPanel slug={company.slug} />
      {:else if tab === 'goals'}
        <CompanyGoalsPage slug={company.slug} />
      {:else if tab === 'projects'}
        <CompanyProjectsPage slug={company.slug} />
      {:else if tab === 'tasks'}
        <CompanyTasksPage slug={company.slug} />
      {:else if tab === 'activity'}
        <ActivityPanel slug={company.slug} />
      {:else if tab === 'deployments'}
        <DeploymentsPanel slug={company.slug} />
      {:else if tab === 'library'}
        <CompanyLibraryPanel slug={company.slug} />
      {:else if tab === 'secrets'}
        <SecretsPanel slug={company.slug} />
      {:else}
        <div class="section-pending">
          <p>This section is on its way.</p>
        </div>
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

  .company-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 20px;
    min-width: 0;
  }

  .company-heading {
    min-width: 0;
  }

  .company-crumb {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    margin-bottom: 7px;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    white-space: nowrap;
  }

  .company-crumb span {
    min-width: 0;
  }

  .company-crumb span:last-child {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .company-header h1 {
    margin: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 29px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .company-header p {
    margin: 5px 0 0;
    max-width: 100%;
    overflow-wrap: anywhere;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .summary-error {
    display: block;
    margin-top: 5px;
    color: var(--amber);
    font-size: var(--text-base);
    line-height: 16px;
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
    font-weight: 600;
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

  .company-actions button:active {
    transform: translateY(0);
    opacity: 0.72;
  }

  .section-pending p {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
  }

  .company-panel {
    min-width: 0;
    animation: panel-enter 220ms cubic-bezier(0.33, 1, 0.68, 1);
    will-change: opacity, transform;
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
    .company-header {
      flex-direction: column;
    }

    .company-header h1 {
      white-space: normal;
    }

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
