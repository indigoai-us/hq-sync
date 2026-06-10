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
  import MessagesPage from './pages/MessagesPage.svelte';
  import CompanyPage from './pages/CompanyPage.svelte';
  import ModerationPanel from './panels/ModerationPanel.svelte';
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
    friendlySyncError,
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
  // Admin gate for the Moderation nav entry (UX only; the server is the sole
  // authorization boundary). DEFAULT-DENY: starts false and only flips true on an
  // explicit `desktop_alt_is_admin === true` (@getindigo.ai), so the row never
  // flashes for a non-admin and stays hidden on any check error. Reuses the same
  // signal ModerationPanel itself gates on.
  let isAdmin = $state(false);
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
      id: 'command-go-messages',
      label: 'Go to Messages',
      detail: 'Direct messages and channels',
      shortcut: '⌘3',
      action: () => navigate({ kind: 'messages' }),
    },
    {
      id: 'command-go-skills',
      label: 'Go to Skills',
      detail: 'Browse skills',
      shortcut: '⌘4',
      action: () => navigate({ kind: 'library', tab: 'skills' }),
    },
    {
      id: 'command-go-workers',
      label: 'Go to Workers',
      detail: 'Browse workers',
      shortcut: '⌘5',
      action: () => navigate({ kind: 'library', tab: 'workers' }),
    },
    {
      id: 'command-go-installed',
      label: 'Go to Installed',
      detail: 'Marketplace packs installed in your HQ',
      shortcut: '⌘6',
      action: () => navigate({ kind: 'library', tab: 'installed' }),
    },
    {
      id: 'command-go-marketplace',
      label: 'Go to Marketplace',
      detail: 'Discover and install skills and workers',
      shortcut: '⌘7',
      action: () => navigate({ kind: 'library', tab: 'marketplace' }),
    },
    {
      id: 'command-go-profile',
      label: 'Go to Profile',
      detail: 'Your HQ profile and published work',
      shortcut: '⌘8',
      action: () => navigate({ kind: 'library', tab: 'profile' }),
    },
    ...companies.map((company, index) => ({
      id: `command-go-company-${company.slug}`,
      label: `Go to ${company.displayName}`,
      detail: 'Show company workspace',
      // Companies start at ⌘9 (after the 8 primary destinations); only ⌘9 is
      // single-digit addressable.
      shortcut: index < 1 ? `⌘${index + 9}` : undefined,
      action: () => navigate({ kind: 'company', slug: company.slug }),
    })),
  ]);

  function formatRelative(iso: string | null): string | null {
    if (!iso) return null;
    const then = new Date(iso).getTime();
    if (Number.isNaN(then)) return null;
    const secs = Math.max(0, Math.round((Date.now() - then) / 1000));
    if (secs < 60) return 'just now';
    const mins = Math.round(secs / 60);
    if (mins < 60) return `${mins}m ago`;
    const hrs = Math.round(mins / 60);
    if (hrs < 24) return `${hrs}h ago`;
    return `${Math.round(hrs / 24)}d ago`;
  }

  // The always-visible sync verdict shown in the title bar: a tone (drives the
  // status dot color), a one-word state, and a mono detail line.
  const verdict = $derived.by(() => {
    const total = companies.length;
    if (syncState === 'syncing') {
      const scope =
        syncFanoutTotal > 0 ? `${syncFanoutDoneCount}/${syncFanoutTotal} companies` : 'workspaces';
      return {
        tone: 'syncing',
        word: 'Syncing',
        counts: syncProgress?.company ? `${syncProgress.company} · ${scope}` : scope,
      };
    }
    if (syncState === 'error' || syncState === 'auth-error') {
      return {
        tone: 'error',
        word: 'Sync error',
        counts: syncErrorMessage
          ? friendlySyncError(syncErrorMessage).summary
          : 'check your connection',
      };
    }
    if (syncState === 'conflict') {
      return { tone: 'conflict', word: 'Needs attention', counts: 'resolve conflicts to continue' };
    }
    const pending = status?.pendingFiles ?? 0;
    return {
      tone: 'idle',
      word: 'All synced',
      counts:
        pending > 0
          ? `${pending} pending · ${total} watched`
          : `${total} workspace${total === 1 ? '' : 's'} watched`,
    };
  });

  const lastSyncLabel = $derived(formatRelative(status?.lastSyncAt ?? null));

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
    // Resolve the admin gate for the Moderation nav entry (default-deny: only an
    // explicit `true` unlocks it; any error leaves it hidden). This MUST use the
    // admin gate (`desktop_alt_is_admin` → @getindigo.ai), NOT `desktop_alt_enabled`
    // (the GA gate, true for every signed-in user) — otherwise the Moderation row
    // shows for normal HQ users.
    void invoke<boolean>('desktop_alt_is_admin')
      .then((admin) => {
        if (mounted) isAdmin = admin === true;
      })
      .catch(() => {
        if (mounted) isAdmin = false;
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
  style={`--desktop-sidebar-width: ${DESKTOP_SHELL_LAYOUT.sidebarWidthPx}px; --desktop-titlebar-height: ${DESKTOP_SHELL_LAYOUT.titleBarHeightPx}px; --desktop-status-bar-height: ${DESKTOP_SHELL_LAYOUT.statusBarHeightPx}px;`}
>
  <header class="desktop-titlebar" data-tauri-drag-region aria-label="Sync status">
    <div class="titlebar-verdict">
      <span class={`verdict-dot ${verdict.tone}`} aria-hidden="true"></span>
      <span class="verdict-word">{verdict.word}</span>
      <span class="verdict-counts">{verdict.counts}</span>
    </div>
    <div class="titlebar-spacer"></div>
    <div class="titlebar-meta">
      {#if lastSyncLabel}
        <span>last sync <span class="meta-mono">{lastSyncLabel}</span></span>
        <span class="titlebar-divider" aria-hidden="true"></span>
      {/if}
      <button
        class="titlebar-sync-now"
        type="button"
        onclick={handleSyncAll}
        disabled={syncState === 'syncing'}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M21 12a9 9 0 1 1-2.64-6.36" />
          <path d="M21 3v5h-5" />
        </svg>
        {syncState === 'syncing' ? 'Syncing…' : 'Sync Now'}
      </button>
    </div>
  </header>

  <div class="desktop-body">
    <DesktopSidebar
      {route}
      {companies}
      {isAdmin}
      onnavigate={navigate}
      onsearch={() => (commandPaletteOpen = true)}
      onsettings={handleOpenSettings}
    />

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
              <LibraryPage tab={route.tab} />
            </div>
          {:else if route.kind === 'messages'}
            <div class="messages-host">
              <MessagesPage />
            </div>
          {:else if route.kind === 'moderation'}
            <!-- Admin-only. Rendered only when the admin gate is satisfied
                 (default-deny); ModerationPanel ALSO re-checks + locks itself, and
                 the server is the real authorization boundary. A non-admin who
                 somehow reaches this route falls through to the placeholder. -->
            {#if isAdmin}
              <div class="page">
                <ModerationPanel />
              </div>
            {:else}
              <section class="page" aria-labelledby="desktop-page-title">
                <div class="page-header">
                  <h1 id="desktop-page-title">Moderation</h1>
                </div>
                <div class="placeholder-panel">
                  <p>Moderation is restricted to reviewers.</p>
                </div>
              </section>
            {/if}
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
      </main>
    </div>
  </div>

  <DesktopStatusBar
    version={__APP_VERSION__}
    state={syncState}
    progress={syncProgress}
    filesProgressed={syncFilesProgressed}
    totalFiles={effectiveTotalFiles}
    workspaceCount={companies.length}
    observedBytes={observedVaultBytes}
    {nextMeetingLabel}
  />

  {#if commandPaletteOpen}
    <CommandPalette commands={commandItems} onclose={() => (commandPaletteOpen = false)} />
  {/if}
</div>

<style>
  /* The Messages route hosts the full-bleed MessagesShell rather than the
     padded, scrolling .page layout — it fills the content area and anchors the
     shell's absolutely-positioned host. */
  .messages-host {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
  }
</style>
