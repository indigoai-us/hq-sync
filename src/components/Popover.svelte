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
    /** Non-null when the user's local hq-core (read from core.yaml's
     *  `hqVersion`) is behind the latest GitHub release of
     *  indigoai-us/hq-core. When non-null, the footer HQ-version row
     *  surfaces an "Update to vX.Y.Z" pill (right-aligned) whose click
     *  handler launches Claude Code at the HQ folder with `/update-hq`
     *  pre-filled in the prompt — same `claude://code/new` deep-link
     *  mechanism as `fixHqCliUpdateInHq`. Replaces the v0.1.84 top-of-
     *  popover update banner (less visually noisy, lives next to the
     *  version string it's about). */
    hqCoreUpdateAvailable?: {
      local: string | null;
      latest: string;
    } | null;
    /** Locally-detected hq-core `hqVersion` (cheap on-disk read from
     *  `core.yaml`). Drives the "HQ vX.Y.Z" footer row, independent of
     *  the GitHub-release check above. Null → render the row with a
     *  "version unknown" label + CopyPromptButton so a broken install
     *  becomes visible rather than silently hidden. */
    hqVersion?: string | null;
    /** Drift summary from `hq-core-drift:available`. When `count > 0`,
     *  the HQ-version footer row renders an "N drifted" pill to the
     *  right of the version label (alongside the Update pill if both
     *  are active). Clicking opens the drift detail window with this
     *  exact payload — passed verbatim so the window doesn't re-fetch
     *  and risk a count mismatch with the pill. Null → no pill. */
    hqCoreDrift?: {
      count: number;
      modified: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
      missing: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
      added: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
      scannedAt: string;
      hqVersion: string;
    } | null;
    /** Staging-flavored drift summary from `hq-core-staging-drift:available`.
     *  Same shape as `hqCoreDrift` but computed against `hq-core-staging@main`
     *  instead of the released tag. Non-null only for eligible
     *  @getindigo.ai builders (Rust-side gate). When non-null and
     *  `count > 0`, REPLACES the release-drift pill — staging users care
     *  about distance from where they're actually headed (staging),
     *  not from the tag they've already moved past. */
    stagingDrift?: {
      count: number;
      modified: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
      missing: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
      added: Array<{ path: string; size: number; gitShaLocal: string | null; gitShaUpstream: string | null }>;
      scannedAt: string;
      hqVersion: string;
    } | null;
    /** "Update from staging" pill state. Null when the feature is dark
     *  (non-@getindigo.ai user, no GH token, GH API unreachable). Otherwise
     *  the pill renders when `available=true` (local stamp missing or
     *  behind staging main HEAD). */
    stagingReplace?: {
      available: boolean;
      localSha: string | null;
      latestSha: string;
      latestShort: string;
      repo: string;
    } | null;
    /** True while the rescue script is in-flight. Disables the pill +
     *  swaps its label to "Updating…" so the user knows something's
     *  happening during the multi-minute scan. */
    stagingReplaceRunning?: boolean;
    /** Last rescue run's result. Surfaced next to the pill so the user
     *  gets immediate feedback (✓ done / ✗ failed) without opening the
     *  log file. Cleared at start of a new run. */
    stagingReplaceLastResult?: {
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
    /** Invoke the rescue script via the Rust `run_replace_from_staging`
     *  command. App.svelte owns the in-flight + result state. Optional
     *  so the prop is omittable for non-eligible users (the parent simply
     *  doesn't bind it; the pill stays hidden anyway). */
    onrunreplacefromstaging?: () => void;
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
    /** Indigo-only dogfood gate for the desktop alternate window toggle. */
    desktopAltEnabled?: boolean;
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
    hqCoreUpdateAvailable = null,
    hqVersion = null,
    hqCoreDrift = null,
    stagingDrift = null,
    stagingReplace = null,
    stagingReplaceRunning = false,
    stagingReplaceLastResult = null,
    onsync,
    oncancel,
    onsettings,
    onsignout,
    onresolve,
    onopen,
    ondismissconflicts,
    oninstallupdate,
    oninstallhqcliupdate,
    onrunreplacefromstaging,
    bindStatsRefresh,
    meetingsEnabled = false,
    onmeetingsclick,
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
    // Eligible users see the staging-flavored report; everyone else sees
    // the release-tagged report. Same window, same structure — only the
    // upstream reference differs. Both reports use the DriftReport shape
    // so the detail window doesn't need to branch.
    const report = stagingDrift ?? hqCoreDrift;
    if (!report) return;
    try {
      await invoke('open_drift_detail', { report });
    } catch (e) {
      console.error('open_drift_detail failed:', e);
    }
  }

  async function updateHqCoreInClaudeCode() {
    const params = new URLSearchParams({ q: '/update-hq' });
    if (config?.hqFolderPath) params.set('folder', config.hqFolderPath);
    const url = `claude://code/new?${params.toString()}`;

    try {
      await invoke('open_claude_code_link', { url });
    } catch (e) {
      console.error('open_claude_code_link failed:', e);
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

  let companyDisplay = $derived(
    config?.companySlug
      ? config.companySlug.charAt(0).toUpperCase() + config.companySlug.slice(1)
      : 'HQ'
  );

  let folderDisplay = $derived(
    config?.hqFolderPath
      ? config.hqFolderPath.replace(/^\/Users\/[^/]+/, '~')
      : '~/hq'
  );
</script>

<div class="popover">
  <!-- Header -->
  <header class="popover-header" class:has-desktop-alt-controls={desktopAltEnabled} data-tauri-drag-region>
    <div class="header-icon">
      <svg width="22" height="22" viewBox="0 0 48 48" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <rect width="48" height="48" rx="12" fill="currentColor" opacity="0.92" />
        <text x="50%" y="54%" dominant-baseline="middle" text-anchor="middle" fill="var(--popover-primary-text, #111113)" font-family="system-ui, -apple-system, BlinkMacSystemFont, sans-serif" font-weight="700" font-size="20">HQ</text>
      </svg>
    </div>
    <div class="header-text">
      <h1>{companyDisplay}</h1>
      <p class="header-path">{folderDisplay}</p>
    </div>

    {#if meetingsEnabled && onmeetingsclick}
      <!-- Discreet meeting-invite icon, sits just left of Sync. Gated to
           @getindigo.ai via `meetings_feature_enabled` (SYNC-1) so this
           branch is dead code for non-Indigo users. -->
      <MeetingIcon onclick={onmeetingsclick} />
    {/if}

    {#if desktopAltEnabled}
      <div class="header-utility-actions">
        <button
          class="header-icon-button desktop-alt-toggle"
          type="button"
          onclick={openDesktopAltWindow}
          title="Open desktop view"
          aria-label="Open desktop view (Indigo dogfood)"
          data-testid="desktop-alt-toggle"
        >
          <!-- Window/dashboard icon, intentionally distinct from the Settings gear. -->
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
            <rect x="2" y="2.5" width="12" height="11" rx="2" stroke="currentColor" stroke-width="1.5" />
            <path d="M2 6h12" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
            <path d="M6 6v7.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
            <path d="M9 9h2.5M9 11.5h2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          </svg>
        </button>
        <button
          class="header-icon-button header-settings-toggle"
          type="button"
          onclick={onsettings}
          title="Settings"
          aria-label="Settings"
        >
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
            <circle cx="8" cy="8" r="2.5" stroke="currentColor" stroke-width="1.5" />
            <path d="M8 1v1.5M8 13.5V15M14.5 8H13M3 8H1.5M12.6 3.4l-1.06 1.06M4.46 11.54l-1.06 1.06M12.6 12.6l-1.06-1.06M4.46 4.46L3.4 3.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    {/if}

    <!-- Sync button — right-aligned in the header so it's always visible
         regardless of how long the workspaces list grows. Same visual
         weight + icon as the original body button; just labelled "Sync"
         instead of "Sync Now" and not full-width. -->
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
  </header>

  {#if desktopAltError}
    <p class="header-inline-error" role="status">{desktopAltError}</p>
  {/if}

  <div class="popover-divider"></div>

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
            {currentLabel === '…' ? 'Preparing sync…' : `Syncing ${currentLabel}`}
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
        <SyncStats bind:this={statsEl} />
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
              text + right-aligned "Update to vX.Y.Z" pill (clickable, opens
              Claude Code with /update-hq pre-filled).
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
        <div class="footer-hq-version-actions">
        <!-- Drift pill (notice tone, not primary white) — appears first so
             the eye lands on the diagnostic before the action pill. Hidden
             when count is 0 or null so the row stays calm on healthy
             installs. Click → opens the drift detail window. -->
        <!-- Drift pill precedence:
             * Eligible users (stagingDrift !== null) see the staging-vs-local
               count — they care about distance from where they're actually
               headed, not from the released tag they've moved past.
             * Non-eligible users keep the existing release-drift pill.
             Both reports use the same DriftReport shape so the detail
             window doesn't fork. -->
        {#if stagingDrift}
          <!-- Eligible users always see a staging-drift chip — at 0 it
               confirms "in sync", >0 it's the same notice pill as before.
               Either state is clickable so the detail window is always
               one click away (lets you inspect even when everything is
               clean). -->
          <button
            class="footer-hq-version-pill {stagingDrift.count > 0 ? 'footer-hq-version-pill-notice' : ''}"
            onclick={openDriftDetail}
            title={stagingDrift.count > 0
              ? `${stagingDrift.count} locked core file${stagingDrift.count === 1 ? '' : 's'} differ from ${stagingDrift.hqVersion}. Click for details. Click "Update to Staging" to reconcile.`
              : `Locked core matches ${stagingDrift.hqVersion}. Click to open the drift detail window.`}
          >
            {stagingDrift.count > 0 ? `${stagingDrift.count} drifted` : 'in sync'}
          </button>
        {:else if hqCoreDrift && hqCoreDrift.count > 0}
          <button
            class="footer-hq-version-pill footer-hq-version-pill-notice"
            onclick={openDriftDetail}
            title={`${hqCoreDrift.count} locked core file${hqCoreDrift.count === 1 ? '' : 's'} differ from upstream v${hqCoreDrift.hqVersion}. Click for details.`}
          >
            {hqCoreDrift.count} drifted
          </button>
        {/if}
        <!-- Pill precedence for the HQ-version footer row:
             * Eligible @getindigo.ai users (stagingReplace !== null) use
               the staging channel for updates — the "Update to Staging"
               pill REPLACES the release "Update to vX.Y.Z" pill entirely
               for them. They're already ahead of releases by definition,
               so the release pill would be misleading noise.
             * Non-eligible users (stagingReplace === null because the
               Rust check returned None) fall back to the existing
               release-update pill.
             The staging pill reuses the standard primary-white pill
             styling so it reads as a first-class action, not a side
             channel. Disabled while running via the new disabled rule
             on .footer-hq-version-pill. -->
        {#if stagingReplace && onrunreplacefromstaging}
          {#if stagingReplace.available || (stagingDrift && stagingDrift.count > 0)}
            <button
              class="footer-hq-version-pill"
              onclick={onrunreplacefromstaging}
              disabled={stagingReplaceRunning}
              title={stagingReplaceRunning
                ? `Running rescue against ${stagingReplace.repo} — see /tmp/hq-sync-replace-from-staging-*.log`
                : stagingReplace.available
                  ? `Replace HQ with ${stagingReplace.repo}@${stagingReplace.latestShort}. Local drifts move to personal/; staging overlays on top.`
                  : `Re-overlay ${stagingReplace.repo}@${stagingReplace.latestShort} to reconcile ${stagingDrift?.count ?? 0} drifted file${(stagingDrift?.count ?? 0) === 1 ? '' : 's'}. Local drifts move to personal/.`}
            >
              {#if stagingReplaceRunning}
                Updating…
              {:else}
                Update to Staging
              {/if}
            </button>
          {/if}
        {:else if hqCoreUpdateAvailable}
          <button
            class="footer-hq-version-pill"
            onclick={updateHqCoreInClaudeCode}
            title="Open Claude Code with /update-hq pre-filled"
          >
            Update to v{hqCoreUpdateAvailable.latest}
          </button>
        {/if}
        {#if stagingReplaceLastResult}
          <!-- Last rescue-run feedback. Tiny inline chip so the user sees
               success/failure without leaving the popover. Click reveals
               the log-tail tooltip via `title`. -->
          <span
            class="footer-hq-version-result footer-hq-version-result-{stagingReplaceLastResult.kind}"
            title={stagingReplaceLastResult.logTail || stagingReplaceLastResult.logPath}
          >
            {#if stagingReplaceLastResult.kind === 'ok'}
              ✓ rescue done
            {:else}
              ✗ rescue failed (exit {stagingReplaceLastResult.exitCode})
            {/if}
          </span>
        {/if}
        </div>
      {/if}
    </div>

    <button class="footer-action" onclick={() => invoke('open_activity_log')}>
      <!-- Clock / history icon -->
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.5" />
        <path d="M8 4.5V8l2.5 1.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      Recent Changes
    </button>

    <button class="footer-action" onclick={onsettings}>
      <!-- Settings gear icon -->
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <circle cx="8" cy="8" r="2.5" stroke="currentColor" stroke-width="1.5" />
        <path d="M8 1v1.5M8 13.5V15M14.5 8H13M3 8H1.5M12.6 3.4l-1.06 1.06M4.46 11.54l-1.06 1.06M12.6 12.6l-1.06-1.06M4.46 4.46L3.4 3.4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
      Settings
    </button>

    <button class="footer-action" onclick={onsignout}>
      <!-- Log out icon -->
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M6 14H3a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1h3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M10.5 11.5L14 8l-3.5-3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M14 8H6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      Sign out
    </button>

    <button class="footer-action footer-quit" onclick={handleQuit}>
      <!-- X / power icon -->
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.5" />
        <path d="M8 3v5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
      Quit
    </button>
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
    gap: 0.625rem;
    /* Tightened from 0.875rem (v0.1.85) to give the body more vertical
       room for the workspace list. Horizontal padding stays at 1rem so
       the brand icon doesn't crowd the window edge. */
    padding: 0.625rem 1rem;
  }

  .popover-header.has-desktop-alt-controls {
    flex-wrap: wrap;
    row-gap: 0.375rem;
  }

  .header-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 10px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    color: var(--popover-primary, #ffffff);
    flex-shrink: 0;
    box-shadow: inset 0 1px 0 var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .header-text {
    min-width: 0;
    /* flex: 1 lets the title/path block soak up the spare horizontal space
       so the Sync button sits flush against the right edge of the header. */
    flex: 1;
  }

  .popover-header.has-desktop-alt-controls .header-text {
    order: 2;
    flex: 1 0 100%;
  }

  .header-text h1 {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    margin: 0;
    line-height: 1.3;
  }

  .header-path {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    margin: 0.125rem 0 0 0;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .header-utility-actions {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    flex-shrink: 0;
  }

  .header-icon-button {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    padding: 0;
    color: var(--popover-text, #e0e0e0);
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    border-radius: 8px;
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
    margin: -0.1875rem 1rem 0.5rem 3.625rem;
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
    gap: 0.5rem;
    padding: 0.5rem 0.875rem;
    font-family: inherit;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--popover-primary-text, #111113);
    background: var(--popover-primary, #ffffff);
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    border-radius: 8px;
    cursor: pointer;
    transition: background-color 0.15s ease, opacity 0.15s ease, color 0.15s ease;
    -webkit-app-region: no-drag;
  }

  .popover-header.has-desktop-alt-controls .header-sync {
    margin-left: auto;
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

  /* Divider */
  .popover-divider {
    height: 1px;
    background: var(--popover-divider, rgba(255, 255, 255, 0.06));
    margin: 0 0.75rem;
  }

  /* Body */
  .popover-body {
    /* Tightened from 0.75rem 1rem (v0.1.85): vertical padding + inter-card
       gap collapsed from 12px to 8px. Horizontal padding to 0.75rem so
       workspace rows get +8px of name room before truncation kicks in. */
    padding: 0.5rem 0.75rem;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
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


  /* Footer */
  .popover-footer {
    display: flex;
    flex-direction: column;
    padding: 0.25rem 0.5rem 0.5rem;
  }

  .footer-action {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.4375rem 0.5rem;
    font-size: 0.8125rem;
    font-family: inherit;
    color: var(--popover-text-muted, #a0a0b0);
    background: none;
    border: none;
    border-radius: 9px;
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

  /* HQ-version footer row. Same padding rhythm as `.footer-action` so it
     reads as part of the same column, but it's a div (not a button) — the
     row itself isn't clickable; the optional right-aligned pill /
     CopyPromptButton is the affordance. */
  .footer-hq-version {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.4375rem 0.5rem;
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

  /* Right-side action group (drift pill + Update/Update-to-Staging pill +
     rescue-result chip). Wraps to a second line when the popover is too
     narrow to fit everything beside the version label, rather than letting
     fixed-width pills overflow and overlap the label. */
  .footer-hq-version-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: 0.5rem;
    min-width: 0;
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

  .footer-hq-version-result-err {
    color: var(--popover-danger, #d56a6a);
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
</style>
