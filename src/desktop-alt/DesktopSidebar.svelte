<script lang="ts">
  import type { Workspace } from '../lib/workspaces';
  import { getDesktopSidebarRows, isDesktopRouteActive, type DesktopRoute } from './route';

  interface Props {
    route: DesktopRoute;
    companies: Workspace[];
    onnavigate: (route: DesktopRoute) => void;
    /** Admin-only Moderation row gate (default-deny). */
    isAdmin?: boolean;
  }

  let { route, companies, onnavigate, isAdmin = false }: Props = $props();

  const rows = $derived(getDesktopSidebarRows(route, companies, { isAdmin }));
  // Top-level destinations (Sync / Meetings / Library / admin-only Moderation)
  // vs. per-company rows. Split on route kind so an optional primary row (e.g.
  // Moderation) can't shift a fixed-index slice and leak a company into the
  // primary nav.
  const primaryRows = $derived(rows.filter((row) => row.route.kind !== 'company'));
  const companyRows = $derived(rows.filter((row) => row.route.kind === 'company'));
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
        {#if row.shortcut}
          <kbd>{row.shortcut}</kbd>
        {/if}
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
