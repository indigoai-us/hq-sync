<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import SignInPrompt from './components/SignInPrompt.svelte';
  import Popover from './components/Popover.svelte';
  import Settings from './components/Settings.svelte';
  import { conflictStore, type ConflictFile } from './stores/conflicts';
  import { shouldSkipSignIn } from './lib/auth';
  import type { Workspace, WorkspacesResult } from './lib/workspaces';
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
  // Company list from the last fanout-plan event. `name` is optional —
  // runners < v5.1.9 only emit `uid` + `slug`, so the UI falls back to the
  // slug in that case. Rendered by Popover so the user sees *which* HQs
  // they're connected to.
  let syncCompanies = $state<Array<{ uid: string; slug: string; name?: string }>>([]);
  // filesSkipped is not on sync:all-complete (backend only aggregates
  // filesDownloaded), so we sum it client-side from per-company complete
  // events. Lets the popover surface "Up to date" when everything was
  // current instead of misreading as "Last sync · 0 files".
  let syncFanoutFilesSkipped = $state(0);
  let syncLastSummary = $state<{
    companiesAttempted: number;
    filesDownloaded: number;
    bytesDownloaded: number;
    filesSkipped: number;
  } | null>(null);
  let syncErrorMessage = $state(''); // Last auth-error or error message
  let showConflictModal = $state(false);
  let conflicts = $state<ConflictFile[]>([]);
  let showSettings = $state(false);
  let syncStatsRefresh = $state<(() => void) | null>(null);

  // Workspaces — populated by `list_syncable_workspaces` (Rust). Replaces the
  // legacy "No companies yet" dead-end with a union over Person + memberships
  // + local company folders. `null` = first invocation in flight; non-null
  // (even empty) = command completed at least once.
  let workspaces = $state<Workspace[] | null>(null);
  let workspacesCloudReachable = $state(true);
  let workspacesError = $state<string | null>(null);

  // Updater state — populated by the `update:available` event from the Rust
  // background checker (launch+10s, then every 6h). Non-null means the user
  // is on an older version and the banner should be shown.
  let updateAvailable = $state<{ version: string; body?: string; date?: string } | null>(null);
  // True while `invoke('install_update')` is in-flight — blocks duplicate
  // clicks and lets the button show a spinner. On macOS the process usually
  // terminates before the promise resolves, so this rarely flips back.
  let updateInstalling = $state(false);

  // Collected unlisten handles for cleanup
  let unlisteners: UnlistenFn[] = [];

  async function loadConfig() {
    try {
      config = await invoke<Config>('get_config');
    } catch (err) {
      console.error('Failed to load config:', err);
    }
  }

  /**
   * Fetch the workspaces union (Personal + memberships + local folders).
   * Called on mount, after sync completes, and after settings change. Errors
   * surface via the `cloudReachable` flag in the result — the Rust command
   * never throws for cloud-side problems, only for environment failures
   * (e.g. cannot resolve hq folder path).
   */
  async function loadWorkspaces() {
    try {
      const result = await invoke<WorkspacesResult>('list_syncable_workspaces');
      workspaces = result.workspaces;
      workspacesCloudReachable = result.cloudReachable;
      workspacesError = result.error;
    } catch (err) {
      // Hard failure (e.g. couldn't resolve hq_root). Keep prior workspaces
      // visible if we had any, but flag the error so the UI can soften.
      console.error('list_syncable_workspaces failed:', err);
      workspacesCloudReachable = false;
      workspacesError = String(err);
      // Don't null out `workspaces` — last-good is better than empty.
    }
  }

  async function handleSyncNow() {
    if (syncState === 'syncing') return;
    syncState = 'syncing';
    syncProgress = null;
    syncFanoutTotal = 0;
    syncFanoutDoneCount = 0;
    syncCompanies = [];
    syncFanoutFilesSkipped = 0;
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
    // User may have changed the HQ folder path in Settings; the header in
    // Popover renders from `config.hqFolderPath`, which was snapshotted at
    // mount. Re-read menubar.json so the change is visible without a quit.
    // Workspaces depend on hq_root too — local folder enumeration would point
    // at the wrong tree otherwise.
    loadConfig();
    loadWorkspaces();
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

  async function handleInstallUpdate() {
    if (updateInstalling) return;
    updateInstalling = true;
    try {
      // Backend re-runs updater.check() inside install_update because
      // tauri_plugin_updater::Update is not Clone — we can't stash the
      // Update object across IPC. See src-tauri/src/updater.rs:41-60.
      // On macOS the app process is typically replaced before this
      // promise resolves; updateInstalling stays true by design.
      await invoke('install_update');
    } catch (err) {
      console.error('install_update failed:', err);
      updateInstalling = false;
    }
  }

  async function handleCheckForUpdates() {
    try {
      const info = await invoke<{ version: string; body?: string; date?: string } | null>(
        'check_for_updates'
      );
      // Backend also emits `update:available` on hit, so the listener
      // picks it up — but set it here too in case the listener races.
      if (info) updateAvailable = info;
    } catch (err) {
      console.error('check_for_updates failed:', err);
    }
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
      await listen<{ companies: Array<{ uid: string; slug: string; name?: string }> }>(
        'sync:fanout-plan',
        async (event) => {
          syncState = 'syncing';
          syncFanoutTotal = event.payload.companies.length;
          syncFanoutDoneCount = 0;
          syncCompanies = event.payload.companies;
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
        syncFanoutFilesSkipped += event.payload.filesSkipped;
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
          filesSkipped: syncFanoutFilesSkipped,
        };
        syncProgress = null;
        // Only flip to idle if nothing raised conflict/error mid-stream
        if (syncState !== 'conflict' && syncState !== 'error') {
          syncState = 'idle';
          await invoke('set_tray_state', { state: 'idle' });
        }
        // Refresh SyncStats so "last synced" updates immediately
        syncStatsRefresh?.();
        // Refresh workspaces — sync may have created new local folders
        // (for newly-provisioned companies) or updated last-synced timestamps.
        loadWorkspaces();
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

    // --- Updater event listener ---
    // Protocol (see src-tauri/src/updater.rs):
    //   update:available — payload { version, body?, date? }
    //     Emitted by setup_update_checker (launch+10s, every 6h) and
    //     also by check_for_updates (on-demand). Render a banner.
    unlisteners.push(
      await listen<{ version: string; body?: string; date?: string }>(
        'update:available',
        (event) => {
          updateAvailable = event.payload;
        }
      )
    );

    // Tray menu "Check for Updates" → on-demand check.
    unlisteners.push(
      await listen('tray:check-for-updates', () => {
        handleCheckForUpdates();
      })
    );
  }

  $effect(() => {
    // Performance: mark app init
    performance.mark('app-init');

    checkAuth();
    loadConfig();
    loadWorkspaces();
    setupTrayListeners();

    return () => {
      unlisteners.forEach((unlisten) => unlisten());
      unlisteners = [];
    };
  });

  async function checkAuth() {
    try {
      // Skip the sign-in step when cognito-tokens.json already holds a
      // non-empty token. See `shouldSkipSignIn` for the ordering: we
      // prefer `get_auth_state`'s verdict (it tries a silent refresh) and
      // only fall back to raw token presence when it reports
      // unauthenticated — a stored token that's actually unusable will
      // raise `sync:auth-error` on first sync and route back through
      // sign-in from there.
      const [hasToken, state] = await Promise.all([
        invoke<boolean>('has_stored_token'),
        invoke<{
          authenticated: boolean;
          expiresAt: string | null;
        }>('get_auth_state'),
      ]);

      authenticated = shouldSkipSignIn(hasToken, state);
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
      companies={syncCompanies}
      {workspaces}
      cloudReachable={workspacesCloudReachable}
      cloudError={workspacesError}
      lastSummary={syncLastSummary}
      errorMessage={syncErrorMessage}
      {conflicts}
      {showConflictModal}
      {updateAvailable}
      {updateInstalling}
      onsync={handleSyncNow}
      onsettings={handleSettings}
      onsignout={handleSignOut}
      onresolve={handleResolveConflict}
      onopen={handleOpenInEditor}
      ondismissconflicts={handleDismissConflicts}
      oninstallupdate={handleInstallUpdate}
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
    width: 100vw;
    height: 100vh;
    /* overflow:hidden prevents scrollbars from appearing on the root
       document. The popover's own scroll container (.popover-body) is
       the only legitimate scrollable region. */
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto,
      Oxygen, Ubuntu, Cantarell, sans-serif;
    /* Transparent so the Popover's rounded corners show the desktop
       behind them (tauri window is transparent). The popover root
       component paints its own background + border-radius. */
    background: transparent;
    color: var(--popover-text, #e0e0e0);
  }

  main {
    /* Fill the window exactly; popover sizes itself via 100vw/100vh.
       No centering flex — that created a sub-viewport box that could
       clip the popover if it ever exceeded window size. */
    width: 100vw;
    height: 100vh;
    padding: 0;
    overflow: hidden;
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
