<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import SignInPrompt from './components/SignInPrompt.svelte';
  import Popover from './components/Popover.svelte';
  import Settings from './components/Settings.svelte';
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
  let syncState = $state<'idle' | 'syncing' | 'error' | 'conflict' | 'setup-needed' | 'auth-error'>('idle');
  let config = $state<Config | null>(null);
  // Phase 7 runner protocol — progress is per-file with a path + bytes.
  // We also track the company currently syncing (last progress event) and
  // the count of companies in this fanout for "Syncing N of M" framing.
  let syncProgress = $state<{
    company: string;
    path: string;
    bytes: number;
  } | null>(null);
  let syncFanoutTotal = $state(0); // How many companies we're syncing
  let syncFanoutDoneCount = $state(0); // How many have hit sync:complete
  let syncLastSummary = $state<{
    companiesAttempted: number;
    filesDownloaded: number;
    bytesDownloaded: number;
  } | null>(null);
  let syncErrorMessage = $state(''); // Last auth-error or error message
  let showConflictModal = $state(false);
  let conflicts = $state<ConflictFile[]>([]);
  let showSettings = $state(false);
  let syncStatsRefresh = $state<(() => void) | null>(null);

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
    syncFanoutTotal = 0;
    syncFanoutDoneCount = 0;
    syncLastSummary = null;
    syncErrorMessage = '';
    await invoke('set_tray_state', { state: 'syncing' });
    try {
      await invoke('start_sync');
    } catch (err) {
      console.error('start_sync failed:', err);
      syncState = 'error';
      syncErrorMessage = String(err);
      await invoke('set_tray_state', { state: 'error' });
    }
  }

  function handleSettings() {
    showSettings = true;
  }

  function handleBackFromSettings() {
    showSettings = false;
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

    // --- Phase 7 runner event listeners ---
    // Protocol (see src-tauri/src/events.rs):
    //   sync:setup-needed  -- signed in, no person entity yet
    //   sync:auth-error    -- token invalid and can't refresh
    //   sync:fanout-plan   -- list of companies about to sync
    //   sync:progress      -- per-file download in-flight
    //   sync:error         -- per-file or per-company error
    //   sync:complete      -- per-company summary (fires N times in a fanout)
    //   sync:all-complete  -- aggregate summary; this is the real "done"

    unlisteners.push(
      await listen('sync:setup-needed', async () => {
        syncState = 'setup-needed';
        syncProgress = null;
        await invoke('set_tray_state', { state: 'error' });
      })
    );

    unlisteners.push(
      await listen<{ message: string }>('sync:auth-error', async (event) => {
        syncState = 'auth-error';
        syncProgress = null;
        syncErrorMessage = event.payload.message;
        await invoke('set_tray_state', { state: 'error' });
      })
    );

    unlisteners.push(
      await listen<{ companies: Array<{ uid: string; slug: string }> }>(
        'sync:fanout-plan',
        async (event) => {
          syncState = 'syncing';
          syncFanoutTotal = event.payload.companies.length;
          syncFanoutDoneCount = 0;
          await invoke('set_tray_state', { state: 'syncing' });
        }
      )
    );

    unlisteners.push(
      await listen<{ company: string; path: string; bytes: number; message?: string }>(
        'sync:progress',
        async (event) => {
          syncState = 'syncing';
          syncProgress = {
            company: event.payload.company,
            path: event.payload.path,
            bytes: event.payload.bytes,
          };
          await invoke('set_tray_state', { state: 'syncing' });
        }
      )
    );

    unlisteners.push(
      await listen<{
        company: string;
        filesDownloaded: number;
        bytesDownloaded: number;
        filesSkipped: number;
        conflicts: number;
        aborted: boolean;
      }>('sync:complete', async (event) => {
        // Per-company event — just tick the counter. Don't go idle yet;
        // wait for sync:all-complete to know the whole fanout is done.
        syncFanoutDoneCount += 1;
        if (event.payload.aborted) {
          // Conflict-aborted: show the conflict state so the user knows
          // something needs attention. ConflictModal wiring is follow-up
          // work (runner doesn't emit per-file conflict events anymore);
          // for now the tray + banner is enough signal.
          syncState = 'conflict';
          await invoke('set_tray_state', { state: 'conflict' });
        }
      })
    );

    unlisteners.push(
      await listen<{
        companiesAttempted: number;
        filesDownloaded: number;
        bytesDownloaded: number;
        errors: Array<{ company: string; message: string }>;
      }>('sync:all-complete', async (event) => {
        syncLastSummary = {
          companiesAttempted: event.payload.companiesAttempted,
          filesDownloaded: event.payload.filesDownloaded,
          bytesDownloaded: event.payload.bytesDownloaded,
        };
        syncProgress = null;
        // Only flip to idle if nothing raised conflict/error mid-stream
        if (syncState !== 'conflict' && syncState !== 'error') {
          syncState = 'idle';
          await invoke('set_tray_state', { state: 'idle' });
        }
        // Refresh SyncStats so "last synced" updates immediately
        syncStatsRefresh?.();
      })
    );

    unlisteners.push(
      await listen<{ company?: string; path: string; message: string }>(
        'sync:error',
        async (event) => {
          syncState = 'error';
          syncProgress = null;
          syncErrorMessage = event.payload.message;
          await invoke('set_tray_state', { state: 'error' });
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
  {:else if authenticated && showSettings}
    <Settings onback={handleBackFromSettings} />
  {:else if authenticated}
    <Popover
      {syncState}
      {config}
      progress={syncProgress}
      fanoutTotal={syncFanoutTotal}
      fanoutDoneCount={syncFanoutDoneCount}
      lastSummary={syncLastSummary}
      errorMessage={syncErrorMessage}
      {conflicts}
      {showConflictModal}
      onsync={handleSyncNow}
      onsettings={handleSettings}
      onsignout={handleSignOut}
      onresolve={handleResolveConflict}
      onopen={handleOpenInEditor}
      ondismissconflicts={handleDismissConflicts}
      bindStatsRefresh={(fn) => (syncStatsRefresh = fn)}
    />
  {:else}
    <SignInPrompt onsuccess={handleAuthSuccess} />
  {/if}
</main>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    padding: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto,
      Oxygen, Ubuntu, Cantarell, sans-serif;
    /* Transparent so the Popover's rounded corners show the desktop
       behind them (tauri window is transparent). The popover root
       component paints its own background + border-radius. */
    background: transparent;
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
