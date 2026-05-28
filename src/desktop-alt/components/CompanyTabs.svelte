<script lang="ts">
  import type { CompanySummary } from '../lib/company-summary.svelte';

  export type CompanyTab = 'board' | 'activity' | 'deployments' | 'secrets';

  interface Props {
    activeTab: CompanyTab;
    summary: CompanySummary;
    role: string | null;
    onselect: (tab: CompanyTab) => void;
  }

  let { activeTab, summary, role, onselect }: Props = $props();

  const tabs = $derived([
    { id: 'board' as const, label: 'Board', count: summary.board },
    { id: 'activity' as const, label: 'Activity', count: summary.activity.last7d },
    { id: 'deployments' as const, label: 'Deployments', count: summary.deployments },
    { id: 'secrets' as const, label: 'Secrets', count: summary.secrets },
  ]);

  const roleLabel = $derived(role ? role : 'No role');
</script>

<div class="company-tabs-row">
  <div class="company-tabs" role="tablist" aria-label="Company sections">
    {#each tabs as tab (tab.id)}
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === tab.id}
        class:active={activeTab === tab.id}
        onclick={() => onselect(tab.id)}
      >
        <span>{tab.label}</span>
        <span class="tab-count" aria-label={`${tab.count} ${tab.label.toLowerCase()}`}>
          {tab.count}
        </span>
      </button>
    {/each}
  </div>

  <span class="role-pill" title="Workspace role">{roleLabel}</span>
</div>

<style>
  .company-tabs-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-width: 0;
    border-bottom: 1px solid #e4e4e7;
  }

  .company-tabs {
    display: flex;
    min-width: 0;
    overflow-x: auto;
  }

  .company-tabs button {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 38px;
    padding: 0 2px;
    margin-right: 22px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: #71717a;
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    line-height: 38px;
    white-space: nowrap;
    cursor: default;
  }

  .company-tabs button:hover {
    color: #27272a;
  }

  .company-tabs button.active {
    border-bottom-color: #27272a;
    color: #18181b;
  }

  .tab-count {
    min-width: 20px;
    height: 18px;
    padding: 0 6px;
    border-radius: 999px;
    background: #e4e4e7;
    color: #52525b;
    font-size: 11px;
    font-weight: 650;
    line-height: 18px;
    text-align: center;
  }

  .company-tabs button.active .tab-count {
    background: #27272a;
    color: #fafafa;
  }

  .role-pill {
    flex: 0 0 auto;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: 1px solid #d4d4d8;
    border-radius: 999px;
    padding: 3px 9px;
    background: #ffffff;
    color: #52525b;
    font-size: 11px;
    font-weight: 650;
    line-height: 16px;
    text-transform: capitalize;
  }

  @media (max-width: 820px) {
    .company-tabs-row {
      align-items: stretch;
      flex-direction: column-reverse;
      gap: 8px;
    }

    .role-pill {
      align-self: flex-start;
    }
  }
</style>
