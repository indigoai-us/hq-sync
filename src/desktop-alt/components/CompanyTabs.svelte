<script lang="ts">
  import type { CompanySummary } from '../lib/company-summary.svelte';

  // Board lives per-company again (US-011): the Board tab is first/default and
  // hosts the company-scoped CompanyBoardPanel (Goals + In flight + Projects).
  export type CompanyTab = 'board' | 'activity' | 'deployments' | 'secrets' | 'library';

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
    { id: 'library' as const, label: 'Library', count: undefined as number | undefined },
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
        <span class="tab-label">{tab.label}</span>
        {#if tab.count !== undefined}
          <span class="tab-count" aria-label={`${tab.count} ${tab.label.toLowerCase()}`}>
            {tab.count}
          </span>
        {/if}
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
    border-bottom: 1px solid var(--border);
  }

  .company-tabs {
    display: flex;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  .company-tabs::-webkit-scrollbar {
    display: none;
  }

  .company-tabs button {
    position: relative;
    display: inline-flex;
    align-items: center;
    flex: 0 0 auto;
    gap: 7px;
    max-width: 170px;
    height: 38px;
    min-width: 0;
    padding: 0 2px;
    margin-right: 22px;
    border: 0;
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 38px;
    white-space: nowrap;
    cursor: default;
    transition: opacity 120ms cubic-bezier(0.33, 1, 0.68, 1),
      transform 120ms cubic-bezier(0.33, 1, 0.68, 1);
  }

  .company-tabs button::after {
    position: absolute;
    right: 0;
    bottom: -1px;
    left: 0;
    height: 2px;
    border-radius: 999px;
    background: var(--fg);
    content: '';
    opacity: 0;
    transform: scaleX(0.3);
    transform-origin: center;
    transition: opacity 140ms cubic-bezier(0.33, 1, 0.68, 1),
      transform 140ms cubic-bezier(0.33, 1, 0.68, 1);
  }

  .company-tabs button:hover {
    color: var(--fg);
    transform: translateY(-1px);
  }

  .company-tabs button:active {
    opacity: 0.72;
    transform: translateY(0);
  }

  .company-tabs button.active {
    color: var(--fg);
  }

  .company-tabs button.active::after {
    opacity: 1;
    transform: scaleX(1);
  }

  .tab-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tab-count {
    flex: 0 0 auto;
    min-width: 20px;
    max-width: 46px;
    height: 18px;
    overflow: hidden;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 18px;
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .company-tabs button.active .tab-count {
    background: var(--row-active);
    color: var(--fg);
  }

  .role-pill {
    flex: 0 0 auto;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 3px 9px;
    background: transparent;
    color: var(--muted-2);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 16px;
    text-transform: capitalize;
    transition: opacity 120ms cubic-bezier(0.33, 1, 0.68, 1),
      transform 120ms cubic-bezier(0.33, 1, 0.68, 1);
  }

  .role-pill:hover {
    transform: translateY(-1px);
  }

  @media (max-width: 820px) {
    .company-tabs-row {
      align-items: stretch;
      flex-direction: column-reverse;
      gap: 8px;
    }

    .role-pill {
      align-self: flex-start;
      max-width: 100%;
    }
  }

  @media (max-width: 520px) {
    .company-tabs button {
      max-width: 136px;
      margin-right: 16px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .company-tabs button,
    .company-tabs button::after,
    .role-pill {
      transition: none;
    }

    .company-tabs button:hover,
    .company-tabs button:active,
    .role-pill:hover {
      transform: none;
    }
  }
</style>
