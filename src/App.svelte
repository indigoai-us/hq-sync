<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
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
  // Per-run cumulative file counter — incremented per sync:progress event so
  // the popover can show "234 files" alongside the current file. Reset on
  // each Sync Now click; not reset by sync:all-complete (the final summary
  // line takes over from there).
  let syncFilesProgressed = $state(0);
  // Personal first-push knows files_total upfront; we capture it so the
  // live-progress card can show "234 of 1,247 files" instead of just a
  // running count. Runner sync:progress events don't carry a total, so
  // these stay null during the runner phase and the UI falls back to
  // "234 files synced".
  let personalFilesDone = $state(0);
  let personalFilesTotal = $state<number | null>(null);
  // Latched flag for the unified progress bar — once the in-process Rust
  // personal first-push completes, this stays true until the next Sync
  // click. Lets the bar treat the personal phase as "fully filled (50%
  // slot)" even after personalFilesTotal has been reset, so there's no
  // visible drop between the Rust phase and the runner taking over.
  let personalFirstPushDone = $state(false);
  // Real total file count for the entire sync — emitted by the Rust pre-walk
  // BEFORE any uploads begin (sums personal allowlist + every local company
  // folder, after applying .hqignore + DEFAULT_IGNORES). Drives the unified
  // per-file progress bar. 0 means pre-walk hasn't fired yet (or hit an
  // error); the UI falls back to workspace-level progress in that case.
  let syncTotalFiles = $state(0);
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
  // Top-level manifest parse/IO error from list_syncable_workspaces. Distinct
  // from workspacesError (which surfaces cloud-side failure). Both can fire
  // independently — a broken manifest doesn't prevent us from talking to the
  // cloud, and an unreachable cloud doesn't make the manifest unreadable.
  let workspacesManifestError = $state<string | null>(null);

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
      workspacesManifestError = result.manifestError;
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
    syncFilesProgressed = 0;
    personalFilesDone = 0;
    personalFilesTotal = null;
    personalFirstPushDone = false;
    syncTotalFiles = 0;
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

  async function handleCancel() {
    if (syncState !== 'syncing') return;
    try {
      await invoke('cancel_sync');
      // Don't flip syncState here — the runner's exit triggers the
      // existing "runner exited" path which emits sync:all-complete (or
      // sync:error) and resets state. Avoids a race where cancel returns
      // before the kill propagates.
    } catch (err) {
      console.error('cancel_sync failed:', err);
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
    // Refresh workspaces every time the menubar popover gains focus. Cheap
    // (single Tauri command + small vault round-trip) and catches external
    // mutations: a new company added via /newcompany, a manifest patch from
    // a CLI tool, or any folder created outside the app between popover
    // openings. Without this, the list only refreshes on mount and after a
    // sync — a brand-new company added between syncs would stay invisible
    // until the next sync click.
    unlisteners.push(
      await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) {
          loadWorkspaces();
        }
      })
    );

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
        // Runner emits this when the caller has no memberships AND no
        // pending invites. As of the Rust auto-create patch, the personal
        // first-push provisions the person entity itself before the runner
        // even starts — so by the time we see setup-needed here, the only
        // remaining gap is "no companies yet", which is a perfectly normal
        // state for a brand-new account, not an error. Don't flip the tray
        // to red; just stay in syncing until all-complete fires.
        syncState = 'syncing';
        syncProgress = null;
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

    // Pre-walk total — fired once after JWT resolution, before any uploads.
    // Carries the real file count for this entire sync so the UI bar can
    // show actual per-file progress instead of fake workspace thirds.
    unlisteners.push(
      await listen<{ totalFiles: number }>('sync:totals', async (event) => {
        syncTotalFiles = event.payload.totalFiles;
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
          // Cumulative file counter — every per-file event from the runner
          // (or personal first-push) bumps this. The popover surfaces it as
          // "234 files" alongside the current path so the user always has
          // something moving even when individual paths scroll by.
          syncFilesProgressed += 1;
          await invoke('set_tray_state', { state: 'syncing' });
        }
      )
    );

    // ── Personal-first-push events ────────────────────────────────────────
    // The in-process Rust personal first-push fires its own progress events
    // (not routed through the runner's sync:progress channel) and carries
    // an upfront filesTotal — we feed both into the live-progress card so
    // the personal phase shows "234 of 1,247 files" while the (unknown-
    // total) runner phase shows just "234 files synced".
    unlisteners.push(
      await listen<{
        personUid: string;
        filesDone: number;
        filesTotal: number;
        currentFile: string | null;
      }>('sync:personal-first-push-progress', async (event) => {
        syncState = 'syncing';
        personalFilesDone = event.payload.filesDone;
        personalFilesTotal = event.payload.filesTotal;
        if (event.payload.currentFile) {
          syncProgress = {
            company: 'personal',
            path: event.payload.currentFile,
            bytes: 0, // personal-first-push doesn't carry per-file bytes
          };
          syncFilesProgressed += 1;
        }
        await invoke('set_tray_state', { state: 'syncing' });
      })
    );

    unlisteners.push(
      await listen<{ personUid: string; filesUploaded: number; filesSkipped: number }>(
        'sync:personal-first-push-complete',
        async () => {
          // Latch the done flag so the unified bar treats the personal
          // slot as 100% filled while the runner spins up. Don't clear
          // personalFilesTotal/Done — leaving them in place keeps the
          // file-level caption visible until the runner takes over with
          // its own caption.
          personalFirstPushDone = true;
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
        // We do NOT add filesSkipped to syncFilesProgressed: the runner
        // only emits per-file `progress` events for transfers, not skips,
        // and the new pre-walk denominator counts only transfers too.
        // Adding skips here would inflate the numerator and break the
        // ratio.
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
      {syncFilesProgressed}
      {personalFilesDone}
      {personalFilesTotal}
      {personalFirstPushDone}
      {syncTotalFiles}
      companies={syncCompanies}
      {workspaces}
      cloudReachable={workspacesCloudReachable}
      cloudError={workspacesError}
      manifestError={workspacesManifestError}
      onworkspacesrefresh={loadWorkspaces}
      lastSummary={syncLastSummary}
      errorMessage={syncErrorMessage}
      {conflicts}
      {showConflictModal}
      {updateAvailable}
      {updateInstalling}
      onsync={handleSyncNow}
      oncancel={handleCancel}
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
    border: 2.5px solid var(--popover-progress-track, rgba(255, 255, 255, 0.14));
    border-top-color: var(--popover-progress-fill, #ffffff);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
