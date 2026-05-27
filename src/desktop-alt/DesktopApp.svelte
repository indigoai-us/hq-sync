<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import type { Workspace, WorkspacesResult } from '../lib/workspaces';
  import {
    DESKTOP_SHELL_LAYOUT,
    getDesktopCompanies,
    getDesktopHotkeyRoute,
    getDesktopPage,
    getDesktopRouteKey,
    initialDesktopRoute,
    type DesktopRoute,
  } from './route';
  import DesktopSidebar from './DesktopSidebar.svelte';
  import DesktopStatusBar from './DesktopStatusBar.svelte';
  import './styles/desktop-alt.css';

  let route = $state<DesktopRoute>(initialDesktopRoute);
  let workspaces = $state<Workspace[]>([]);
  let workspaceError = $state<string | null>(null);

  const companies = $derived(getDesktopCompanies(workspaces));
  const routeKey = $derived(getDesktopRouteKey(route));
  const page = $derived(getDesktopPage(route, companies));

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
    const nextRoute = getDesktopHotkeyRoute(event, companies);
    if (!nextRoute) return;

    event.preventDefault();
    navigate(nextRoute);
  }

  onMount(() => {
    loadWorkspaces();
    window.addEventListener('keydown', handleKeydown);

    return () => {
      window.removeEventListener('keydown', handleKeydown);
    };
  });
</script>

<div
  class="desktop-shell"
  style={`--desktop-sidebar-width: ${DESKTOP_SHELL_LAYOUT.sidebarWidthPx}px; --desktop-status-bar-height: ${DESKTOP_SHELL_LAYOUT.statusBarHeightPx}px;`}
>
  <DesktopSidebar {route} {companies} onnavigate={navigate} />

  <div class="desktop-content">
    <main class="desktop-main" aria-label="Desktop content">
      <div class="desktop-main-scroll">
        {#key routeKey}
          <section class="page" aria-labelledby="desktop-page-title">
            <div class="page-header">
              <h1 id="desktop-page-title">{page.title}</h1>
            </div>
            <div class="placeholder-panel">
              <p>{page.placeholder}</p>
              {#if route.kind === 'company' && page.activeCompany}
                <span>{page.activeCompany.slug}</span>
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
