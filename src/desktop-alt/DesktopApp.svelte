<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import type { Workspace, WorkspacesResult } from '../lib/workspaces';
  import DesktopSidebar from './DesktopSidebar.svelte';
  import DesktopStatusBar from './DesktopStatusBar.svelte';
  import './styles/desktop-alt.css';

  type DesktopRoute = { kind: 'sync' | 'meetings' | 'company'; slug?: string };

  let route = $state<DesktopRoute>({ kind: 'sync' });
  let workspaces = $state<Workspace[]>([]);
  let workspaceError = $state<string | null>(null);

  const companies = $derived(workspaces.filter((workspace) => workspace.kind === 'company'));
  const routeKey = $derived(route.kind === 'company' ? `company:${route.slug ?? ''}` : route.kind);
  const activeCompany = $derived(
    route.kind === 'company'
      ? companies.find((company) => company.slug === route.slug) ?? null
      : null
  );
  const pageTitle = $derived.by(() => {
    if (route.kind === 'sync') return 'Sync';
    if (route.kind === 'meetings') return 'Meetings';
    return activeCompany?.displayName ?? 'Company';
  });
  const pagePlaceholder = $derived.by(() => {
    if (route.kind === 'sync') return 'Sync page - wired in US-005';
    if (route.kind === 'meetings') return 'Meetings page - wired in US-005';
    return 'Company page - wired in US-005';
  });

  function navigate(nextRoute: DesktopRoute) {
    route = nextRoute;
  }

  async function loadWorkspaces() {
    try {
      const result = await invoke<WorkspacesResult>('list_syncable_workspaces');
      workspaces = result.workspaces;
      workspaceError = result.error ?? result.manifestError;
    } catch (err) {
      console.error('list_syncable_workspaces failed:', err);
      workspaceError = String(err);
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!(event.metaKey || event.ctrlKey)) return;

    if (event.key === '1') {
      event.preventDefault();
      navigate({ kind: 'sync' });
      return;
    }

    if (event.key === '2') {
      event.preventDefault();
      navigate({ kind: 'meetings' });
      return;
    }

    if (['3', '4', '5', '6'].includes(event.key)) {
      const company = companies[Number.parseInt(event.key, 10) - 3];
      if (company) {
        event.preventDefault();
        navigate({ kind: 'company', slug: company.slug });
      }
    }
  }

  onMount(() => {
    loadWorkspaces();
    window.addEventListener('keydown', handleKeydown);

    return () => {
      window.removeEventListener('keydown', handleKeydown);
    };
  });
</script>

<div class="desktop-shell">
  <DesktopSidebar {route} {companies} onnavigate={navigate} />

  <div class="desktop-content">
    <main class="desktop-main" aria-label="Desktop content">
      <div class="desktop-main-scroll">
        {#key routeKey}
          <section class="page" aria-labelledby="desktop-page-title">
            <div class="page-header">
              <h1 id="desktop-page-title">{pageTitle}</h1>
            </div>
            <div class="placeholder-panel">
              <p>{pagePlaceholder}</p>
              {#if route.kind === 'company' && activeCompany}
                <span>{activeCompany.slug}</span>
              {/if}
              {#if workspaceError}
                <span class="workspace-error">{workspaceError}</span>
              {/if}
            </div>
          </section>
        {/key}
      </div>

      <DesktopStatusBar version={__APP_VERSION__} />
    </main>
  </div>
</div>
