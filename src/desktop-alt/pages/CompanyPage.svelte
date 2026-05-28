<script lang="ts">
  import type { Workspace } from '../../lib/workspaces';
  import CompanyTabPlaceholder from '../components/CompanyTabPlaceholder.svelte';
  import CompanyTabs, { type CompanyTab } from '../components/CompanyTabs.svelte';
  import { useCompanySummary } from '../lib/company-summary.svelte';

  interface Props {
    company: Workspace;
  }

  let { company }: Props = $props();

  let activeTab = $state<CompanyTab>('board');
  let previousSlug = $state<string | null>(null);
  const summaryState = useCompanySummary({ slug: () => company.slug });

  $effect(() => {
    if (company.slug !== previousSlug) {
      previousSlug = company.slug;
      activeTab = 'board';
    }
  });

  const subtitle = $derived(
    `${summaryState.summary.board} board cards · ${summaryState.summary.activity.last7d} activity this week · ${summaryState.summary.deployments} deployments · ${summaryState.summary.secrets} secrets`,
  );

  function selectTab(tab: CompanyTab) {
    activeTab = tab;
  }
</script>

<section class="company-page" aria-labelledby="company-page-title">
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
      <button type="button">Open in browser</button>
      <button type="button">Invite</button>
    </div>
  </header>

  <CompanyTabs
    {activeTab}
    summary={summaryState.summary}
    role={company.role}
    onselect={selectTab}
  />

  <div class="company-panel">
    {#if activeTab === 'board'}
      <CompanyTabPlaceholder label="Board panel - wired in US-009" />
    {:else if activeTab === 'activity'}
      <CompanyTabPlaceholder label="Activity panel - wired in US-010" />
    {:else if activeTab === 'deployments'}
      <CompanyTabPlaceholder label="Deployments panel - wired in US-011" />
    {:else}
      <CompanyTabPlaceholder label="Secrets panel - wired in US-012" />
    {/if}
  </div>
</section>

<style>
  .company-page {
    display: grid;
    gap: 18px;
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
    margin-bottom: 7px;
    color: #71717a;
    font-size: 12px;
    line-height: 16px;
  }

  .company-header h1 {
    margin: 0;
    overflow: hidden;
    color: #18181b;
    font-size: 22px;
    font-weight: 680;
    line-height: 29px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .company-header p {
    margin: 5px 0 0;
    color: #71717a;
    font-size: 13px;
    line-height: 18px;
  }

  .summary-error {
    display: block;
    margin-top: 5px;
    color: #a16207;
    font-size: 12px;
    line-height: 16px;
  }

  .company-actions {
    display: flex;
    flex: 0 0 auto;
    gap: 8px;
  }

  .company-actions button {
    height: 30px;
    padding: 0 11px;
    border: 1px solid #d4d4d8;
    border-radius: 6px;
    background: #ffffff;
    color: #27272a;
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    cursor: default;
  }

  .company-actions button:hover {
    border-color: #a1a1aa;
    background: #f4f4f5;
  }

  .company-panel {
    min-width: 0;
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
      flex: 1 1 0;
    }
  }
</style>
