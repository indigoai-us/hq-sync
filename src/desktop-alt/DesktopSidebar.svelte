<script lang="ts">
  import type { Workspace } from '../lib/workspaces';
  import { getDesktopSidebarRows, isDesktopRouteActive, type DesktopRoute } from './route';

  interface Props {
    route: DesktopRoute;
    companies: Workspace[];
    onnavigate: (route: DesktopRoute) => void;
    onsearch?: () => void;
    onsettings?: () => void;
    /** Admin-only Moderation row gate (default-deny). */
    isAdmin?: boolean;
  }

  let { route, companies, onnavigate, onsearch, onsettings, isAdmin = false }: Props = $props();

  const rows = $derived(getDesktopSidebarRows(route, companies, { isAdmin }));
  // Top-level destinations (Sync / Meetings / Library / admin-only Moderation)
  // vs. per-company rows. Split on route kind so an optional primary row (e.g.
  // Moderation) can't shift a fixed-index slice and leak a company into the
  // primary nav.
  const primaryRows = $derived(rows.filter((row) => row.route.kind !== 'company'));
  const companyRows = $derived(rows.filter((row) => row.route.kind === 'company'));
</script>

<aside class="desktop-sidebar" aria-label="Desktop navigation">
  <button class="sidebar-search" type="button" onclick={() => onsearch?.()} aria-label="Search HQ">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <circle cx="11" cy="11" r="7" />
      <path d="m21 21-4.3-4.3" />
    </svg>
    <span class="sidebar-search-label">Search HQ…</span>
    <kbd>⌘K</kbd>
  </button>

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

  <button class="sidebar-account" type="button" onclick={() => onsettings?.()} aria-label="Open settings">
    <span class="account-avatar" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" width="14" height="14">
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
      </svg>
    </span>
    <span class="account-meta">
      <span class="account-name">Settings</span>
      <span class="account-status">preferences · sync</span>
    </span>
  </button>
</aside>
