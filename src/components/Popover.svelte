<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import SyncStats from './SyncStats.svelte';
  import ConflictModal from './ConflictModal.svelte';
  import WorkspaceList from './WorkspaceList.svelte';
  import CopyPromptButton from './CopyPromptButton.svelte';
  import OpenInClaudeCodeButton from './OpenInClaudeCodeButton.svelte';
  import NewFilesBadge from './NewFilesBadge.svelte';
  import MeetingIcon from './MeetingIcon.svelte';
  import type { Workspace } from '../lib/workspaces';
  import { liveProgressCaption } from '../lib/live-progress-caption';
  import { isCorePath, CORE_SETUP_LABEL } from '../lib/progressLabel';
  import type { ConflictFile } from '../stores/conflicts';

  interface Config {
    configured: boolean;
    companySlug: string;
    hqFolderPath: string;
    error?: string;
  }

  interface Props {
    syncState: 'idle' | 'syncing' | 'error' | 'conflict' | 'setup-needed' | 'auth-error';
    config: Config | null;
    progress?: { company: string; path: string; bytes: number } | null;
    fanoutTotal?: number;
    fanoutDoneCount?: number;
    /** Cumulative count of files touched in the current run (incremented per
     *  sync:progress event). Surfaces as "234 files synced" when no upfront
     *  total is known (runner phase). */
    syncFilesProgressed?: number;
    /** Personal first-push knows files_total upfront — when populated, the
     *  count line shows "234 of 1,247 files" instead of just "234 files
     *  synced". Null/0 outside the personal phase. */
    personalFilesDone?: number;
    personalFilesTotal?: number | null;
    /** Latched true once the in-process Rust personal first-push completes,
     *  reset on the next Sync click. The unified bar uses this to keep
     *  the personal slot at 100% during the gap between Rust complete and
     *  the runner emitting its first event — without it, the bar would
     *  drop back to 0 in that window. */
    personalFirstPushDone?: boolean;
    /** Effective denominator for the unified progress *bar*. Sourced from
     *  App.svelte's `effectiveTotalFiles` derived value (plan-event total
     *  when available, else Rust pre-walk total). The bar uses this for
     *  fill animation; the "N of M transferred" caption does NOT — it uses
     *  `syncPlanTotalFiles` (strict transfer count) instead, to avoid
     *  showing the tree-walk total as if it were a transfer count. */
    syncTotalFiles?: number;
    /** Strict transfer count for the entire sync — sum of
     *  `filesToDownload + filesToUpload + filesToConflict` across every
     *  per-company `sync:plan` event the runner emits (hq-cloud@5.5.0+).
     *  Used by the "N of M transferred" caption so M reflects work the
     *  sync is actually doing, not the size of the local tree. 0 means
     *  no plan events have landed yet (either runner is pre-5.5.0 or
     *  we're still in the Rust pre-walk phase) — caption falls through
     *  to the count-only branch. */
    syncPlanTotalFiles?: number;
    /** Companies in the current/last fanout — rendered live during sync.
     *  `name` is optional; runners < v5.1.9 only emit `uid` + `slug`. The
     *  steady-state list is rendered by `workspaces` below; this prop only
     *  drives the in-flight progress display. */
    companies?: Array<{ uid: string; slug: string; name?: string }>;
    /** Union of cloud entities + local company folders, produced by the Rust
     *  `list_syncable_workspaces` command. The menubar's source of truth for
     *  the steady-state view (replaces the legacy "No companies yet"
     *  dead-end). When `null`, the command hasn't completed yet — render
     *  nothing (App.svelte fires it on mount + after every sync). */
    workspaces?: Workspace[] | null;
    /** Whether `list_syncable_workspaces` could reach the vault. False means
     *  we still rendered local-only data; the UI shows a soft notice. */
    cloudReachable?: boolean;
    /** Error string surfaced when `cloudReachable` is false. */
    cloudError?: string | null;
    /** Top-level manifest parse error from list_syncable_workspaces. Non-null
     *  = soft warning rendered above the workspace list (workspaces fell back
     *  to folder-enumerated discovery). */
    manifestError?: string | null;
    /** Re-fetch workspaces — called by WorkspaceList after a successful
     *  Connect, and from any other code path that mutates workspace state. */
    onworkspacesrefresh?: () => void;
    lastSummary?: {
      companiesAttempted: number;
      filesDownloaded: number;
      bytesDownloaded: number;
      filesSkipped: number;
    } | null;
    errorMessage?: string;
    /** Company slug attached to the last `sync:error` event. Threaded into
     *  the `sync-failed` Copy-Prompt so it can render the per-slug journal
     *  path. Empty string when the failure isn't company-scoped (auth,
     *  discovery-phase, local catch-block). */
    errorCompany?: string;
    /** Number of new files detected in the last sync run (accumulated from
     *  one or more `sync:new-files` events). 0 = no badge shown. */
    newFilesCount?: number;
    /** Flat list of new files across all companies — passed through to the
     *  NewFilesBadge component for future detail-view use (US-006). */
    newFilesList?: Array<{ path: string; bytes: number; addedBy: string | null }>;
    conflicts?: ConflictFile[];
    showConflictModal?: boolean;
    /** Non-null when the Tauri updater has found a newer release. */
    updateAvailable?: { version: string; body?: string; date?: string } | null;
    /** True while `install_update` is in flight — disables the button. */
    updateInstalling?: boolean;
    /** Non-null when the globally-installed `hq` CLI is behind npm `latest`. */
    hqCliUpdateAvailable?: { local: string | null; latest: string } | null;
    /** True while the in-app `npm install -g` is running — disables the
     *  banner button and flips its label to "Installing…". */
    hqCliUpdateInstalling?: boolean;
    /** Last error returned from `install_hq_cli_update`. When set, the
     *  banner body reads "Update failed" and the CTA flips to "Fix this
     *  in HQ", which opens a Claude Code session at the HQ folder with
     *  the captured stderr pre-filled (typical case: EACCES on a
     *  system-prefix npm that needs sudo). */
    hqCliUpdateError?: string | null;
    /** Locally-detected hq-core `hqVersion` (cheap on-disk read from
     *  `core.yaml`). Drives the "HQ vX.Y.Z" footer row, independent of
     *  the unified state check below. Null → render the row with a
     *  "version unknown" label + CopyPromptButton so a broken install
     *  becomes visible rather than silently hidden. */
    hqVersion?: string | null;
    /** Unified HQ-core state. Replaces the pre-refactor quad
     *  (hqCoreUpdateAvailable + hqCoreDrift + stagingDrift +
     *  stagingReplace). Drives both the state badge ("N drifted" / "in
     *  sync") and the action pill ("Update to v…" / "Update to Staging")
     *  on the HQ-version footer row.
     *
     *  Pill visibility logic:
     *    state badge → render when `coreState != null` (showing either
     *                  "N drifted" or "in sync" depending on drift count)
     *    action pill → render when `coreState.versionBehind ||
     *                  coreState.driftReport.count > 0`
     *
     *  Null = checker hasn't run yet (≠ "in sync"). The bg checker fires
     *  30s after launch, so a freshly opened popover may briefly show no
     *  pills. */
    coreState?: {
      channel: 'release' | 'staging';
      targetRepo: string;
      targetVersion: string;
      targetRef: string;
      localVersion: string | null;
      floorSha: string | null;
      isEligible: boolean;
      versionBehind: boolean;
      driftReport: {
        count: number;
        modified: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
        missing: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
        added: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
        scannedAt: string;
        hqVersion: string;
        targetRepo: string;
        targetRef: string;
      };
      unchangedCount: number;
      userOnlyCount: number;
      scannedAt: string;
    } | null;
    /** True while the unified "Update" rescue script is running (either
     *  release `install_hq_core_update` or staging `run_replace_from_staging`;
     *  App.svelte dispatches based on `coreState.channel`). Disables the
     *  pill + swaps its label to "Updating…". */
    coreInstalling?: boolean;
    /** Last rescue-run result. Surfaced next to the pill so the user gets
     *  immediate feedback (✓ done / ✗ failed) without opening the log file.
     *  Cleared at start of a new run. */
    coreInstallLastResult?: {
      kind: 'ok' | 'err';
      exitCode: number;
      logTail: string;
      logPath: string;
    } | null;
    onsync: () => void;
    /** Cancel the in-flight sync (kills the runner subprocess). The same
     *  header button doubles as Sync/Stop — only meaningful when
     *  syncState === 'syncing'. */
    oncancel?: () => void;
    onsettings: () => void;
    onsignout: () => void;
    onresolve?: (path: string, strategy: 'keep-local' | 'keep-remote') => void;
    onopen?: (path: string) => void;
    ondismissconflicts?: () => void;
    oninstallupdate?: () => void;
    /** Run `npm install -g @indigoai-us/hq-cli@latest` via the Rust
     *  backend. App.svelte owns the in-flight + error state. */
    oninstallhqcliupdate?: () => void;
    /** Click handler for the unified Update pill. App.svelte dispatches
     *  to either `install_hq_core_update` (release) or
     *  `run_replace_from_staging` (staging) based on `coreState.channel`.
     *  Optional so the prop is omittable; the pill stays interactive but
     *  no-ops without a handler. */
    oninstallcore?: () => void;
    // Parent can call the returned fn to refresh SyncStats (bound to
    // the child's exported refresh()). We pass a setter down rather
    // than using bind:this because App.svelte holds the ref.
    bindStatsRefresh?: (fn: () => void) => void;
    /** Whether the discreet meeting-invite icon should render in the header.
     *  Driven by `meetings_feature_enabled` (currently @getindigo.ai). */
    meetingsEnabled?: boolean;
    /** Click handler for the meeting icon — toggles the modal open state
     *  in App.svelte (where the modal itself is rendered). */
    onmeetingsclick?: () => void;
    /** Active meeting detections from the Recall Desktop SDK. Rendered as
     *  rows above the sync section with Record/Stop controls. */
    activeMeetings?: ActiveMeeting[];
    /** Triggers `start_recording(windowId)` on the Rust side. Owner is
     *  App.svelte, which also flips the row state on the response. */
    onstartrecording?: (windowId: string) => void | Promise<void>;
    /** Triggers `stop_recording(windowId)` on the Rust side. */
    onstoprecording?: (windowId: string) => void | Promise<void>;
    /**
     * Company memberships the user can attribute new recordings to.
     * Empty when the user is Personal-only or memberships are still
     * loading — the row still renders with "Personal" as the only
     * option in that case.
     */
    recordingCompanies?: Array<{
      companyUid: string;
      companyName: string | null;
      role: string | null;
      status: string;
    }>;
    /**
     * Fires when the user picks a different company in the row's
     * dropdown (pre-recording only). `companyUid = null` means
     * Personal. The Rust upload-token mint reads the snapshot at
     * `start_recording` time, so changes made post-recording are
     * frontend-only.
     */
    onchangerecordingcompany?: (
      windowId: string,
      companyUid: string | null,
    ) => void;
    /** Indigo-only dogfood gate for the desktop alternate window toggle. */
    desktopAltEnabled?: boolean;
  }

  /** Mirror of `App.svelte`'s `ActiveMeeting` interface — duplicated here
   *  so we don't pull a runtime import from a parent. */
  interface ActiveMeeting {
    windowId: string;
    platform: string;
    meetingUrl: string;
    detectedAt: string;
    state: 'detected' | 'starting' | 'recording' | 'stopping' | 'error';
    recordingId?: string;
    error?: string;
    /** Company UID to attribute the recording to. `null` = Personal. */
    companyUid: string | null;
  }

  let {
    syncState,
    config,
    progress = null,
    fanoutTotal = 0,
    fanoutDoneCount = 0,
    syncFilesProgressed = 0,
    personalFilesDone = 0,
    personalFilesTotal = null,
    personalFirstPushDone = false,
    syncTotalFiles = 0,
    syncPlanTotalFiles = 0,
    companies = [],
    workspaces = null,
    cloudReachable = true,
    cloudError = null,
    manifestError = null,
    onworkspacesrefresh,
    lastSummary = null,
    errorMessage = '',
    errorCompany = '',
    newFilesCount = 0,
    newFilesList = [],
    conflicts = [],
    showConflictModal = false,
    updateAvailable = null,
    updateInstalling = false,
    hqCliUpdateAvailable = null,
    hqCliUpdateInstalling = false,
    hqCliUpdateError = null,
    hqVersion = null,
    coreState = null,
    coreInstalling = false,
    coreInstallLastResult = null,
    onsync,
    oncancel,
    onsettings,
    onsignout,
    onresolve,
    onopen,
    ondismissconflicts,
    oninstallupdate,
    oninstallhqcliupdate,
    oninstallcore,
    bindStatsRefresh,
    meetingsEnabled = false,
    onmeetingsclick,
    activeMeetings = [],
    onstartrecording,
    onstoprecording,
    recordingCompanies = [],
    onchangerecordingcompany,
    desktopAltEnabled = false,
  }: Props = $props();

  let desktopAltError = $state('');
  let desktopAltErrorTimer: ReturnType<typeof setTimeout> | null = null;

  function clearDesktopAltErrorTimer() {
    if (desktopAltErrorTimer) {
      clearTimeout(desktopAltErrorTimer);
      desktopAltErrorTimer = null;
    }
  }

  function showDesktopAltError(message: string) {
    clearDesktopAltErrorTimer();
    desktopAltError = message;
    desktopAltErrorTimer = setTimeout(() => {
      desktopAltError = '';
      desktopAltErrorTimer = null;
    }, 5000);
  }

  async function openDesktopAltWindow() {
    desktopAltError = '';
    clearDesktopAltErrorTimer();

    try {
      await invoke('open_desktop_alt_window');
    } catch (e) {
      console.error('open_desktop_alt_window failed:', e);
      showDesktopAltError('Could not open desktop view.');
    }
  }

  async function openNotificationHistory() {
    try {
      await invoke('open_notification_history');
    } catch (e) {
      console.error('open_notification_history failed:', e);
    }
  }

  $effect(() => {
    return () => {
      clearDesktopAltErrorTimer();
    };
  });

  // Instance ref for SyncStats so parent can trigger refresh
  let statsEl: SyncStats | undefined = $state();
  $effect(() => {
    if (statsEl && bindStatsRefresh) {
      bindStatsRefresh(() => statsEl?.refresh());
    }
  });


  // Human-readable formatters
  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  // Unified progress bar. Two modes:
  //
  //   1. Real per-file progress (preferred). Rust pre-walks every syncable
  //      target before any uploads and emits the total file count via
  //      `sync:totals`. Once `syncTotalFiles > 0`, the bar is just
  //      `syncFilesProgressed / syncTotalFiles` — a true denominator.
  //
  //   2. Phase-weighted fallback. If the pre-walk hasn't fired yet (or
  //      returned 0), split the bar 50/50 between personal first-push
  //      (file-level via personalFilesDone/Total) and the runner fanout
  //      (workspace-level via fanoutDoneCount/Total). Better than a
  //      stuck bar, worse than real per-file progress.
  const barPct = $derived.by(() => {
    if (syncTotalFiles > 0) {
      return Math.min(100, Math.max(0, (syncFilesProgressed / syncTotalFiles) * 100));
    }
    let p = 0;
    if (personalFirstPushDone) {
      p += 0.5;
    } else if (personalFilesTotal != null && personalFilesTotal > 0) {
      p += (personalFilesDone / personalFilesTotal) * 0.5;
    }
    if (fanoutTotal > 0) {
      p += (fanoutDoneCount / fanoutTotal) * 0.5;
    }
    return Math.min(100, Math.max(0, p * 100));
  });

  // Caption decision under the bar. Pure function lives in
  // `../lib/live-progress-caption.ts` so it can be unit-tested without a
  // Svelte component harness — see the regression test that proves the
  // pre-walk total is never shown as "transferred".
  const caption = $derived(
    liveProgressCaption({
      syncFilesProgressed,
      syncPlanTotalFiles,
      syncTotalFiles,
      fanoutTotal,
      fanoutDoneCount,
      personalFilesDone,
      personalFilesTotal,
    })
  );

  // Current workspace label — prefer the fanout slot we're currently
  // working on (companies[fanoutDoneCount]) over progress.company,
  // because progress.company is stale when the runner skips a
  // workspace silently (no per-file progress events fire). During
  // the Rust phase (no fanout yet), fall back to "personal".
  const currentLabel = $derived.by(() => {
    if (fanoutTotal > 0 && fanoutDoneCount < fanoutTotal) {
      const w = companies[fanoutDoneCount];
      if (w) return w.name ?? w.slug;
    }
    if (personalFilesTotal != null || personalFirstPushDone) return 'personal';
    return progress?.company ?? '…';
  });

  // Calm first-sync framing: while the file currently being transferred lives
  // under `core/` (the release-shipped scaffold, identical for everyone and
  // not the user's own content), show one steady "Setting up HQ core files…"
  // line instead of a churn of unfamiliar core paths / company names. Reads as
  // one-time setup rather than "all my stuff is uploading". See
  // `../lib/progressLabel.ts`. The honest file counter (caption) is unchanged.
  const liveWorkspaceLine = $derived.by(() => {
    if (progress && isCorePath(progress.path)) return CORE_SETUP_LABEL;
    return currentLabel === '…' ? 'Preparing sync…' : `Syncing ${currentLabel}`;
  });

  // Performance timing — log mount latency
  $effect(() => {
    const mountTime = performance.now();
    console.log(`[popover] mounted at ${mountTime.toFixed(1)}ms`);
    performance.mark('popover-mounted');
  });

  // hq CLI update banner — the button runs `npm install -g` directly via
  // the Rust backend (see install_hq_cli_update). If that fails (typical
  // case: EACCES against a system-prefix npm that needs sudo) the banner
  // swaps to a "Fix this in HQ" CTA that opens a Claude Code session at
  // the user's HQ folder via the `claude://code/new` deep link. Claude
  // Desktop is registered as the system handler for the scheme; the Rust
  // backend just forwards the URL to macOS `open`. The pre-filled prompt
  // includes the install command + the raw stderr so Claude has enough
  // signal to diagnose the actual permission/network failure.
  const HQ_CLI_UPGRADE_CMD = 'npm install -g @indigoai-us/hq-cli@latest';
  async function fixHqCliUpdateInHq() {
    const prompt = [
      'The hq CLI auto-update failed inside the HQ Sync menubar app.',
      '',
      `Install command: ${HQ_CLI_UPGRADE_CMD}`,
      '',
      'Stderr from the failed run:',
      hqCliUpdateError ?? '(no error captured)',
      '',
      'Please diagnose the root cause (EACCES on a system-prefix npm is the usual suspect) and walk me through fixing it.',
    ].join('\n');

    const params = new URLSearchParams({ q: prompt });
    if (config?.hqFolderPath) params.set('folder', config.hqFolderPath);
    const url = `claude://code/new?${params.toString()}`;

    try {
      await invoke('open_claude_code_link', { url });
    } catch (e) {
      console.error('open_claude_code_link failed:', e);
    }
  }

  // Kick off an hq-core update by opening Claude Code at the user's
  // HQ folder with the `/update-hq` slash command pre-filled in the
  // prompt. Claude Desktop is registered as the system handler for
  // `claude://` on macOS, so the OS routes the URL to a fresh Claude
  // Code session; the renderer just builds the URL and hands it to
  // the existing `open_claude_code_link` Rust command (which validates
  // the scheme and shells out to `open`). Same mechanism as
  // `fixHqCliUpdateInHq` above — different prompt, different intent
  // (here it's the success path, not an error-recovery fallback).
  // Open the drift detail window with the current report. Passed
  // verbatim from the prop so the window receives exactly what the pill
  // count was computed from (no re-fetch, no rate-limit double-spend, no
  // pill-vs-window count drift). The Rust side mirrors `new_files.rs`:
  // managed-state stash → window creation → `drift_window_ready`
  // handshake → emit. Errors are logged but not surfaced — the worst
  // case is a click that does nothing, much better than a Sentry-level
  // exception in the user's face for an opt-in diagnostic surface.
  async function openDriftDetail() {
    // Single source of truth — the report inside coreState. Channel is
    // baked into the report's metadata so the detail window doesn't need
    // to branch on staging vs release.
    const report = coreState?.driftReport;
    if (!report) return;
    try {
      await invoke('open_drift_detail', { report });
    } catch (e) {
      console.error('open_drift_detail failed:', e);
    }
  }

  async function handleQuit() {
    try {
      // Mirror the tray's Quit menu item: terminate the process via the
      // dedicated `quit_app` Rust command. We can't use window.close()
      // here — the menubar-app close handler in main.rs intercepts that
      // and only hides the popover, leaving the process running.
      await invoke('quit_app');
    } catch (e) {
      console.error('Failed to quit:', e);
    }
  }

</script>

<div class="popover">
  <!-- Header -->
  <header class="popover-header" data-tauri-drag-region>
    <!-- Left anchor. The HQ identity badge + workspace name/path were removed;
         a quiet "HQ Sync" wordmark replaces them so the bar has a left edge to
         balance the right-aligned action cluster (without it, the controls and
         Sync pill float marooned over dead space). Monochrome, muted, no badge
         — it reads as a calm window title, not branding. Stays draggable. The
         legacy empty `.header-spacer` is gone; `.header-wordmark` now soaks the
         spare width and right-aligns the cluster. -->
    <div class="header-wordmark" data-tauri-drag-region>
      <svg
        class="header-logo"
        viewBox="0 0 280 161"
        height="15"
        fill="currentColor"
        role="img"
        aria-label="HQ"
        xmlns="http://www.w3.org/2000/svg"
      >
        <path d="M85.7251 3.66162H118.034V154.434H85.7251V89.8176H32.3085V154.434H0V3.66162H32.3085V57.5091H85.7251V3.66162Z" />
        <path d="M257.169 160.035L241.014 144.096C235.343 147.973 229.096 150.988 222.276 153.142C215.527 155.296 208.419 156.373 200.952 156.373C190.757 156.373 181.172 154.363 172.197 150.342C163.223 146.25 155.325 140.65 148.505 133.542C141.684 126.362 136.335 118.07 132.458 108.664C128.581 99.187 126.642 89.0278 126.642 78.1865C126.642 67.417 128.581 57.3296 132.458 47.9242C136.335 38.4471 141.684 30.1187 148.505 22.939C155.325 15.7593 163.223 10.1592 172.197 6.1386C181.172 2.0462 190.757 0 200.952 0C211.219 0 220.84 2.0462 229.814 6.1386C238.789 10.1592 246.686 15.7593 253.507 22.939C260.328 30.1187 265.641 38.4471 269.446 47.9242C273.323 57.3296 275.261 67.417 275.261 78.1865C275.261 86.0123 274.184 93.5151 272.031 100.695C269.948 107.803 267.077 114.444 263.415 120.618L280 137.203L257.169 160.035ZM200.952 124.065C203.896 124.065 206.732 123.741 209.46 123.095C212.26 122.449 214.952 121.552 217.537 120.403L208.491 111.357L231.322 88.5252L239.291 96.4946C240.512 93.6946 241.409 90.7509 241.984 87.6637C242.63 84.5764 242.953 81.4173 242.953 78.1865C242.953 71.8684 241.84 65.9452 239.614 60.4168C237.461 54.8885 234.445 50.0422 230.568 45.878C226.691 41.642 222.204 38.3394 217.106 35.9701C212.08 33.529 206.696 32.3085 200.952 32.3085C195.208 32.3085 189.788 33.529 184.69 35.9701C179.664 38.3394 175.213 41.642 171.336 45.878C167.459 50.0422 164.407 54.8885 162.182 60.4168C160.028 65.9452 158.951 71.8684 158.951 78.1865C158.951 84.5046 160.028 90.4637 162.182 96.0639C164.407 101.592 167.459 106.474 171.336 110.71C175.213 114.875 179.664 118.141 184.69 120.511C189.788 122.88 195.208 124.065 200.952 124.065Z" />
      </svg>
    </div>

    <!-- Right-aligned action cluster. `.header-wordmark` (flex:1) pushes it to
         the edge, so it sits on one line. The three secondary icon buttons
         group together; a wider gap before Sync sets the primary action apart
         by separation rather than by crowding. For a basic user this is just
         the Sync button; Indigo adds the meeting + desktop-view entries (both
         identity-gated). Settings is intentionally NOT here — it lives once,
         in the footer. -->
    <div class="header-actions">
      <!-- Secondary icon controls cluster — visually grouped and set apart from
           the primary Sync pill by the gap on `.header-sync` below. -->
      <div class="header-icon-group">
      <!-- Notification history → opens a window listing past DMs, shares, and
           this session's new files. Always available (not identity-gated). A
           bell glyph reads as "things that pinged me". -->
      <button
        class="header-icon-button notif-history-toggle"
        type="button"
        onclick={openNotificationHistory}
        title="Notification history"
        aria-label="Notification history"
        data-testid="notif-history-toggle"
      >
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
          <path d="M8 1.75a3.5 3.5 0 0 0-3.5 3.5c0 3-1.25 4-1.25 4h9.5s-1.25-1-1.25-4A3.5 3.5 0 0 0 8 1.75Z" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M6.75 12.25a1.25 1.25 0 0 0 2.5 0" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>

      {#if meetingsEnabled && onmeetingsclick}
        <!-- Discreet meeting hook → opens MeetingsWindow. State (detected /
             recording) is carried monochromatically (fill weight), not by a
             stoplight tint, per the app's no-severity-colour system. -->
        <MeetingIcon
          onclick={onmeetingsclick}
          detected={activeMeetings.some(
            (m) => m.state === 'detected' || m.state === 'error',
          )}
          recording={activeMeetings.some(
            (m) =>
              m.state === 'recording' ||
              m.state === 'starting' ||
              m.state === 'stopping',
          )}
        />
      {/if}

      {#if desktopAltEnabled}
        <!-- Open the full desktop window (Indigo dogfood). An "open in window"
             glyph reads as "pop this out into the big app" — no longer confused
             with a settings/config control. -->
        <button
          class="header-icon-button desktop-alt-toggle"
          type="button"
          onclick={openDesktopAltWindow}
          title="Open desktop view"
          aria-label="Open desktop view (Indigo dogfood)"
          data-testid="desktop-alt-toggle"
        >
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
            <path d="M9.5 2.5H13.5V6.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M13.5 2.5L7.5 8.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M11.5 9v2.5A1.5 1.5 0 0 1 10 13H4.5A1.5 1.5 0 0 1 3 11.5V6A1.5 1.5 0 0 1 4.5 4.5H7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
      {/if}
      </div>

      <!-- Sync — the header's primary action, always present. Sits apart from
           the secondary icon group above via the cluster gap, so it reads as
           primary without an outsized fill dominating the bar. -->
      <button
        class="header-sync"
        class:syncing={syncState === 'syncing'}
        class:error={syncState === 'error'}
        disabled={syncState === 'auth-error'}
        onclick={syncState === 'syncing' ? oncancel : onsync}
        title={
          syncState === 'syncing'
            ? 'Click to stop the sync'
            : syncState === 'error'
              ? 'Sync initialized — click to resume, or finish in Claude Code'
              : syncState === 'auth-error'
                ? 'Sign in again to sync'
                : 'Sync'
        }
      >
        {#if syncState === 'syncing'}
          <!-- Stop / square icon — replaces the spinner so the button reads
               clearly as a Stop affordance, not a busy indicator. -->
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
            <rect x="3.5" y="3.5" width="9" height="9" rx="1.5" stroke="currentColor" stroke-width="1.5" fill="currentColor" fill-opacity="0.85" />
          </svg>
        {:else if syncState === 'error'}
          <!-- Retry / alert-circle icon -->
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
            <path d="M1.5 8a6.5 6.5 0 0 1 11.48-4.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M14.5 8A6.5 6.5 0 0 1 3.02 12.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M11 1.5v2.5h2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M5 12h-2.5v2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        {:else}
          <!-- Refresh / sync icon — same as the legacy body SyncButton. -->
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
            <path d="M1.5 8a6.5 6.5 0 0 1 11.48-4.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M14.5 8A6.5 6.5 0 0 1 3.02 12.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M11 1.5v2.5h2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
            <path d="M5 12h-2.5v2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        {/if}
        {#if syncState === 'syncing'}
          Stop
        {:else if syncState === 'error'}
          Resume
        {:else if syncState === 'auth-error'}
          Sign in
        {:else}
          Sync
        {/if}
      </button>
    </div>
  </header>

  {#if desktopAltError}
    <p class="header-inline-error" role="status">{desktopAltError}</p>
  {/if}

  <div class="popover-divider"></div>

  <!-- Active meeting detections used to render here as a Detected/Record
       row above the sync list. They were moved to the top of
       MeetingsWindow so the popover stays focused on sync state — the
       calendar icon in the header now tints yellow (detected) / red
       (recording) and clicking it opens the meetings window where the
       same controls live. The `activeMeetings` / `onstartrecording` /
       `onstoprecording` / `onchangerecordingcompany` props are still
       received by this component (they drive the MeetingIcon tint) but
       no longer rendered inline. -->

  <!-- Body -->
  <section class="popover-body">
    {#if showConflictModal && conflicts.length > 0 && onresolve && onopen && ondismissconflicts}
      <ConflictModal
        {conflicts}
        onresolve={onresolve}
        onopen={onopen}
        ondismiss={ondismissconflicts}
      />
    {:else}
      <!-- Update banner — rendered above sync banners since the user should
           always see "there's a new version" regardless of sync state. The
           button calls `install_update` via the parent; backend re-runs
           updater.check() because the Update type isn't Clone. -->
      {#if updateAvailable}
        <div class="banner banner-info banner-update">
          <div class="banner-update-text">
            <p class="banner-title">Update available: v{updateAvailable.version}</p>
            {#if updateAvailable.body}
              <p class="banner-body">{updateAvailable.body}</p>
            {/if}
          </div>
          <div class="banner-actions">
            <button
              class="banner-update-button"
              onclick={oninstallupdate}
              disabled={updateInstalling || !oninstallupdate}
            >
              {updateInstalling ? 'Installing…' : 'Install'}
            </button>
            <!-- No Copy prompt here: the Install button calls Tauri's
                 `install_update` + restart, which is self-sufficient.
                 Unlike conflict/sign-in/hq-cli-update banners, there's
                 no failure mode where handing a prompt to Claude is
                 the next step — the auto-updater either works or the
                 user grabs the DMG from GitHub. Keep this banner quiet. -->
          </div>
        </div>
      {/if}

      <!-- hq CLI update banner — separate from the app updater. The
           Update button shells out to `npm install -g @indigoai-us/hq-cli@latest`
           in the Rust backend. If that errors (most commonly EACCES on a
           system-prefix npm), the banner shows "Update failed" and the
           CTA flips to "Fix this in HQ", which opens a Claude Code
           session at the HQ folder via the `claude://code/new` deep
           link (see fixHqCliUpdateInHq). -->

      {#if hqCliUpdateAvailable}
        <div class="banner banner-info banner-update">
          <div class="banner-update-text">
            <p class="banner-title">
              hq CLI update available: v{hqCliUpdateAvailable.latest}
            </p>
            {#if hqCliUpdateError}
              <p class="banner-body">Update failed</p>
            {:else if hqCliUpdateAvailable.local}
              <p class="banner-body">
                You're on v{hqCliUpdateAvailable.local}. Click Update to install the latest version.
              </p>
            {:else}
              <p class="banner-body">
                Click Update to install the latest version.
              </p>
            {/if}
          </div>
          <div class="banner-actions">
            {#if hqCliUpdateError}
              <button
                class="banner-update-button"
                onclick={fixHqCliUpdateInHq}
              >
                Fix this in HQ
              </button>
            {:else}
              <button
                class="banner-update-button"
                onclick={oninstallhqcliupdate}
                disabled={hqCliUpdateInstalling || !oninstallhqcliupdate}
              >
                {hqCliUpdateInstalling ? 'Installing…' : 'Update'}
              </button>
            {/if}
          </div>
        </div>
      {/if}

      <!-- hq-core "Update available" used to live here as a top-of-popover
           banner. Moved to the footer HQ-version row in v0.1.85 (right-side
           pill next to the version string it's about) — see footer below.
           Less visual noise at the top, action sits next to its context. -->

      <!-- Runner state banners — auth and runtime errors only. The previous
           `setup-needed` "No companies yet" dead-end is gone: the WorkspaceList
           below ALWAYS renders the Personal row, so the menubar is never empty
           even for a fresh sign-in. The legacy onboarding.indigo-hq.com link
           lived here too — replaced by the live "Create a company" / "Join
           via invite" affordances inside WorkspaceList. -->
      {#if syncState === 'auth-error'}
        <div class="banner banner-notice">
          <div class="banner-update-text">
            <p class="banner-title">Session expired</p>
            <p class="banner-body">{errorMessage || 'Please sign in again to continue syncing.'}</p>
          </div>
          <CopyPromptButton
            variant="inline"
            label="Copy prompt"
            issue={{ kind: 'auth-expired', payload: { message: errorMessage } }}
          />
        </div>
      {:else if syncState === 'error' && errorMessage}
        <!-- "Sync needs attention" framing — we deliberately avoid "failed"
             wording and any red treatment on the recoverable-error path.
             The raw `errorMessage` is no longer rendered in the banner body
             because it always sounds alarming ("failed to push", "exit 1");
             the OpenInClaudeCodeButton template (lib/copy-prompts.ts
             'sync-failed') still includes it in the prefilled prompt, so
             Claude Code sees the full stderr the moment the user clicks. -->
        <div class="banner banner-notice">
          <div class="banner-update-text">
            <p class="banner-title">Sync initialized</p>
            <p class="banner-body">Click the button to finish sync in Claude Code.</p>
          </div>
          <div class="banner-actions">
            <OpenInClaudeCodeButton
              variant="inline"
              label="Finish sync in Claude Code"
              folder={config?.hqFolderPath ?? ''}
              issue={{ kind: 'sync-failed', payload: { message: errorMessage, company: errorCompany } }}
            />
            <CopyPromptButton
              variant="inline"
              label="Copy prompt"
              issue={{ kind: 'sync-failed', payload: { message: errorMessage, company: errorCompany } }}
            />
          </div>
        </div>
      {/if}

      <!-- Top stats slot: while syncing, the SyncStats card is replaced
           by a same-shaped live-progress card. When idle, SyncStats shows
           "Last synced X ago" as before. -->
      {#if syncState === 'syncing'}
        <!-- Live progress card — single unified bar that fills 0→100%
             monotonically across the entire sync. The bar value comes
             from `barPct` (50% personal phase + 50% runner phase). The
             label comes from `currentLabel` which reads ahead in the
             fanout plan rather than trailing per-file events, so the
             label stays correct even when a workspace skips silently. -->
        <div class="live-progress">
          <p class="live-line live-workspace">
            {liveWorkspaceLine}
          </p>
          <div class="live-bar">
            <div class="live-bar-fill" style="width: {barPct}%"></div>
          </div>
          <!-- Caption decision lives in `liveProgressCaption` so the
               "do not show the tree-walk total as transferred" rule is
               unit-tested. See `../lib/live-progress-caption.ts`. -->
          {#if caption.kind === 'transferred-of'}
            <p class="live-line muted">
              {caption.progressed.toLocaleString()} of
              {caption.planTotal.toLocaleString()} transferred
            </p>
          {:else if caption.kind === 'transferred'}
            <p class="live-line muted">
              {caption.progressed.toLocaleString()} transferred
            </p>
          {:else if caption.kind === 'up-to-date'}
            <p class="live-line muted">Up to date — finalizing…</p>
          {:else if caption.kind === 'fanout'}
            <p class="live-line muted">
              Workspace {caption.current} of {caption.total}
              {#if caption.progressed > 0}
                · {caption.progressed.toLocaleString()} file{caption.progressed === 1 ? '' : 's'}
              {/if}
            </p>
          {:else if caption.kind === 'personal'}
            <p class="live-line muted">
              {caption.done} of {caption.total} files
            </p>
          {/if}
        </div>
      {:else}
        <SyncStats bind:this={statsEl} onhistory={() => invoke('open_activity_log')} />
        {#if newFilesCount > 0}
          <NewFilesBadge count={newFilesCount} files={newFilesList} onclick={() => invoke('open_new_files_detail', { files: newFilesList })} />
        {/if}
      {/if}

      <!-- Workspaces (Personal + companies) — the steady-state list.
           Renders as soon as `list_syncable_workspaces` returns; null while
           the first invocation is in flight. -->
      {#if workspaces && workspaces.length > 0}
        <WorkspaceList
          {workspaces}
          {cloudReachable}
          {cloudError}
          {manifestError}
          hqFolderPath={config?.hqFolderPath ?? ''}
          onrefresh={onworkspacesrefresh}
        />
      {/if}

      <!-- Sync button moved to the header (right-aligned). The body no
           longer hosts a full-width sync action — keeps the workspace list
           visible even when it grows long, instead of pushing the button
           out of the popover. -->
    {/if}
  </section>

  <div class="popover-divider"></div>

  <!-- Footer -->
  <footer class="popover-footer">
    <!-- HQ-version row. Always rendered above Settings (sits below the
         divider) so the user always knows which HQ they're synced to.
         Three states:
           1. hqVersion present + hqCoreUpdateAvailable null → "HQ vX.Y.Z" only
           2. hqVersion present + hqCoreUpdateAvailable non-null → version
              text + right-aligned "Update to vX.Y.Z" pill. Click invokes
              `install_hq_core_update` (spawns the rescue script against
              indigoai-us/hq-core at the release tag — same engine the
              staging pill uses). While running the pill is disabled and
              relabelled "Updating…". On success a muted "✓ update done"
              chip appears next to it; on FAILURE we never show a raw
              "exit N" chip — instead a "Update failed — copy fix"
              CopyPromptButton hands the user a guided-`/update-hq` prompt
              for their HQ agent (the usual cause is an install too old for
              the in-app rescue to bridge).
           3. hqVersion null → "HQ version unknown" + right-aligned
              CopyPromptButton so the user can hand a triage prompt to an
              agent in-session (the install is broken in a way we can't
              self-repair from the menubar). -->
    <div class="footer-hq-version">
      <div class="footer-hq-version-label">
        <!-- Stack / layers icon -->
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
          <path d="M8 1.5L1.5 4.5L8 7.5L14.5 4.5L8 1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
          <path d="M1.5 8L8 11L14.5 8" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
          <path d="M1.5 11.5L8 14.5L14.5 11.5" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
        </svg>
        {#if hqVersion}
          <span>HQ v{hqVersion}</span>
        {:else}
          <span>HQ version unknown</span>
        {/if}
      </div>
      {#if hqVersion === null}
        <CopyPromptButton
          variant="compact"
          label="Copy prompt"
          issue={{ kind: 'hq-version-undetectable', payload: { hqFolderPath: config?.hqFolderPath ?? '' } }}
        />
      {:else}
        <!-- `.footer-hq-version-actions` wrapper (from main) makes the
             pill group wrap to a second line on narrow widths instead of
             overlapping the "HQ vX.Y.Z" label. Inner rendering is the
             unified `coreState` block (this branch). -->
        <div class="footer-hq-version-actions">
        <!-- Unified state badge + action pill. Both derive entirely from
             `coreState` (see commands/hq_core_state.rs). Channel selection
             (release vs staging) is a parameter on the Rust side; this
             render block is identical for every user.

             State badge: "N drifted" when user_edit > 0, "in sync"
             otherwise. Always clickable so the detail window is one click
             away even on clean installs.

             Action pill: shown when `versionBehind || hasDrift` — i.e.
             whenever the rescue would do something. Label flips to
             "Update to Staging" / "Update to v…" by channel. -->
        {#if coreState}
          {@const hasDrift = coreState.driftReport.count > 0}
          {@const needsUpdate = coreState.versionBehind || hasDrift}
          {@const updateLabel =
            coreState.channel === 'staging'
              ? coreState.versionBehind
                ? 'Update to Staging'
                : 'Restore Staging'
              : coreState.versionBehind
                ? `Update to v${coreState.targetVersion}`
                : `Restore v${coreState.targetVersion}`}
          <!-- State badge gating:
               * Eligible (@getindigo.ai) — full diagnostic surface. "N
                 drifted" with the real count when there's drift, "in
                 sync" otherwise. Both clickable → opens the per-file
                 detail window.
               * Non-eligible — static "in sync" label only. Drift count
                 is suppressed (no per-file surface offered) and the
                 badge is non-clickable. The Update pill still respects
                 the actual `needsUpdate`, so the user still has an
                 action if drift exists. -->
          {#if coreState.isEligible}
            <button
              class="footer-hq-version-pill {hasDrift ? 'footer-hq-version-pill-notice' : ''}"
              onclick={openDriftDetail}
              aria-label={hasDrift
                ? `${coreState.driftReport.count} drifted core file${coreState.driftReport.count === 1 ? '' : 's'}. Click for details.`
                : 'Locked core in sync. Click for details.'}
              title={hasDrift
                ? `${coreState.driftReport.count} locked core file${coreState.driftReport.count === 1 ? '' : 's'} edited since last sync vs ${coreState.targetRepo}@${coreState.targetVersion}. Click for details.`
                : `Locked core matches ${coreState.targetRepo}@${coreState.targetVersion}. Click to open the drift detail window.`}
            >
              {hasDrift
                ? `${coreState.driftReport.count} drifted`
                : 'in sync'}
            </button>
          {:else}
            <span
              class="footer-hq-version-pill footer-hq-version-pill-static"
              title={`Locked core matches ${coreState.targetRepo}@${coreState.targetVersion}.`}
            >
              in sync
            </span>
          {/if}
          {#if needsUpdate && oninstallcore}
            <!-- {#key coreInstalling}: remount the button when in-flight
                 flips so WebKit's GPU compositor can't leave residue from the
                 wider "Restore v14.2.1" / "Update to v14.2.1" label after it
                 shrinks to "Updating…". The popover runs in a transparent
                 NSWindow (decorations:false, transparent:true in tauri.conf),
                 which exposes a known WKWebView quirk where in-place text
                 swaps that change the pill's intrinsic width can leave a
                 ghost rect of the old pill in the compositor cache. -->
            {#key coreInstalling}
              <button
                class="footer-hq-version-pill"
                onclick={oninstallcore}
                disabled={coreInstalling}
                title={coreInstalling
                  ? `Running rescue against ${coreState.targetRepo}@${coreState.targetVersion} — see /tmp/hq-sync-*.log`
                  : `Replace HQ with ${coreState.targetRepo}@${coreState.targetVersion}. Local drifts move to personal/; the upstream tree overlays on top.`}
              >
                {#if coreInstalling}
                  Updating…
                {:else}
                  {updateLabel}
                {/if}
              </button>
            {/key}
          {/if}
        {/if}
        {#if coreInstallLastResult}
          {#if coreInstallLastResult.kind === 'ok'}
            <!-- Success stays a muted inline chip — nothing for the user to
                 act on. -->
            <span
              class="footer-hq-version-result footer-hq-version-result-ok"
              title={coreInstallLastResult.logTail || coreInstallLastResult.logPath}
            >
              ✓ update done
            </span>
          {:else}
            <!-- Failure NEVER surfaces a raw "exit N" chip — an exit code is
                 not something the user can act on. Instead hand them a
                 one-click prompt for their HQ agent to run a guided
                 `/update-hq` (the usual cause is an install too old for the
                 in-app rescue to bridge). The payload carries the exit code +
                 log tail + target so the agent can triage without guessing. -->
            <CopyPromptButton
              variant="inline"
              label="Update failed — copy fix"
              issue={{
                kind: 'hq-core-update-failed',
                payload: {
                  exitCode: coreInstallLastResult.exitCode,
                  logTail: coreInstallLastResult.logTail,
                  logPath: coreInstallLastResult.logPath,
                  channel: coreState?.channel ?? '',
                  targetVersion: coreState?.targetVersion ?? '',
                  targetRepo: coreState?.targetRepo ?? '',
                  hqVersion: hqVersion ?? '',
                },
              }}
            />
          {/if}
        {/if}
        </div>
      {/if}
    </div>

    <!-- Primary navigation. "Recent Changes" now lives on the "Last synced"
         row in SyncStats (the history affordance sits with the status it
         describes), so the footer holds just Settings + the demoted
         destructive row. -->
    <button class="footer-action" onclick={onsettings}>
      <!-- Settings gear icon -->
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <circle cx="8" cy="8" r="2.5" stroke="currentColor" stroke-width="1.5" />
        <path d="M8 1v1.5M8 13.5V15M14.5 8H13M3 8H1.5M12.6 3.4l-1.06 1.06M4.46 11.54l-1.06 1.06M12.6 12.6l-1.06-1.06M4.46 4.46L3.4 3.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
      Settings
    </button>

    <!-- Destructive actions — demoted beneath a divider and compacted onto a
         single muted row so Quit / Sign out can't be hit by reflex while
         reaching for Settings. -->
    <div class="footer-divider"></div>

    <div class="footer-destructive">
      <button class="footer-mini" onclick={onsignout}>
        <!-- Log out icon -->
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
          <path d="M6 14H3a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1h3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M10.5 11.5L14 8l-3.5-3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M14 8H6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        Sign out
      </button>

      <button class="footer-mini footer-quit" onclick={handleQuit}>
        <!-- Power icon -->
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
          <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.5" />
          <path d="M8 3v5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        </svg>
        Quit
      </button>
    </div>
  </footer>
</div>

<style>
  .popover {
    display: flex;
    flex-direction: column;
    /* Fill the window exactly. box-sizing:border-box is critical — the
       1px border must be accounted for inside the width, otherwise the
       popover overflows the 320x400 window by 2px in both axes and
       triggers both scrollbars + clips the footer. */
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    background: var(--popover-bg, rgba(18, 18, 20, 0.68));
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    color: var(--popover-text, #e0e0e0);
    overflow: hidden;
    /* Rounded corners — requires tauri window transparent:true +
       decorations:false + macOSPrivateApi:true for the OS to honor
       transparency outside the radius. Native window shadow comes from
       tauri.conf.json `shadow: true`; CSS box-shadow here would be
       clipped at the window edge and is pointless. */
    border-radius: 18px;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    box-shadow: inset 0 1px 0 var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  :global([data-tauri-drag-region] button),
  :global([data-tauri-drag-region] a),
  :global([data-tauri-drag-region] input) {
    -webkit-app-region: no-drag;
  }

  /* Header */
  .popover-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    /* Tightened bar height: 0.5rem vertical (down from 0.625rem) reads as a
       lean title bar now that the identity block is gone. Horizontal inset is
       var(--space-3) (12px) so the header's left/right edges line up with the
       body's surface-card insets below for one cohesive column. */
    padding: var(--space-2) var(--space-3);
  }

  .header-wordmark {
    /* Left anchor + flex:1 spacer in one: the HQ logomark gives the bar a left
       edge so the action cluster isn't marooned, while flex:1 soaks the spare
       width to keep that cluster flush right. Draggable like the bar. */
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    color: var(--popover-text-heading, #ffffff);
  }

  .header-logo {
    /* Monochrome mark — inherits currentColor so it flips with light/dark.
       Height-locked, width auto-derived from the viewBox so the H+Q proportion
       holds. Slightly held back from full contrast to stay calm in the bar. */
    flex-shrink: 0;
    height: 15px;
    width: auto;
    opacity: 0.9;
  }

  .header-actions {
    display: inline-flex;
    align-items: center;
    /* Wider gap between the secondary icon group and the primary Sync pill so
       Sync reads as primary by separation, not by an outsized fill. */
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .header-icon-group {
    display: inline-flex;
    align-items: center;
    /* Tight inner gap keeps the three secondary controls reading as one
       grouped unit, distinct from the Sync pill set apart to their right. */
    gap: var(--space-1);
    flex-shrink: 0;
  }

  .header-icon-button {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    /* 28px square — matched to the Sync pill height so the whole cluster sits
       on one optical baseline. Slightly leaner than the prior 32px so the
       secondary controls don't compete with the primary action. */
    width: 1.75rem;
    height: 1.75rem;
    padding: 0;
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.52));
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transform: translateY(0);
    transition:
      background-color 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease,
      box-shadow 0.12s ease,
      transform 0.12s ease;
    -webkit-app-region: no-drag;
  }

  .header-icon-button:hover {
    color: var(--popover-text-heading, #ffffff);
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
    transform: translateY(-1px);
    box-shadow: inset 0 1px 0 var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .header-icon-button:active {
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    transform: translateY(0);
    transition-duration: 0.08s;
  }

  .header-icon-button:focus-visible {
    outline: 2px solid var(--popover-highlight, rgba(255, 255, 255, 0.34));
    outline-offset: 2px;
    color: var(--popover-text-heading, #ffffff);
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .header-inline-error {
    margin: -0.1875rem 1rem 0.5rem 1rem;
    color: var(--popover-notice-strong, #ffffff);
    font-size: 0.75rem;
    line-height: 1.35;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Header Sync button — same visual weight as the legacy body SyncButton
     (icon + pill, popover-primary background) but inline + right-aligned
     instead of full-width. The data-tauri-drag-region on .popover-header
     means clicks-and-holds drag the window; -webkit-app-region: no-drag
     restores click handling for this button. */
  .header-sync {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    /* 28px tall (matches the icon buttons), tighter horizontal padding so the
       primary fill is a compact pill rather than the heaviest mass on screen.
       It still reads as primary — solid `--popover-primary` fill vs. the now
       transparent secondary controls — just not oversized. */
    height: 1.75rem;
    padding: 0 var(--space-3);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--popover-primary-text, #111113);
    background: var(--popover-primary, #ffffff);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background-color 0.15s ease, opacity 0.15s ease, color 0.15s ease;
    -webkit-app-region: no-drag;
  }

  .header-sync:hover:not(:disabled) {
    background: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
  }

  .header-sync:active:not(:disabled) {
    background: var(--popover-primary-active, rgba(255, 255, 255, 0.78));
  }

  .header-sync:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .header-sync.syncing {
    opacity: 0.85;
    cursor: progress;
  }

  /* Error state on the header Sync button: same primary look, with a "Retry"
     icon + label instead of "Sync". No red — calm visual; the icon/text plus
     the notice banner below carry the meaning. */
  .header-sync.error {
    background: var(--popover-primary, #ffffff);
    color: var(--popover-primary-text, #111113);
    border-color: var(--popover-border, rgba(255, 255, 255, 0.18));
  }

  .header-sync-spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(17, 17, 19, 0.25);
    border-top-color: var(--popover-primary-text, #111113);
    border-radius: 50%;
    animation: header-sync-spin 0.6s linear infinite;
  }

  @keyframes header-sync-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .header-icon-button {
      transition:
        background-color 0.12s ease,
        color 0.12s ease,
        border-color 0.12s ease,
        box-shadow 0.12s ease;
    }

    .header-icon-button,
    .header-icon-button:hover,
    .header-icon-button:active {
      transform: none;
    }
  }

  /* Divider — inset to var(--space-3) (12px) so its ends line up with the
     header, body, and footer horizontal insets: one continuous column edge. */
  .popover-divider {
    height: 1px;
    background: var(--popover-divider, rgba(255, 255, 255, 0.06));
    margin: 0 var(--space-3);
  }

  /* Body */
  .popover-body {
    /* var(--space-3) (12px) horizontal inset aligns the body's left/right edges
       with the header, divider, and footer for one cohesive column. A touch
       more top padding (var(--space-3)) than bottom sets the first card off the
       divider with deliberate breathing room; the inter-card gap is var(--space-2). */
    padding: var(--space-3) var(--space-3) var(--space-2);
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow-y: auto;
    /* Firefox scrollbar styling */
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
    /* min-height:0 is required on flex children so overflow-y:auto
       actually constrains height. Without it, the body expands to fit
       content and the scrollbar never engages (content pushes past
       window bounds instead). */
    min-height: 0;
  }

  /* WebKit scrollbar — thin, subtle, only visible on hover/scroll */
  .popover-body::-webkit-scrollbar {
    width: 4px;
  }
  .popover-body::-webkit-scrollbar-track {
    background: transparent;
  }
  .popover-body::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.08);
    border-radius: 2px;
  }
  .popover-body:hover::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.18);
  }


  /* Footer — a calm grouped base. Horizontal inset is var(--space-3) (12px) so
     it lines up with the body, divider, and header; a small inner row gap gives
     the three tiers (version → Settings nav → demoted destructive row) a
     deliberate vertical rhythm rather than crammed-flush stacking. The interior
     buttons carry an extra -4px horizontal margin so their hover fills bleed to
     the column edge while their text stays optically aligned at 12px. */
  .popover-footer {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3) var(--space-3);
  }

  .footer-action {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    /* -4px horizontal margin lets the hover fill bleed toward the column edge
       while the icon+label still sit at ~16px from the window edge (footer
       inset 12px - 4px margin + 8px padding), optically aligned with the
       version row's icon above it. */
    width: calc(100% + var(--space-2));
    margin: 0 calc(-1 * var(--space-1));
    padding: 0.4375rem var(--space-2);
    font-size: var(--text-base);
    font-family: inherit;
    color: var(--popover-text-muted, #a0a0b0);
    background: none;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease;
    text-align: left;
  }

  .footer-action:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
    color: var(--popover-text, #e0e0e0);
  }

  .footer-quit:hover {
    color: var(--popover-danger, #ef4444);
  }

  /* Hairline separating primary nav from the demoted destructive row.
     Inset to match the footer's horizontal padding rhythm. */
  .footer-divider {
    height: 1px;
    background: var(--popover-divider, rgba(255, 255, 255, 0.06));
    margin: var(--space-1) var(--space-2);
  }

  /* Destructive actions share one compact, muted row. Each button is
     center-aligned and lighter than a nav row, so the pair reads as
     secondary and sits clearly apart from Settings above the divider. The
     matching -4px horizontal margin keeps its hover fills bleeding to the
     same column edge as the Settings row above. */
  .footer-destructive {
    display: flex;
    gap: var(--space-1);
    width: calc(100% + var(--space-2));
    margin: 0 calc(-1 * var(--space-1));
  }

  .footer-mini {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2);
    font-size: var(--text-sm);
    font-family: inherit;
    color: var(--popover-text-muted, #a0a0b0);
    background: none;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease;
  }

  .footer-mini:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
    color: var(--popover-text, #e0e0e0);
  }

  /* Higher specificity than `.footer-mini:hover` so Quit's hover stays
     the danger tone rather than the neutral text color. */
  .footer-mini.footer-quit:hover {
    color: var(--popover-danger, #ef4444);
  }

  /* HQ-version footer row. Same padding rhythm as `.footer-action` so it
     reads as part of the same column, but it's a div (not a button) — the
     row itself isn't clickable; the optional right-aligned pill /
     CopyPromptButton is the affordance. */
  .footer-hq-version {
    display: flex;
    align-items: center;
    /* Wrap the action group to a second line when the version label + pills
       can't share one line, instead of overflowing the 320px popover and
       overlapping the "HQ vX.Y.Z" label. The actions group keeps
       `margin-left:auto` so it stays right-aligned whether it sits beside the
       label or drops below it.

       Unlike `.footer-action` (a text-only row that uses a -4px bleed so its
       hover fill can reach the column edge), this row carries a VISIBLE
       right-aligned pill — so it must NOT bleed past the inset, or the pill
       kisses the window edge. It spans the footer content box exactly: the
       right pill lands at the normal 12px inset (aligned with the body cards),
       and a small left pad keeps the layers icon optically aligned with the
       Settings row's icon above it. */
    flex-wrap: wrap;
    gap: 0.375rem 0.5rem;
    box-sizing: border-box;
    width: 100%;
    margin: 0;
    padding: 0.4375rem 0 0.4375rem var(--space-1);
    font-size: 0.8125rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .footer-hq-version-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    /* Keep "HQ vX.Y.Z" on a single line — without this the label wraps
       ("HQ" / "vX.Y.Z") when the action pills crowd the row, and the
       wrapped text collided with the drifted pill. The pills now live in
       a wrapping `.footer-hq-version-actions` group that drops to a
       second line instead, so the label can stay intact. */
    flex-shrink: 0;
    white-space: nowrap;
  }

  /* Right-side action group (drift count chip + Update/Update-to-Staging
     pill + rescue-result chip). Stays on a single line: the drift badge is
     now a bare count ("14") rather than "N drifted", so the group is narrow
     enough to sit beside the version label without wrapping. */
  .footer-hq-version-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    /* Wrap internally too, so an unusually wide combination (e.g.
       "Update to vX.Y.Z" + the "Update failed — copy fix" prompt button)
       stacks right-aligned rather than overflowing. `margin-left:auto` keeps
       the group pinned right on whichever row it lands. */
    flex-wrap: wrap;
    gap: 0.375rem;
    min-width: 0;
    margin-left: auto;
  }

  /* Right-aligned "Update to vX.Y.Z" pill. Same visual weight as
     `.banner-update-button` (white background, dark text — the popover's
     primary action treatment) but pill-shaped + smaller, since it lives
     inline next to the version label rather than as a banner-level CTA. */
  .footer-hq-version-pill {
    font-size: 0.6875rem;
    font-family: inherit;
    font-weight: 600;
    padding: 0.1875rem 0.5rem;
    background: var(--popover-primary, #ffffff);
    color: var(--popover-primary-text, #111113);
    border: none;
    border-radius: 999px;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    transition: background-color 0.1s ease, opacity 0.1s ease;
  }

  .footer-hq-version-pill:hover {
    background: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
  }

  /* Disabled state — used by the "Update to Staging" pill while the
     rescue script is in flight (multi-minute clone + scan). Keeps the
     button visually present so the user can see what's happening, but
     non-interactive (opacity drop + default cursor + suppress hover). */
  .footer-hq-version-pill:disabled {
    cursor: default;
    opacity: 0.7;
  }

  .footer-hq-version-pill:disabled:hover {
    background: var(--popover-primary, #ffffff);
  }

  /* Notice variant — used by the drift "N drifted" pill so it reads as
     diagnostic rather than action. Sits next to (and visually beneath)
     the primary white Update pill so the eye still lands on the action.
     Calm grey surface — no severity colour, consistent with the rest of
     the menubar's notice-tone language. */
  .footer-hq-version-pill-notice {
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
  }

  .footer-hq-version-pill-notice:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text-heading, #ffffff);
  }

  /* Count variant — the drift badge renders as a bare count ("14") instead
     of "N drifted" so the whole version row stays on one line. Tighter and
     smaller than the base pill: a compact numeric chip beside the white
     Update pill. `tabular-nums` keeps 1- and 2-digit counts from jiggling. */
  .footer-hq-version-pill-count {
    font-size: 0.625rem;
    padding: 0.125rem 0.375rem;
    min-width: 1.25rem;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  /* Static variant — non-clickable "in sync" label shown to non-eligible
     users (no @getindigo.ai email). Drift count is suppressed for them
     and there's no per-file detail surface to open, so the badge renders
     as a plain `<span>` with the same shape as the notice pill but no
     cursor/hover/click affordances. Calm-grey background so it reads as
     informational rather than actionable. */
  .footer-hq-version-pill-static {
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    cursor: default;
  }

  /* Inline result chip rendered next to the pill after a rescue run. Small,
     non-clickable, hover surfaces the log tail via the parent's `title=`. */
  .footer-hq-version-result {
    font-size: 0.6875rem;
    font-family: inherit;
    white-space: nowrap;
    flex-shrink: 0;
    padding: 0.1875rem 0.375rem;
    border-radius: 4px;
  }

  .footer-hq-version-result-ok {
    color: var(--popover-success, #6ad59c);
  }

  /* Banners — actionable state callouts (setup / auth / error) */
  .banner {
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
    /* Tightened from 0.625rem 0.75rem (v0.1.85) — 8px / 10px reads as
       calmer when the body stacks several at once (update + cli + error). */
    padding: 0.5rem 0.625rem;
    border-radius: 10px;
    border: 1px solid transparent;
  }

  .banner-info {
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border-color: var(--popover-border, rgba(255, 255, 255, 0.18));
  }

  /* Notice variant — replaces the legacy banner-error red treatment. Visually
     identical to banner-info; the title + body + Copy-prompt button carry
     the urgency. Column layout (inherited from .banner) so long error
     messages get the full 320px popover width and the action buttons sit
     on their own row below — fixes the v0.1.69 word-per-line wrap. */
  .banner-notice {
    background: var(--popover-notice-bg, rgba(255, 255, 255, 0.05));
    border-color: var(--popover-notice-border, rgba(255, 255, 255, 0.16));
    gap: 0.5rem;
  }

  .banner-title {
    margin: 0;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    line-height: 1.3;
  }

  .banner-body {
    margin: 0;
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.4;
  }

  /* Update banner: column layout. Title + body on top (full width), action
     buttons on the bottom row. Earlier horizontal layout assumed exactly
     one action button — adding the Copy-prompt button alongside Install
     squeezed the text column down to ~76px on the 320px window and made
     each word wrap to its own line (v0.1.69 regression). Column layout
     fixes that without sacrificing visual hierarchy. */
  .banner-update {
    gap: 0.5rem;
  }

  .banner-update-text {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    min-width: 0;
  }

  .banner-update-button {
    font-size: 0.75rem;
    font-family: inherit;
    font-weight: 600;
    padding: 0.3125rem 0.75rem;
    background: var(--popover-primary, #ffffff);
    color: var(--popover-primary-text, #111113);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: background-color 0.1s ease, opacity 0.1s ease;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .banner-update-button:hover:not(:disabled) {
    background: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
  }

  .banner-update-button:disabled {
    opacity: 0.6;
    cursor: default;
  }

  /* Secondary variant of the banner button — used for "Copy command" in the
     hq CLI update-failed state. Same shape, calm grey tone instead of
     primary white, so the primary Update / Install affordance is preserved
     when both buttons sit side-by-side. */
  .banner-update-button-secondary {
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
  }

  .banner-update-button-secondary:hover:not(:disabled) {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text-heading, #ffffff);
  }

  /* Action buttons row — sits beneath banner text in column-stacked banners.
     justify-content: flex-end keeps the buttons hugging the right edge so
     the eye lands on them as the next action. flex-wrap lets the two-button
     variants (Copy command + Copy prompt) drop the secondary onto a new
     line on narrow widths instead of overflowing. */
  .banner-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: 0.375rem;
  }

  /* Live progress — replaces the SyncStats card while actively syncing.
     Padding/radius/background/border + inset highlight match .sync-stats
     exactly so the swap-in feels like a content change, not a layout
     change. width: 100% + box-sizing keep the right edge flush like
     SyncStats does. */
  .live-progress {
    width: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.6rem 0.75rem;
    border-radius: 12px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    box-shadow: inset 0 1px 0 var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .live-line {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.35;
    color: var(--popover-text, #e0e0e0);
    display: flex;
    align-items: center;
    gap: 0.375rem;
    min-width: 0;
  }

  .live-line.muted {
    color: var(--popover-text-muted, #a0a0b0);
    font-size: 0.6875rem;
  }

  /* Workspace label — line 1 of the standardized 3-line live-progress
     card. Prominent so the user can see at a glance which workspace is
     currently syncing. Same visual weight as SyncStats' stat-value. */
  .live-workspace {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  /* Determinate progress bar — fill width set inline from the markup
     (barPct %). The 0.25s ease-out transition smooths the per-file
     ticks during the personal phase and the discrete workspace jumps
     during the runner phase. overflow:hidden on the track guards
     against rounding errors that could push the fill past 100% by a
     sub-pixel. */
  .live-bar {
    width: 100%;
    height: 6px;
    border-radius: 3px;
    background: var(--popover-progress-track, rgba(255, 255, 255, 0.14));
    overflow: hidden;
  }

  .live-bar-fill {
    height: 100%;
    background: var(--popover-progress-fill, #ffffff);
    border-radius: 3px;
    transition: width 0.25s ease-out;
  }

  /* Summary line — "Last sync · X files · Y MB" */
  .summary-line {
    margin: 0;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.4;
  }

  /* The active-meetings row UI (Detected/Record + company-attribution
     select + Stop button) moved to MeetingsWindow.svelte on
     2026-05-30. Its CSS (`.active-meetings` / `.meeting-row` /
     `.meeting-company` / `.recording-dot` / `@keyframes
     recording-pulse` etc) lived here previously; the equivalent
     `.active-*` classes now live alongside the markup in
     MeetingsWindow's <style> block. Removed from this file so
     `vite-plugin-svelte` doesn't flag them as unused. */
</style>
