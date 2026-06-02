<script lang="ts">
  import type { Workspace } from '../lib/workspaces';
  import { getDesktopSidebarRows, isDesktopRouteActive, type DesktopRoute } from './route';

  interface Props {
    route: DesktopRoute;
    companies: Workspace[];
    onnavigate: (route: DesktopRoute) => void;
  }

  let { route, companies, onnavigate }: Props = $props();

  const rows = $derived(getDesktopSidebarRows(route, companies));
  // Sync / Meetings are the two top-level destinations; everything after them
  // is a company row.
  const primaryRows = $derived(rows.slice(0, 2));
  const companyRows = $derived(rows.slice(2));
</script>

<aside class="desktop-sidebar" aria-label="Desktop navigation">
  <div class="sidebar-title">HQ</div>

  <nav class="sidebar-nav" aria-label="Primary">
    {#each primaryRows as row (row.label)}
      <button
        type="button"
        class:active={row.active}
        aria-current={row.active ? 'page' : undefined}
        onclick={() => onnavigate(row.route)}
      >
        <span>{row.label}</span>
        <kbd>{row.shortcut}</kbd>
      </button>
    {/each}
  </nav>

  <div class="company-section">
    <div class="section-label">Companies</div>
    <nav class="sidebar-nav company-list" aria-label="Companies">
      {#each companyRows as row (row.route.slug)}
        <button
          type="button"
          class:active={isDesktopRouteActive(route, row.route)}
          aria-current={isDesktopRouteActive(route, row.route) ? 'page' : undefined}
          onclick={() => onnavigate(row.route)}
        >
          <span>{row.label}</span>
          {#if row.shortcut}
            <kbd>{row.shortcut}</kbd>
          {/if}
        </button>
      {:else}
        <div class="empty-row">No companies</div>
      {/each}
    </nav>
  </div>
</aside>
