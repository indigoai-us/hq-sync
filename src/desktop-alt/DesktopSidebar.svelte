<script lang="ts">
  import type { Workspace } from '../lib/workspaces';

  type DesktopRoute = { kind: 'sync' | 'meetings' | 'company'; slug?: string };

  interface Props {
    route: DesktopRoute;
    companies: Workspace[];
    onnavigate: (route: DesktopRoute) => void;
  }

  let { route, companies, onnavigate }: Props = $props();

  function isCompanyActive(slug: string) {
    return route.kind === 'company' && route.slug === slug;
  }
</script>

<aside class="desktop-sidebar" aria-label="Desktop navigation">
  <div class="sidebar-title">HQ</div>

  <nav class="sidebar-nav" aria-label="Primary">
    <button
      type="button"
      class:active={route.kind === 'sync'}
      aria-current={route.kind === 'sync' ? 'page' : undefined}
      onclick={() => onnavigate({ kind: 'sync' })}
    >
      <span>Sync</span>
      <kbd>⌘1</kbd>
    </button>
    <button
      type="button"
      class:active={route.kind === 'meetings'}
      aria-current={route.kind === 'meetings' ? 'page' : undefined}
      onclick={() => onnavigate({ kind: 'meetings' })}
    >
      <span>Meetings</span>
      <kbd>⌘2</kbd>
    </button>
  </nav>

  <div class="company-section">
    <div class="section-label">Companies</div>
    <nav class="sidebar-nav company-list" aria-label="Companies">
      {#each companies as company, index (company.slug)}
        <button
          type="button"
          class:active={isCompanyActive(company.slug)}
          aria-current={isCompanyActive(company.slug) ? 'page' : undefined}
          onclick={() => onnavigate({ kind: 'company', slug: company.slug })}
        >
          <span>{company.displayName}</span>
          {#if index < 4}
            <kbd>⌘{index + 3}</kbd>
          {/if}
        </button>
      {:else}
        <div class="empty-row">No companies</div>
      {/each}
    </nav>
  </div>
</aside>
