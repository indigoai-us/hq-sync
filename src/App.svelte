<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import SignInPrompt from './components/SignInPrompt.svelte';
  import Popover from './components/Popover.svelte';
  import { conflictStore, type ConflictFile } from './stores/conflicts';
  import './styles/popover.css';

  interface Config {
    configured: boolean;
    companySlug: string;
    hqFolderPath: string;
    error?: string;
  }

  let authenticated = $state(false);
  let expiresAt = $state('');
  let checking = $state(true);
  let syncState = $state<'idle' | 'syncing' | 'error' | 'conflict'>('idle');
  let config = $state<Config | null>(null);
  let syncProgress = $state<{ filesComplete: number; filesTotal: number } | null>(null);
  let showConflictModal = $state(false);
  let conflicts = $state<ConflictFile[]>([]);

  // Collected unlisten handles for cleanup
  let unlisteners: UnlistenFn[] = [];

  async function loadConfig() {
    try {
      config = await invoke<Config>('get_config');
    } catch (err) {
      console.error('Failed to load config:', err);
    }
  }

  async function handleSyncNow() {
    if (syncState === 'syncing') return;
    syncState = 'syncing';
    syncProgress = null;
    await invoke('set_tray_state', { state: 'syncing' });
    try {
      await invoke('start_sync');
    } catch (err) {
      console.error('start_sync failed:', err);
      syncState = 'error';
      await invoke('set_tray_state', { state: 'error' });
    }
  }

  function handleSettings() {
    console.log('Settings requested (not yet implemented — US-012)');
  }

  function handleSignOut() {
    // Placeholder: clear auth state, return to sign-in
    authenticated = false;
    expiresAt = '';
    console.log('Sign out requested — clearing local auth state');
  }

  async function handleResolveConflict(path: string, strategy: 'keep-local' | 'keep-remote') {
    await conflictStore.resolveConflict(path, strategy);
    conflicts = conflictStore.conflicts;
    if (conflictStore.allResolved) {
      syncState = 'idle';
      await invoke('set_tray_state', { state: 'idle' });
    }
  }

  async function handleOpenInEditor(path: string) {
    await conflictStore.openInEditor(path);
  }

  function handleDismissConflicts() {
    showConflictModal = false;
  }

  async function setupTrayListeners() {
    // Tray menu events
    unlisteners.push(
      await listen('tray:sync-now', () => {
        handleSyncNow();
      })
    );

    unlisteners.push(
      await listen('tray:open-settings', () => {
        handleSettings();
      })
    );

    // Sync state events → update local state + tray icon
    unlisteners.push(
      await listen<{ phase: string; filesComplete: number; filesTotal: number }>(
        'sync:progress',
        async (event) => {
          syncState = 'syncing';
          syncProgress = {
            filesComplete: event.payload.filesComplete,
            filesTotal: event.payload.filesTotal,
          };
          await invoke('set_tray_state', { state: 'syncing' });
        }
      )
    );

    unlisteners.push(
      await listen('sync:complete', async () => {
        syncState = 'idle';
        syncProgress = null;
        conflictStore.clear();
        conflicts = [];
        showConflictModal = false;
        await invoke('set_tray_state', { state: 'idle' });
      })
    );

    unlisteners.push(
      await listen('sync:error', async () => {
        syncState = 'error';
        syncProgress = null;
        await invoke('set_tray_state', { state: 'error' });
      })
    );

    unlisteners.push(
      await listen<{ path: string; localHash: string; remoteHash: string; canAutoResolve: boolean }>(
        'sync:conflict',
        async (event) => {
          syncState = 'conflict';
          syncProgress = null;
          conflictStore.addConflict(event.payload);
          conflicts = conflictStore.conflicts;
          showConflictModal = true;
          await invoke('set_tray_state', { state: 'conflict' });
        }
      )
    );
  }

  $effect(() => {
    // Performance: mark app init
    performance.mark('app-init');

    checkAuth();
    loadConfig();
    setupTrayListeners();

    return () => {
      unlisteners.forEach((unlisten) => unlisten());
      unlisteners = [];
    };
  });

  async function checkAuth() {
    try {
      const state = await invoke<{
        authenticated: boolean;
        expiresAt: string | null;
      }>('get_auth_state');

      authenticated = state.authenticated;
      expiresAt = state.expiresAt ?? '';
    } catch {
      authenticated = false;
    } finally {
      checking = false;
    }
  }

  function handleAuthSuccess(auth: { authenticated: boolean; expiresAt: string }) {
    authenticated = auth.authenticated;
    expiresAt = auth.expiresAt;
  }
</script>

<main>
  {#if checking}
    <div class="loading">
      <span class="dot-spinner"></span>
    </div>
  {:else if authenticated}
    <Popover
      {syncState}
      {config}
      progress={syncProgress}
      {conflicts}
      {showConflictModal}
      onsync={handleSyncNow}
      onsettings={handleSettings}
      onsignout={handleSignOut}
      onresolve={handleResolveConflict}
      onopen={handleOpenInEditor}
      ondismissconflicts={handleDismissConflicts}
    />
  {:else}
    <SignInPrompt onsuccess={handleAuthSuccess} />
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto,
      Oxygen, Ubuntu, Cantarell, sans-serif;
    background-color: var(--popover-bg, #1a1a2e);
    color: var(--popover-text, #e0e0e0);
  }

  main {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    padding: 0;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
  }

  .dot-spinner {
    display: inline-block;
    width: 20px;
    height: 20px;
    border: 2.5px solid rgba(99, 102, 241, 0.2);
    border-top-color: #6366f1;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
