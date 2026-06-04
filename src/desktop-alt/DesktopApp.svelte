<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import { loadMeetingsCache } from '../lib/meetingsCache';
  import type { Workspace, WorkspacesResult } from '../lib/workspaces';
  import SyncPage from './pages/SyncPage.svelte';
  import MeetingsPage from './pages/MeetingsPage.svelte';
  import LibraryPage from './pages/LibraryPage.svelte';
  import CompanyPage from './pages/CompanyPage.svelte';
  import { startMeetingsStore } from './lib/meetings-store.svelte';
  import { startCompanyStore } from './lib/company-store.svelte';
  import {
    DESKTOP_SHELL_LAYOUT,
    getDesktopActiveCompany,
    getDesktopCompanies,
    getDesktopHotkeyRoute,
    getDesktopRouteKey,
    initialDesktopRoute,
    type DesktopRoute,
  } from './route';
  import DesktopSidebar from './DesktopSidebar.svelte';
  import DesktopStatusBar from './DesktopStatusBar.svelte';
  import CommandPalette, {
    type CommandPaletteItem,
  } from './components/CommandPalette.svelte';
  import {
    eventStart,
    isToday,
    sortByStart,
    type GoogleAccount,
    type GoogleCalendar,
    type MeetingEvent,
    type ScheduledBot,
  } from './lib/meetings-model';
  import {
    emptyWorkspaceStats,
    type ActivityEntry,
    type DaemonStatus,
    type SyncCompanyRef,
    type SyncProgress,
    type SyncState,
    type SyncStatus,
    type WorkspaceSyncStats,
  } from './lib/sync-model';
  import './styles/desktop-alt.css';

  let route = $state<DesktopRoute>(initialDesktopRoute);
  let workspaces = $state<Workspace[]>([]);
  let workspacesCloudReachable = $state(true);
  let workspaceError = $state<string | null>(null);
  let workspaceManifestError = $state<string | null>(null);
  let syncState = $state<SyncState>('idle');
  let syncProgress = $state<SyncProgress | null>(null);
  let syncFanoutTotal = $state(0);
  let syncFanoutDoneCount = $state(0);
  let syncCompanies = $state<SyncCompanyRef[]>([]);
  let syncFilesProgressed = $state(0);
  let syncTotalFiles = $state(0);
  let syncPlanTotalFiles = $state(0);
  let syncFanoutFilesSkipped = $state(0);
  let syncLastSummary = $state<{
    companiesAttempted: number;
    filesDownloaded: number;
    bytesDownloaded: number;
    filesSkipped: number;
  } | null>(null);
  let syncErrorMessage = $state('');
  let statsBySlug = $state<Record<string, WorkspaceSyncStats>>({});
  let activity = $state<ActivityEntry[]>([]);
  let status = $state<SyncStatus | null>(null);
  let daemon = $state<DaemonStatus | null>(null);
  // Flips true once the first real-state load (workspaces + status + activity)
  // resolves, so the Sync surface shows skeletons instead of a 0/empty flash
  // during the initial fetch window.
  let ready = $state(false);
  let actionMessage = $state('');
  let actionError = $state('');
  let commandPaletteOpen = $state(false);
  let meetingEvents = $state<MeetingEvent[]>([]);
  let meetingCompanyNamesByUid = $state<Map<string, string>>(new Map());
  let meetingStatusNow = $state(Date.now());

  const companies = $derived(getDesktopCompanies(workspaces));
  const routeKey = $derived(getDesktopRouteKey(route));
  const activeCompany = $derived(getDesktopActiveCompany(route, companies));
  const effectiveTotalFiles = $derived(syncPlanTotalFiles > 0 ? syncPlanTotalFiles : syncTotalFiles);
  const indexedFiles = $derived(
    syncPlanTotalFiles > 0
      ? syncPlanTotalFiles
      : syncTotalFiles > 0
        ? syncTotalFiles
        : Math.max(syncFilesProgressed, status?.pendingFiles ?? 0),
  );
  const observedVaultBytes = $derived.by(() => {
    const activityBytes = activity.reduce((sum, entry) => sum + entry.bytes, 0);
    const workspaceBytes = Object.values(statsBySlug).reduce(
      (sum, stats) => sum + Math.max(stats.transferredBytes, stats.completedBytes),
      0,
    );
    return Math.max(activityBytes, workspaceBytes, syncLastSummary?.bytesDownloaded ?? 0);
  });
  const nextMeetingLabel = $derived.by(() => {
    const now = new Date(meetingStatusNow);
    const upcoming = meetingEvents
      .filter((event) => isToday(event, now))
      .filter((event) => (eventStart(event)?.getTime() ?? 0) >= now.getTime())
      .sort(sortByStart)[0];
    const startsAt = upcoming ? eventStart(upcoming) : null;
    if (!upcoming || !startsAt) return null;

    const company =
      (upcoming.sourceCompanyUid
        ? meetingCompanyNamesByUid.get(upcoming.sourceCompanyUid) ?? upcoming.sourceCompanyUid
        : null) ?? 'Meetings';
    const minutes = Math.max(0, Math.ceil((startsAt.getTime() - now.getTime()) / 60000));
    return `${company} · in ${minutes}m`;
  });
  const commandItems = $derived<CommandPaletteItem[]>([
    {
      id: 'command-sync-now',
      label: 'Sync now',
      detail: 'Start a full workspace sync',
      action: handleSyncAll,
    },
    {
      id: 'command-open-settings',
      label: 'Open settings',
      detail: 'Open sync settings',
      action: handleOpenSettings,
    },
    {
      id: 'command-go-sync',
      label: 'Go to Sync',
      detail: 'Show sync overview',
      shortcut: '⌘1',
      action: () => navigate({ kind: 'sync' }),
    },
    {
      id: 'command-go-meetings',
      label: 'Go to Meetings',
      detail: 'Show calendar and recordings',
      shortcut: '⌘2',
      action: () => navigate({ kind: 'meetings' }),
    },
    {
      id: 'command-go-library',
      label: 'Go to Library',
      detail: 'Browse skills and workers',
      shortcut: '⌘3',
      action: () => navigate({ kind: 'library' }),
    },
    ...companies.map((company, index) => ({
      id: `command-go-company-${company.slug}`,
      label: `Go to ${company.displayName}`,
      detail: 'Show company workspace',
      shortcut: index < 4 ? `⌘${index + 4}` : undefined,
      action: () => navigate({ kind: 'company', slug: company.slug }),
    })),
  ]);

  function navigate(nextRoute: DesktopRoute) {
    route = nextRoute;
  }

  function hydrateMeetingStatus() {
    const snapshot = loadMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>();
    meetingEvents = snapshot?.events ?? [];
    meetingCompanyNamesByUid = new Map(snapshot?.companyNamesByUid ?? []);
  }

  function resetRunState(options: { preserveTotalFiles?: boolean } = {}) {
    const previousTotalFiles = syncTotalFiles;
    syncState = 'syncing';
    syncProgress = null;
    syncFanoutTotal = 0;
    syncFanoutDoneCount = 0;
    syncCompanies = [];
    syncFanoutFilesSkipped = 0;
    syncFilesProgressed = 0;
    syncTotalFiles = options.preserveTotalFiles ? previousTotalFiles : 0;
    syncPlanTotalFiles = 0;
    syncLastSummary = null;
    syncErrorMessage = '';
    statsBySlug = {};
  }

  function updateWorkspaceStats(slug: string, update: (stats: WorkspaceSyncStats) => WorkspaceSyncStats) {
    const current = statsBySlug[slug] ?? emptyWorkspaceStats();
    statsBySlug = { ...statsBySlug, [slug]: update(current) };
  }

  async function loadWorkspaces() {
    try {
      const result = await invoke<WorkspacesResult>('list_syncable_workspaces');
      workspaces = result.workspaces;
      workspacesCloudReachable = result.cloudReachable;
      workspaceError = result.error;
      workspaceManifestError = result.manifestError;
      // Warm the company-tab preload cache for every known company once the real
      // slugs resolve. Idempotent + reconciles, so companies that appear on a
      // later refresh still get warmed; the 30s poll + focus listener wire once.
      startCompanyStore(getDesktopCompanies(result.workspaces).map((company) => company.slug));
    } catch (err) {
      console.error('list_syncable_workspaces failed:', err);
      workspacesCloudReachable = false;
      workspaceError = String(err);
    }
  }

  async function loadSyncStatus() {
    try {
      status = await invoke<SyncStatus>('get_sync_status');
    } catch (err) {
      console.error('get_sync_status failed:', err);
    }
  }

  async function loadDaemonStatus() {
    try {
      daemon = await invoke<DaemonStatus>('daemon_status');
    } catch (err) {
      console.error('daemon_status failed:', err);
    }
  }

  async function loadActivity() {
    try {
      activity = await invoke<ActivityEntry[]>('get_activity_log');
    } catch (err) {
      console.error('get_activity_log failed:', err);
    }
  }

  async function refreshRealState() {
    await Promise.all([loadWorkspaces(), loadSyncStatus(), loadDaemonStatus(), loadActivity()]);
  }

  async function handleSyncAll() {
    if (syncState === 'syncing') return;
    actionError = '';
    actionMessage = '';
    resetRunState();
    try {
      await invoke('set_tray_state', { state: 'syncing' });
      await invoke('start_sync');
    } catch (err) {
      console.error('start_sync failed:', err);
      syncState = 'error';
      syncErrorMessage = String(err);
      actionError = 'Could not start sync.';
      await invoke('set_tray_state', { state: 'error' }).catch(() => undefined);
    }
  }

  async function handleOpenSettings() {
    actionError = '';
    actionMessage = '';
    try {
      await invoke('open_settings_window');
    } catch (err) {
      console.error('open_settings_window failed:', err);
      actionError = 'Could not open Settings.';
    }
  }

  function handleAddSource() {
    actionError = '';
    actionMessage = 'Coming soon.';
  }

  function handleKeydown(event: KeyboardEvent) {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      commandPaletteOpen = true;
      return;
    }

    if (commandPaletteOpen) return;

    const nextRoute = getDesktopHotkeyRoute(event, companies);
    if (!nextRoute) return;

    event.preventDefault();
    navigate(nextRoute);
  }

  onMount(() => {
    let mounted = true;
    let unlistenFocus: UnlistenFn | undefined;
    const unlisteners: UnlistenFn[] = [];
    const meetingStatusInterval = window.setInterval(() => {
      meetingStatusNow = Date.now();
      hydrateMeetingStatus();
    }, 30_000);

    void refreshRealState().finally(() => {
      if (mounted) ready = true;
    });
    hydrateMeetingStatus();
    // Warm the Meetings singleton at app launch so its data is ready before the
    // user ever navigates to Meetings — the page then reads the warm store on
    // remount (instant) instead of running a blocking fetch each nav. Idempotent.
    startMeetingsStore();
    // A notification click can request a specific screen (e.g. Meetings) before
    // this window existed. The opener queued it; consume it once on mount so we
    // land on the right screen instead of the default Sync route. The
    // already-open case is handled live by the `desktop:navigate` listener below.
    void invoke<string | null>('desktop_alt_consume_pending_route')
      .then((pending) => {
        if (mounted && pending === 'meetings') {
          navigate({ kind: 'meetings' });
        }
      })
      .catch(() => undefined);
    window.addEventListener('keydown', handleKeydown);
    window.addEventListener('focus', hydrateMeetingStatus);
    window.addEventListener('storage', hydrateMeetingStatus);

    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) {
          refreshRealState();
          hydrateMeetingStatus();
        }
      })
      .then((unlisten) => {
        if (mounted) {
          unlistenFocus = unlisten;
        } else {
          unlisten();
        }
      });

    void Promise.all([
      listen<{ totalFiles: number }>('sync:totals', (event) => {
        if (syncState !== 'syncing') {
          resetRunState();
        }
        syncTotalFiles = event.payload.totalFiles;
      }),
      listen<{
        company: string;
        filesToDownload: number;
        bytesToDownload: number;
        filesToUpload: number;
        bytesToUpload: number;
        filesToSkip: number;
        filesToConflict: number;
      }>('sync:plan', (event) => {
        const plannedFiles =
          event.payload.filesToDownload +
          event.payload.filesToUpload +
          event.payload.filesToConflict;
        const plannedBytes = event.payload.bytesToDownload + event.payload.bytesToUpload;
        syncPlanTotalFiles += plannedFiles;
        updateWorkspaceStats(event.payload.company, (stats) => ({
          ...stats,
          plannedFiles: stats.plannedFiles + plannedFiles,
          plannedBytes: stats.plannedBytes + plannedBytes,
        }));
      }),
      listen<{ companies: SyncCompanyRef[] }>('sync:fanout-plan', async (event) => {
        if (syncState !== 'syncing') {
          resetRunState({ preserveTotalFiles: true });
        }
        syncFanoutTotal = event.payload.companies.length;
        syncFanoutDoneCount = 0;
        syncCompanies = event.payload.companies;
        await invoke('set_tray_state', { state: 'syncing' }).catch(() => undefined);
      }),
      listen<{ company: string; path: string; bytes: number; message?: string }>(
        'sync:progress',
        async (event) => {
          syncState = 'syncing';
          syncProgress = {
            company: event.payload.company,
            path: event.payload.path,
            bytes: event.payload.bytes,
          };
          syncFilesProgressed += 1;
          updateWorkspaceStats(event.payload.company, (stats) => ({
            ...stats,
            progressedFiles: stats.progressedFiles + 1,
            transferredBytes: stats.transferredBytes + event.payload.bytes,
            lastEventAt: Date.now(),
          }));
          await invoke('set_tray_state', { state: 'syncing' }).catch(() => undefined);
        },
      ),
      listen<{
        personUid: string;
        filesDone: number;
        filesTotal: number;
        currentFile: string | null;
      }>('sync:personal-first-push-progress', async (event) => {
        syncState = 'syncing';
        syncFilesProgressed = Math.max(syncFilesProgressed, event.payload.filesDone);
        syncTotalFiles = Math.max(syncTotalFiles, event.payload.filesTotal);
        if (event.payload.currentFile) {
          syncProgress = {
            company: 'personal',
            path: event.payload.currentFile,
            bytes: 0,
          };
        }
        updateWorkspaceStats('personal', (stats) => ({
          ...stats,
          progressedFiles: Math.max(stats.progressedFiles, event.payload.filesDone),
          plannedFiles: Math.max(stats.plannedFiles, event.payload.filesTotal),
          lastEventAt: Date.now(),
        }));
        await invoke('set_tray_state', { state: 'syncing' }).catch(() => undefined);
      }),
      listen<{ personUid: string; filesUploaded: number; filesSkipped: number }>(
        'sync:personal-first-push-complete',
        (event) => {
          syncFilesProgressed = Math.max(syncFilesProgressed, event.payload.filesUploaded);
          updateWorkspaceStats('personal', (stats) => ({
            ...stats,
            completedFiles: Math.max(stats.completedFiles, event.payload.filesUploaded),
            skippedFiles: stats.skippedFiles + event.payload.filesSkipped,
            lastEventAt: Date.now(),
          }));
        },
      ),
      listen<{
        company: string;
        filesDownloaded: number;
        bytesDownloaded: number;
        filesSkipped: number;
        conflicts: number;
        aborted: boolean;
      }>('sync:complete', async (event) => {
        syncFanoutDoneCount += 1;
        syncFanoutFilesSkipped += event.payload.filesSkipped;
        updateWorkspaceStats(event.payload.company, (stats) => ({
          ...stats,
          completedBytes: Math.max(stats.completedBytes, event.payload.bytesDownloaded),
          completedFiles: stats.completedFiles + event.payload.filesDownloaded,
          skippedFiles: stats.skippedFiles + event.payload.filesSkipped,
          conflicts: stats.conflicts + event.payload.conflicts,
          aborted: stats.aborted || event.payload.aborted,
          lastEventAt: Date.now(),
        }));
        if (event.payload.aborted) {
          syncState = 'conflict';
          await invoke('set_tray_state', { state: 'conflict' }).catch(() => undefined);
        }
      }),
      listen<{
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
        if (syncState !== 'conflict' && syncState !== 'error') {
          syncState = event.payload.errors.length > 0 ? 'error' : 'idle';
          await invoke('set_tray_state', { state: syncState === 'idle' ? 'idle' : 'error' }).catch(
            () => undefined,
          );
        }
        if (event.payload.errors.length > 0) {
          syncErrorMessage = event.payload.errors.map((item) => item.message).join('; ');
        }
        await refreshRealState();
      }),
      listen<{ company?: string; path: string; message: string }>('sync:error', async (event) => {
        syncState = 'error';
        syncProgress = null;
        syncErrorMessage = event.payload.message;
        if (event.payload.company) {
          updateWorkspaceStats(event.payload.company, (stats) => ({
            ...stats,
            errorMessage: event.payload.message,
          }));
        }
        await invoke('set_tray_state', { state: 'error' }).catch(() => undefined);
      }),
      listen<{ message: string }>('sync:auth-error', async (event) => {
        syncState = 'auth-error';
        syncProgress = null;
        syncErrorMessage = event.payload.message;
        await invoke('set_tray_state', { state: 'error' }).catch(() => undefined);
      }),
      listen<ActivityEntry>('activity:append', (event) => {
        activity = [...activity, event.payload];
      }),
      listen<ActivityEntry[]>('activity:list', (event) => {
        activity = event.payload;
      }),
      // Live navigation request from the backend — fired when the window is
      // already open and a notification click (or other intent) wants a
      // specific screen. The fresh-window case is handled by the
      // `desktop_alt_consume_pending_route` consume above.
      listen<string>('desktop:navigate', (event) => {
        if (event.payload === 'meetings') {
          navigate({ kind: 'meetings' });
        }
      }),
    ]).then((offs) => {
      if (mounted) {
        unlisteners.push(...offs);
      } else {
        offs.forEach((off) => off());
      }
    });

    return () => {
      mounted = false;
      unlistenFocus?.();
      unlisteners.forEach((unlisten) => unlisten());
      window.clearInterval(meetingStatusInterval);
      window.removeEventListener('keydown', handleKeydown);
      window.removeEventListener('focus', hydrateMeetingStatus);
      window.removeEventListener('storage', hydrateMeetingStatus);
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
          {#if route.kind === 'sync'}
            <div class="page">
              <SyncPage
                {workspaces}
                {syncState}
                {ready}
                progress={syncProgress}
                companies={syncCompanies}
                {status}
                {daemon}
                {indexedFiles}
                {observedVaultBytes}
                {statsBySlug}
                cloudReachable={workspacesCloudReachable}
                cloudError={workspaceError}
                manifestError={workspaceManifestError}
                {activity}
                {syncErrorMessage}
                onsync={handleSyncAll}
                onsettings={handleOpenSettings}
                onaddsource={handleAddSource}
                {actionMessage}
                {actionError}
              />
            </div>
          {:else if route.kind === 'meetings'}
            <div class="page">
              <MeetingsPage />
            </div>
          {:else if route.kind === 'library'}
            <div class="page">
              <LibraryPage />
            </div>
          {:else if activeCompany}
            <div class="page">
              <CompanyPage company={activeCompany} />
            </div>
          {:else}
            <section class="page" aria-labelledby="desktop-page-title">
              <div class="page-header">
                <h1 id="desktop-page-title">Company</h1>
              </div>
              <div class="placeholder-panel">
                <p>This company isn’t synced yet. Run a sync to load its board.</p>
                {#if workspaceError}
                  <span class="workspace-error">{workspaceError}</span>
                {/if}
              </div>
            </section>
          {/if}
        {/key}
      </div>

      <DesktopStatusBar
        version={__APP_VERSION__}
        state={syncState}
        progress={syncProgress}
        filesProgressed={syncFilesProgressed}
        totalFiles={effectiveTotalFiles}
        {nextMeetingLabel}
      />
    </main>
  </div>

  {#if commandPaletteOpen}
    <CommandPalette commands={commandItems} onclose={() => (commandPaletteOpen = false)} />
  {/if}
</div>
