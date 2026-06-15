<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount, tick } from 'svelte';
  import { loadMeetingsCache } from '../lib/meetingsCache';
  import type { Workspace, WorkspacesResult } from '../lib/workspaces';
  import HomePage from './pages/HomePage.svelte';
  import MeetingsPage from './pages/MeetingsPage.svelte';
  import LibraryPage from './pages/LibraryPage.svelte';
  import MessagesPage from './pages/MessagesPage.svelte';
  import CompanyPage from './pages/CompanyPage.svelte';
  import CompaniesPage from './pages/CompaniesPage.svelte';
  import SettingsPage from './pages/SettingsPage.svelte';
  import ModerationPanel from './panels/ModerationPanel.svelte';
  import { startMeetingsStore } from './lib/meetings-store.svelte';
  import { loadLocalProjects } from './lib/local-projects';
  import type { Project } from './lib/projects-model';
  import { startCompanyStore, setActiveCompany } from './lib/company-store.svelte';
  import { openAgentWorkflow } from './lib/agent-workflow';
  import {
    COMPANY_SECTIONS,
    LIBRARY_SECTIONS,
    SETTINGS_SECTIONS,
    companyHotkey,
    DEFAULT_COMPANY_TAB,
    DEFAULT_LIBRARY_TAB,
    DEFAULT_SETTINGS_TAB,
    formatRelativeTime,
    fromV4Route,
    getDesktopActiveCompany,
    getDesktopCompanies,
    getDesktopHotkeyRoute,
    getDesktopRouteKey,
    getDesktopSecondarySidebar,
    initialDesktopRoute,
    resolvePendingDesktopRoute,
    type CompanyTab,
    type DesktopRoute,
    type LibraryTab,
    type SettingsTab,
  } from './route';
  import { V4_CHROME_LAYOUT } from './v4/model';
  import type { HomeConflict, HomeCoreState } from './v4/home-model';
  import V4Sidebar from './v4/V4Sidebar.svelte';
  import V4SecondarySidebar from './v4/V4SecondarySidebar.svelte';
  import { open as openExternal } from '@tauri-apps/plugin-shell';
  import { companyConsoleUrl } from './lib/hq-console';
  import V4TitleBar from './v4/V4TitleBar.svelte';
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

  const WORKSPACE_CACHE_KEY = 'hq-sync.desktop.workspaces.v1';

  function readCachedWorkspaces(): Workspace[] {
    try {
      const raw = window.localStorage.getItem(WORKSPACE_CACHE_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      if (!Array.isArray(parsed)) return [];
      return parsed.filter(
        (item): item is Workspace =>
          item &&
          typeof item.slug === 'string' &&
          typeof item.displayName === 'string' &&
          (item.kind === 'personal' || item.kind === 'company'),
      );
    } catch {
      return [];
    }
  }

  function writeCachedWorkspaces(items: Workspace[]) {
    try {
      window.localStorage.setItem(WORKSPACE_CACHE_KEY, JSON.stringify(items));
    } catch {
      // Best-effort bootstrap cache only.
    }
  }

  const cachedWorkspaces = readCachedWorkspaces();
  const cachedCompanies = getDesktopCompanies(cachedWorkspaces);

  let route = $state<DesktopRoute>(initialDesktopRoute);
  // Admin gate for the Moderation nav entry (UX only; the server is the sole
  // authorization boundary). DEFAULT-DENY: starts false and only flips true on an
  // explicit `desktop_alt_is_admin === true` (@getindigo.ai), so the row never
  // flashes for a non-admin and stays hidden on any check error. Reuses the same
  // signal ModerationPanel itself gates on.
  let isAdmin = $state(false);
  let workspaces = $state<Workspace[]>(cachedWorkspaces);
  let workspaceError = $state<string | null>(null);
  // Whether the last workspace fetch reached the vault. Drives Companies-page
  // write gating (Connect / Retry / sync-mode toggle) + a quiet notice. Assume
  // reachable until a fetch says otherwise so we never gate on a cold cache.
  let cloudReachable = $state(true);
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
  // Company the failing run reported (when `sync:error` carried one) — drives
  // the Home error card's "Sync failed for {company}" framing.
  let syncErrorCompany = $state<string | null>(null);
  // Epoch ms when the running sync started — Home's syncing meta line.
  let syncStartedAt = $state<number | null>(null);
  // Unresolved conflicts from the `sync:conflict` stream → Home's NEEDS YOU
  // queue (Keep mine / Take theirs / Compare). Cleared when a new run starts —
  // the runner re-emits anything still conflicted.
  let homeConflicts = $state<HomeConflict[]>([]);
  // Core drift snapshot (`check_core_state` + `core-state:changed`) → Home's
  // drift card (Restore / Keep edit / View diff). `driftDismissed` is the
  // session-local "Keep edit" ack; a fresh scan re-surfaces the card.
  let coreState = $state<HomeCoreState | null>(null);
  let driftDismissed = $state(false);
  let driftRestoring = $state(false);
  // Local hq-core version ("15.0.15") for Home's meta line; null = unreadable.
  let hqVersion = $state<string | null>(null);
  // Resolved HQ folder from get_config, used for path labels and handoffs.
  let hqFolderPath = $state<string | null>(null);
  // `realtimeSync` preference (auto-sync cadence in Home's meta line).
  let autoSyncOn = $state<boolean | null>(null);
  let statsBySlug = $state<Record<string, WorkspaceSyncStats>>({});
  let activity = $state<ActivityEntry[]>([]);
  let status = $state<SyncStatus | null>(null);
  let daemon = $state<DaemonStatus | null>(null);
  // Flips true once the first real-state load (workspaces + status + activity)
  // resolves, so the Sync surface shows skeletons instead of a 0/empty flash
  // during the initial fetch window.
  let ready = $state(false);
  let commandPaletteOpen = $state(false);
  let meetingEvents = $state<MeetingEvent[]>([]);
  // Local projects across every company — ONE `get_local_projects` scan (no
  // per-company vault fan-out), feeding the Home portfolio stats + table.
  let homeProjects = $state<Project[]>([]);
  let meetingCompanyNamesByUid = $state<Map<string, string>>(new Map());
  let meetingStatusNow = $state(Date.now());
  let desktopRenderAuditQueued = false;

  let companies = $state<Workspace[]>(cachedCompanies);
  let renderCompanies = $state<Workspace[]>(cachedCompanies);
  let renderWorkspaceCount = $state(cachedCompanies.length);
  const shellCompanies = $derived(
    renderCompanies.length > 0
      ? renderCompanies
      : companies.length > 0
        ? companies
        : getDesktopCompanies(workspaces),
  );
  const watchedWorkspaceCount = $derived(shellCompanies.length);
  const routeKey = $derived(getDesktopRouteKey(route));
  const activeCompany = $derived(getDesktopActiveCompany(route, shellCompanies));
  // Point the company-store's background poll at whichever company is on screen,
  // so it re-fetches only the open company instead of all of them every 30s.
  $effect(() => {
    setActiveCompany(activeCompany?.slug ?? null);
  });
  const libraryTab = $derived<LibraryTab>(
    route.kind === 'library' ? route.tab ?? DEFAULT_LIBRARY_TAB : DEFAULT_LIBRARY_TAB,
  );
  const companyTab = $derived<CompanyTab>(
    route.kind === 'company' ? route.tab ?? DEFAULT_COMPANY_TAB : DEFAULT_COMPANY_TAB,
  );
  // Secondary (contextual) sidebar — only on company / library / settings
  // surfaces (SPEC section 4); null hides the column entirely.
  const secondarySidebar = $derived(
    getDesktopSecondarySidebar(route, shellCompanies, {
      version: __APP_VERSION__,
      hqFolderPath,
    }),
  );
  const effectiveTotalFiles = $derived(syncPlanTotalFiles > 0 ? syncPlanTotalFiles : syncTotalFiles);
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
      id: 'command-deploy',
      label: activeCompany ? `Deploy a result for ${activeCompany.displayName}` : 'Deploy a result',
      detail: 'Open the HQ deploy workflow in Claude Code',
      action: () => runDesktopWorkflow('deploy'),
    },
    {
      id: 'command-share',
      label: 'Share a file',
      detail: 'Mint an encrypted single-use share link',
      action: () => runDesktopWorkflow('share'),
    },
    {
      id: 'command-run-worker',
      label: activeCompany ? `Run a worker for ${activeCompany.displayName}` : 'Run a worker',
      detail: 'Hand work to a specialized agent',
      action: () => runDesktopWorkflow('run-worker'),
    },
    {
      id: 'command-go-home',
      label: 'Go to Home',
      detail: 'Sync health and activity',
      shortcut: '⌘1',
      action: () => navigate({ kind: 'home' }),
    },
    {
      id: 'command-go-companies',
      label: 'Go to Companies',
      detail: 'Connected companies overview',
      shortcut: '⌘2',
      action: () => navigate({ kind: 'companies' }),
    },
    {
      id: 'command-go-messages',
      label: 'Go to Messages',
      detail: 'Direct messages and channels',
      shortcut: '⌘3',
      action: () => navigate({ kind: 'messages' }),
    },
    {
      id: 'command-go-meetings',
      label: 'Go to Meetings',
      detail: 'Show calendar and recordings',
      shortcut: '⌘4',
      action: () => navigate({ kind: 'meetings' }),
    },
    {
      id: 'command-go-library',
      label: 'Go to Library',
      detail: 'Skills, workers, and the marketplace',
      shortcut: '⌘5',
      action: () => navigate({ kind: 'library' }),
    },
    ...LIBRARY_SECTIONS.filter((section) => section.id !== DEFAULT_LIBRARY_TAB).map(
      (section) => ({
        id: `command-go-library-${section.id}`,
        label: `Go to Library ${section.label}`,
        detail: `Show ${section.label.toLowerCase()} in the library`,
        action: () => navigate({ kind: 'library', tab: section.id }),
      }),
    ),
    {
      id: 'command-go-settings',
      label: 'Go to Settings',
      detail: 'Sync preferences and account',
      action: () => navigate({ kind: 'settings' }),
    },
    ...SETTINGS_SECTIONS.filter((section) => section.id !== DEFAULT_SETTINGS_TAB).map(
      (section) => ({
        id: `command-go-settings-${section.id}`,
        label: `Go to Settings ${section.label}`,
        detail: `Open ${section.label.toLowerCase()} settings`,
        action: () => navigate({ kind: 'settings', tab: section.id }),
      }),
    ),
    // Admin-only (default-deny) — Moderation has no sidebar row in the V4 IA,
    // so the palette is its navigation surface.
    ...(isAdmin
      ? [
          {
            id: 'command-go-moderation',
            label: 'Go to Moderation',
            detail: 'Review marketplace submissions',
            action: () => navigate({ kind: 'moderation' }),
          },
        ]
      : []),
    ...shellCompanies.flatMap((company, index) => [
      {
        id: `command-go-company-${company.slug}`,
        label: `Go to ${company.displayName}`,
        detail: 'Show company overview',
        // Companies start at ⌘6 (after the five primary destinations).
        shortcut: companyHotkey(index),
        action: () => navigate({ kind: 'company', slug: company.slug }),
      },
      ...COMPANY_SECTIONS.filter((section) => section.id !== DEFAULT_COMPANY_TAB).map(
        (section) => ({
          id: `command-go-company-${company.slug}-${section.id}`,
          label: `Go to ${company.displayName} ${section.label}`,
          detail: `Show ${company.displayName} ${section.label.toLowerCase()}`,
          action: () => navigate({ kind: 'company', slug: company.slug, tab: section.id }),
        }),
      ),
    ]),
  ]);

  // Plain-language error summary for the V4 title bar's error state.
  const titleBarErrorSummary = $derived(
    syncErrorMessage ? friendlySyncError(syncErrorMessage).summary : null,
  );

  const lastSyncLabel = $derived(formatRelativeTime(status?.lastSyncAt ?? null));

  // Live transfer total for Home's syncing progress card.
  const syncTransferredBytes = $derived(
    Object.values(statsBySlug).reduce((sum, stats) => sum + stats.transferredBytes, 0),
  );

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
    syncErrorCompany = null;
    syncStartedAt = Date.now();
    // The runner re-emits anything still conflicted; stale cards would offer
    // actions against files the new run may have already reconciled.
    homeConflicts = [];
    statsBySlug = {};
  }

  function updateWorkspaceStats(slug: string, update: (stats: WorkspaceSyncStats) => WorkspaceSyncStats) {
    const current = statsBySlug[slug] ?? emptyWorkspaceStats();
    statsBySlug = { ...statsBySlug, [slug]: update(current) };
  }

  function queueDesktopRenderAudit() {
    // Dev-only instrumentation: the backend `desktop_alt_dev_audit_render`
    // command no-ops unless HQ_DEV_AUDIT_DESKTOP_RENDER=1, so in a production
    // build these timers only burn `document.body.textContent` scans + IPC for
    // nothing. Gate the whole thing out of prod (import.meta.env.DEV is false
    // in the Tauri release build, true under `vite`/`tauri dev`).
    if (!import.meta.env.DEV) return;
    if (desktopRenderAuditQueued) return;
    desktopRenderAuditQueued = true;
    for (const delayMs of [250, 1_000, 3_000, 7_000]) {
      window.setTimeout(() => {
        void auditDesktopRender();
      }, delayMs);
    }
  }

  async function auditDesktopRender() {
    await tick();
    const names = Array.from(document.querySelectorAll<HTMLElement>('.v4-company-row'))
      .map((row) => row.textContent?.trim() ?? '')
      .filter(Boolean);
    const footer =
      document
        .querySelector<HTMLElement>('.desktop-status-bar')
        ?.textContent?.replace(/\s+/g, ' ')
        .trim() ?? null;
    const hasMoreCompaniesText = (document.body.textContent ?? '').includes('more companies');
    const domWorkspaceCount =
      document.querySelector<HTMLElement>('.desktop-shell')?.dataset.workspaceCount ?? 'missing';
    const stateSummary = `state companies=${companies.length} workspaces=${workspaces.length} render=${renderWorkspaceCount} shell=${shellCompanies.length} watched=${watchedWorkspaceCount} dom=${domWorkspaceCount}`;
    await invoke('desktop_alt_dev_audit_render', {
      companyRowCount: names.length,
      footer,
      names: [stateSummary, ...names],
      hasMoreCompaniesText,
    }).catch(() => undefined);
  }

  async function loadWorkspaces() {
    try {
      const result = await invoke<WorkspacesResult>('list_syncable_workspaces');
      const nextCompanies = getDesktopCompanies(result.workspaces);
      workspaces = result.workspaces;
      companies = nextCompanies;
      renderCompanies = nextCompanies;
      renderWorkspaceCount = nextCompanies.length;
      workspaceError = result.error;
      cloudReachable = result.cloudReachable;
      writeCachedWorkspaces(result.workspaces);
      // The chrome (V4Sidebar / V4TitleBar / DesktopStatusBar) consumes
      // renderCompanies + renderWorkspaceCount reactively ($derived / $props),
      // so the reassignments above refresh it on their own. We deliberately do
      // NOT reload the document or remount the chrome on a workspace-list change:
      // a full reload mid-paint is what blanked/froze the desktop on focus/sync.
      // Warm the company-tab preload cache for every known company once the real
      // slugs resolve. Idempotent + reconciles, so companies that appear on a
      // later refresh still get warmed; the 30s poll + focus listener wire once.
      startCompanyStore(
        nextCompanies
          .filter(
            (company) =>
              company.state === 'synced' || company.state === 'cloud-only' || Boolean(company.cloudUid),
          )
          .map((company) => company.slug),
      );
      if (nextCompanies.length > 0) queueDesktopRenderAudit();
    } catch (err) {
      console.error('list_syncable_workspaces failed:', err);
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

  async function loadHomeProjects() {
    try {
      homeProjects = await loadLocalProjects();
    } catch (err) {
      // A missing/locked HQ tree leaves the portfolio table empty rather than
      // breaking Home — the stats simply read 0 / "—".
      console.error('get_local_projects failed:', err);
    }
  }

  async function refreshRealState() {
    await Promise.all([
      loadWorkspaces(),
      loadSyncStatus(),
      loadDaemonStatus(),
      loadActivity(),
      loadHomeProjects(),
    ]);
  }

  async function handleSyncAll() {
    if (syncState === 'syncing') return;
    resetRunState();
    try {
      await invoke('set_tray_state', { state: 'syncing' });
      await invoke('start_sync');
    } catch (err) {
      console.error('start_sync failed:', err);
      syncState = 'error';
      syncErrorMessage = String(err);
      await invoke('set_tray_state', { state: 'error' }).catch(() => undefined);
    }
  }

  // ── Home NEEDS YOU actions ─────────────────────────────────────────────────

  async function handleResolveConflict(path: string, strategy: 'keep-local' | 'keep-remote') {
    const conflict = homeConflicts.find((entry) => entry.path === path);
    if (!conflict || conflict.status === 'resolving') return;
    homeConflicts = homeConflicts.map((entry) =>
      entry.path === path ? { ...entry, status: 'resolving' as const, error: undefined } : entry,
    );
    try {
      await invoke('resolve_conflict', { path, strategy });
      homeConflicts = homeConflicts.filter((entry) => entry.path !== path);
    } catch (err) {
      homeConflicts = homeConflicts.map((entry) =>
        entry.path === path
          ? { ...entry, status: 'error' as const, error: String(err) }
          : entry,
      );
    }
  }

  function handleCompareConflict(path: string) {
    void invoke('open_in_editor', { path }).catch((err) =>
      console.error('open_in_editor failed:', err),
    );
  }

  // Restore every USER-EDIT drifted core file from the scanned upstream
  // target, then re-run the state check so the card reflects post-restore
  // truth (same target-forwarding rationale as the popover's DriftDetail).
  async function handleRestoreDrift() {
    const report = coreState?.driftReport;
    if (!report || driftRestoring) return;
    driftRestoring = true;
    try {
      for (const entry of report.modified) {
        await invoke('restore_from_upstream', {
          path: entry.path,
          expectedUpstreamSha: entry.gitShaUpstream,
          targetRepo: report.targetRepo,
          targetRef: report.targetRef,
        });
      }
      coreState = await invoke<HomeCoreState | null>('check_core_state');
    } catch (err) {
      console.error('restore_from_upstream failed:', err);
    } finally {
      driftRestoring = false;
    }
  }

  function handleKeepDrift() {
    // Session-local ack — the next scan (`core-state:changed`) re-surfaces it.
    driftDismissed = true;
  }

  function handleViewDrift() {
    const report = coreState?.driftReport;
    if (!report) return;
    void invoke('open_drift_detail', { report }).catch((err) =>
      console.error('open_drift_detail failed:', err),
    );
  }

  // "Sign in again" on the Home error card: a silent token refresh fixes the
  // common expired-session case in place (then retries the sync); if the
  // refresh itself fails the session is truly gone, so open Settings where
  // the account surface lives.
  async function handleSignInAgain() {
    try {
      await invoke('refresh_tokens');
      await handleSyncAll();
    } catch (err) {
      console.error('refresh_tokens failed:', err);
      handleOpenSettings();
    }
  }

  function handleOpenActivityLog() {
    void invoke('open_activity_log').catch((err) =>
      console.error('open_activity_log failed:', err),
    );
  }

  async function handleCancelSync() {
    if (syncState !== 'syncing') return;
    try {
      await invoke('cancel_sync');
    } catch (err) {
      console.error('cancel_sync failed:', err);
    }
  }

  // Secondary-sidebar row selection — the id is the section/tab for the
  // current contextual surface (company / library / settings).
  function handleSecondarySelect(id: string) {
    if (route.kind === 'company') {
      navigate({ kind: 'company', slug: route.slug, tab: id as CompanyTab });
    } else if (route.kind === 'library') {
      navigate({ kind: 'library', tab: id as LibraryTab });
    } else if (route.kind === 'settings') {
      // The Settings page renders all sections in one scroll; the secondary
      // rows are a section index. Setting the tab drives both the active-row
      // highlight and SettingsPage's scroll-into-view (US-013).
      navigate({ kind: 'settings', tab: id as SettingsTab });
    }
  }

  function handleSecondaryFooter() {
    if (secondarySidebar?.surface === 'library') {
      // "Publish a pack" — the Profile tab hosts publishing today.
      navigate({ kind: 'library', tab: 'profile' });
      return;
    }
    // "Company settings" — sync rules, members, roles all live in the HQ web
    // console, so open the company's console page in the system browser rather
    // than the in-app Settings route.
    const slug = activeCompany?.slug;
    if (slug) {
      void openExternal(companyConsoleUrl(slug));
      return;
    }
    handleOpenSettings();
  }

  function handleOpenSettings(tab?: SettingsTab) {
    navigate({ kind: 'settings', tab });
  }

  // ── Agent-handoff actions (the hq-* ACTIONS in the ⌘K palette) ─────────────
  // Each opens a Claude Code session cwd'd into HQ with a prepared prompt for
  // the matching hq-* skill. The desktop is a viewer; the agent does the work —
  // so these hand off rather than re-implement deploy/share/run in the app.
  // Company-scoped verbs target the company currently on screen when there is
  // one (activeCompany), otherwise stay general so the agent can ask.

  type DesktopWorkflow = 'deploy' | 'share' | 'run-worker';

  let actionToast = $state<{ text: string; tone: 'ok' | 'warn' } | null>(null);
  let actionToastTimer: ReturnType<typeof setTimeout> | null = null;

  function flashToast(text: string, tone: 'ok' | 'warn') {
    actionToast = { text, tone };
    if (actionToastTimer !== null) clearTimeout(actionToastTimer);
    actionToastTimer = setTimeout(() => {
      actionToast = null;
      actionToastTimer = null;
    }, 5000);
  }

  function dismissToast() {
    if (actionToastTimer !== null) clearTimeout(actionToastTimer);
    actionToastTimer = null;
    actionToast = null;
  }

  function desktopWorkflowPrompt(kind: DesktopWorkflow): { prompt: string; label: string } {
    const slug = activeCompany?.slug ?? null;
    const forCompany = slug ? ` for ${slug}` : '';
    switch (kind) {
      case 'deploy':
        return {
          label: 'deploy workflow',
          prompt: [
            slug ? `/deploy ${slug}` : '/deploy',
            '',
            `Help me deploy or share a result${forCompany}.`,
            'Confirm the artifact or path, run the HQ deploy workflow, and return the preview/share URL when it is ready.',
          ].join('\n'),
        };
      case 'share':
        return {
          label: 'share workflow',
          prompt: [
            '/hq-share',
            '',
            'Help me securely share a file from my HQ vault.',
            'Ask which path and which recipients, then mint the encrypted single-use share link.',
          ].join('\n'),
        };
      case 'run-worker':
        return {
          label: 'worker run',
          prompt: [
            '/run',
            '',
            `Help me run a worker${forCompany}.`,
            'List the available workers and their skills, then run the one I choose.',
          ].join('\n'),
        };
    }
  }

  async function runDesktopWorkflow(kind: DesktopWorkflow) {
    const { prompt, label } = desktopWorkflowPrompt(kind);
    const result = await openAgentWorkflow(prompt, label);
    flashToast(result.message, result.ok ? 'ok' : 'warn');
  }

  function handleKeydown(event: KeyboardEvent) {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      commandPaletteOpen = true;
      return;
    }

    if (commandPaletteOpen) return;

    const nextRoute = getDesktopHotkeyRoute(event, shellCompanies);
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

    if (renderCompanies.length > 0) queueDesktopRenderAudit();
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
    // Home meta-line + drift-card context. All best-effort: a failure leaves
    // the corresponding line/card off rather than blocking the surface.
    void invoke<string | null>('get_hq_version')
      .then((version) => {
        if (mounted) hqVersion = version;
      })
      .catch(() => undefined);
    void invoke<{ hqFolderPath?: string | null }>('get_config')
      .then((config) => {
        if (mounted) hqFolderPath = config?.hqFolderPath ?? null;
      })
      .catch(() => undefined);
    void invoke<{ realtimeSync?: boolean | null }>('get_settings')
      .then((settings) => {
        if (mounted) autoSyncOn = settings.realtimeSync ?? null;
      })
      .catch(() => undefined);
    void invoke<HomeCoreState | null>('check_core_state')
      .then((state) => {
        if (mounted) coreState = state;
      })
      .catch(() => undefined);
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
        // Legacy aliases stay functional ('sync' → Home); unknown intents are
        // ignored so a stale queue entry can't strand the window.
        const pendingRoute = resolvePendingDesktopRoute(pending);
        if (mounted && pendingRoute) {
          navigate(pendingRoute);
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
        // Don't clobber an attention state set mid-run. 'setup-needed' is added
        // here alongside conflict/error: the runner bails on setup-needed and
        // still fires all-complete, so without this guard the status would snap
        // back to "Idle · all safe" and hide that the account isn't provisioned.
        if (syncState !== 'conflict' && syncState !== 'error' && syncState !== 'setup-needed') {
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
      // Conflict stream → Home's NEEDS YOU queue (dedupe by path; the same
      // conflict can re-emit across fanout retries within one run).
      listen<{ path: string; localHash: string; remoteHash: string; canAutoResolve: boolean }>(
        'sync:conflict',
        (event) => {
          if (homeConflicts.some((entry) => entry.path === event.payload.path)) return;
          homeConflicts = [
            ...homeConflicts,
            {
              path: event.payload.path,
              canAutoResolve: event.payload.canAutoResolve,
              status: 'pending',
              at: Date.now(),
            },
          ];
        },
      ),
      // Background core-state scans (6h cadence + on-demand checks) keep the
      // drift card honest; a fresh scan clears a session-local "Keep edit" ack.
      listen<HomeCoreState | null>('core-state:changed', (event) => {
        coreState = event.payload;
        driftDismissed = false;
      }),
      listen<{ company?: string; path: string; message: string }>('sync:error', async (event) => {
        syncState = 'error';
        syncProgress = null;
        syncErrorMessage = event.payload.message;
        syncErrorCompany = event.payload.company ?? null;
        if (event.payload.company) {
          updateWorkspaceStats(event.payload.company, (stats) => ({
            ...stats,
            errorMessage: event.payload.message,
          }));
        }
        await invoke('set_tray_state', { state: 'error' }).catch(() => undefined);
      }),
      // Brand-new account with no person entity / no companies yet: the runner
      // emits sync:setup-needed and bails. The desktop has a purpose-built,
      // non-alarming "Sync not set up" surface (model.ts + DesktopStatusBar) —
      // surface it instead of letting all-complete fall through to "Idle · all
      // safe", which falsely told the user the account was ready. Not an error
      // tone (idle), matching the classic popover's "this is normal" framing.
      listen('sync:setup-needed', () => {
        syncState = 'setup-needed';
        syncProgress = null;
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
        const nextRoute = resolvePendingDesktopRoute(event.payload);
        if (nextRoute) {
          navigate(nextRoute);
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
  data-workspace-count={renderWorkspaceCount}
  style={`--desktop-titlebar-height: ${V4_CHROME_LAYOUT.titleBarHeightPx}px;`}
>
  <V4TitleBar
    {syncState}
    watchedCount={renderWorkspaceCount}
    {lastSyncLabel}
    syncingCompany={syncProgress?.company ?? null}
    fanoutDone={syncFanoutDoneCount}
    fanoutTotal={syncFanoutTotal}
    errorSummary={titleBarErrorSummary}
    onsync={handleSyncAll}
    oncancel={handleCancelSync}
    onretry={handleSyncAll}
  />

  <div class="desktop-body">
    <V4Sidebar
      {route}
      companies={renderCompanies}
      onnavigate={(next) => navigate(fromV4Route(next))}
    />

    {#if secondarySidebar}
      <V4SecondarySidebar
        header={secondarySidebar.header}
        headerTone={secondarySidebar.headerTone}
        meta={secondarySidebar.meta}
        items={secondarySidebar.items}
        activeId={secondarySidebar.activeId}
        footer={secondarySidebar.footer}
        onselect={handleSecondarySelect}
        onfooterselect={handleSecondaryFooter}
      />
    {/if}

    <div class="desktop-content">
      <main class="desktop-main" aria-label="Desktop content">
        <div class="desktop-main-scroll">
        {#key routeKey}
          {#if route.kind === 'home'}
            <div class="page">
              <HomePage
                {syncState}
                {ready}
                {workspaces}
                progress={syncProgress}
                companies={syncCompanies}
                {statsBySlug}
                {status}
                {daemon}
                {activity}
                {syncErrorMessage}
                {syncErrorCompany}
                {syncFilesProgressed}
                syncTotalFiles={effectiveTotalFiles}
                transferredBytes={syncTransferredBytes}
                {syncStartedAt}
                {autoSyncOn}
                {hqVersion}
                conflicts={homeConflicts}
                {coreState}
                {driftDismissed}
                {driftRestoring}
                projects={homeProjects}
                {meetingEvents}
                companyNamesByUid={meetingCompanyNamesByUid}
                onopencompany={(slug) => navigate({ kind: 'company', slug })}
                onresolveconflict={handleResolveConflict}
                oncompareconflict={handleCompareConflict}
                onrestoredrift={handleRestoreDrift}
                onkeepdrift={handleKeepDrift}
                onviewdrift={handleViewDrift}
                onsignin={handleSignInAgain}
                onretry={handleSyncAll}
                onopenlog={handleOpenActivityLog}
              />
            </div>
          {:else if route.kind === 'companies'}
            <div class="page">
              <CompaniesPage
                {workspaces}
                {ready}
                {autoSyncOn}
                {workspaceError}
                {cloudReachable}
                onopencompany={(slug) => navigate({ kind: 'company', slug })}
                onrefresh={() => void loadWorkspaces()}
              />
            </div>
          {:else if route.kind === 'meetings'}
            <div class="page">
              <MeetingsPage />
            </div>
          {:else if route.kind === 'library'}
            <div class="page">
              <LibraryPage tab={libraryTab} />
            </div>
          {:else if route.kind === 'settings'}
            <div class="page">
              <SettingsPage activeTab={route.tab ?? 'sync'} />
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
              <CompanyPage
                company={activeCompany}
                tab={companyTab}
                onopenprojects={() =>
                  navigate({ kind: 'company', slug: activeCompany.slug, tab: 'projects' })
                }
              />
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
    workspaceCount={renderWorkspaceCount}
    observedBytes={observedVaultBytes}
    {nextMeetingLabel}
  />

  {#if commandPaletteOpen}
    <CommandPalette commands={commandItems} onclose={() => (commandPaletteOpen = false)} />
  {/if}

  {#if actionToast}
    <div class={`action-toast ${actionToast.tone}`} role="status">
      <span class="toast-dot" aria-hidden="true"></span>
      <span class="toast-text">{actionToast.text}</span>
      <button class="toast-dismiss" type="button" aria-label="Dismiss" onclick={dismissToast}>×</button>
    </div>
  {/if}
</div>

<style>
  /* V4 ground (SPEC section 2): the window + main content background. The V4
     chrome components (title bar / sidebars) paint their own surfaces. */
  .desktop-shell {
    background: var(--v4-ground);
  }

  /* The Messages route hosts the full-bleed MessagesShell rather than the
     padded, scrolling .page layout — it fills the content area and anchors the
     shell's absolutely-positioned host. */
  .messages-host {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
  }

  /* Transient confirmation for the hq-* palette actions (Deploy / Share / Run a
     worker). Floats above the status bar; status carried by a 6px dot per the
     V4 convention (green = opened in Claude Code, amber = prompt copied as a
     fallback). Auto-dismisses; the × dismisses early. */
  .action-toast {
    position: fixed;
    right: 16px;
    bottom: 38px;
    z-index: 60;
    display: flex;
    align-items: center;
    gap: 9px;
    max-width: min(420px, calc(100vw - 32px));
    padding: 9px 10px 9px 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.5);
  }

  .toast-dot {
    flex: 0 0 auto;
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--v4-idle);
  }

  .action-toast.ok .toast-dot {
    background: var(--v4-ok);
  }

  .action-toast.warn .toast-dot {
    background: var(--v4-warn);
  }

  .toast-text {
    min-width: 0;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    line-height: 17px;
  }

  .toast-dismiss {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--v4-text-3);
    font-size: 15px;
    line-height: 1;
    cursor: pointer;
  }

  .toast-dismiss:hover {
    background: var(--v4-active-row);
    color: var(--v4-text-1);
  }

  @media (prefers-reduced-motion: no-preference) {
    .action-toast {
      animation: action-toast-in 160ms cubic-bezier(0.2, 0.8, 0.2, 1);
    }
  }

  @keyframes action-toast-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
</style>
