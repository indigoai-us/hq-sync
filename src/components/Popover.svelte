<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import SyncStats from './SyncStats.svelte';
  import ConflictModal from './ConflictModal.svelte';
  import WorkspaceList from './WorkspaceList.svelte';
  import type { Workspace } from '../lib/workspaces';
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
    /** Real expected file count for the entire sync — emitted by the Rust
     *  pre-walk before any uploads. When > 0, the bar uses it as the
     *  denominator for true per-file progress. When 0 (pre-walk hasn't
     *  fired yet, or hit an error), the bar falls back to workspace-level
     *  progress. */
    syncTotalFiles?: number;
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
    conflicts?: ConflictFile[];
    showConflictModal?: boolean;
    /** Non-null when the Tauri updater has found a newer release. */
    updateAvailable?: { version: string; body?: string; date?: string } | null;
    /** True while `install_update` is in flight — disables the button. */
    updateInstalling?: boolean;
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
    // Parent can call the returned fn to refresh SyncStats (bound to
    // the child's exported refresh()). We pass a setter down rather
    // than using bind:this because App.svelte holds the ref.
    bindStatsRefresh?: (fn: () => void) => void;
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
    companies = [],
    workspaces = null,
    cloudReachable = true,
    cloudError = null,
    manifestError = null,
    onworkspacesrefresh,
    lastSummary = null,
    errorMessage = '',
    conflicts = [],
    showConflictModal = false,
    updateAvailable = null,
    updateInstalling = false,
    onsync,
    oncancel,
    onsettings,
    onsignout,
    onresolve,
    onopen,
    ondismissconflicts,
    oninstallupdate,
    bindStatsRefresh,
  }: Props = $props();

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

  async function handleQuit() {
    try {
      // For a menubar app, closing the window hides the popover.
      // The Rust backend handles actual app exit via tray quit menu.
      await getCurrentWindow().close();
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
  <header class="popover-header" data-tauri-drag-region>
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
            ? 'Last sync failed — click to retry'
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
        Retry
      {:else if syncState === 'auth-error'}
        Sign in
      {:else}
        Sync
      {/if}
    </button>
  </header>

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
          <button
            class="banner-update-button"
            onclick={oninstallupdate}
            disabled={updateInstalling || !oninstallupdate}
          >
            {updateInstalling ? 'Installing…' : 'Install'}
          </button>
        </div>
      {/if}

      <!-- Runner state banners — auth and runtime errors only. The previous
           `setup-needed` "No companies yet" dead-end is gone: the WorkspaceList
           below ALWAYS renders the Personal row, so the menubar is never empty
           even for a fresh sign-in. The legacy onboarding.indigo-hq.com link
           lived here too — replaced by the live "Create a company" / "Join
           via invite" affordances inside WorkspaceList. -->
      {#if syncState === 'auth-error'}
        <div class="banner banner-error">
          <p class="banner-title">Session expired</p>
          <p class="banner-body">{errorMessage || 'Please sign in again to continue syncing.'}</p>
        </div>
      {:else if syncState === 'error' && errorMessage}
        <div class="banner banner-error">
          <p class="banner-title">Sync failed</p>
          <p class="banner-body">{errorMessage}</p>
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
          {#if syncTotalFiles > 0 && syncFilesProgressed <= syncTotalFiles}
            <!-- Real per-file caption: pre-walk computed the exact number
                 of transfers (uploads + downloads) the runner will do, so
                 the bar fills with each progress event. Skips don't fire
                 events and don't count toward either side. -->
            <p class="live-line muted">
              {syncFilesProgressed.toLocaleString()} of
              {syncTotalFiles.toLocaleString()} transferred
            </p>
          {:else if syncTotalFiles === 0 && fanoutTotal > 0}
            <!-- Pre-walk computed 0 transfers and the runner has started
                 (fanout-plan landed). Everything matches the journal —
                 sync will finalize in a moment with no actual work. -->
            <p class="live-line muted">Up to date — finalizing…</p>
          {:else if syncFilesProgressed > 0}
            <!-- Bar overshot the estimate (pre-walk under-counted because
                 we don't yet count pull-side downloads). Show the honest
                 running count rather than a fake "X of Y". -->
            <p class="live-line muted">
              {syncFilesProgressed.toLocaleString()} transferred
            </p>
          {:else if fanoutTotal > 0}
            <!-- Fallback: pre-walk hasn't landed yet (or returned 0).
                 Show workspace progress + rolling file count. -->
            <p class="live-line muted">
              Workspace {Math.min(fanoutDoneCount + 1, fanoutTotal)} of {fanoutTotal}
              {#if syncFilesProgressed > 0}
                · {syncFilesProgressed.toLocaleString()} file{syncFilesProgressed === 1 ? '' : 's'}
              {/if}
            </p>
          {:else if personalFilesTotal != null && personalFilesTotal > 0}
            <p class="live-line muted">
              {personalFilesDone} of {personalFilesTotal} files
            </p>
          {/if}
        </div>
      {:else}
        <SyncStats bind:this={statsEl} />
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
    padding: 0.875rem 1rem;
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

  .header-sync.error {
    background: var(--popover-danger, #ef4444);
    color: #ffffff;
    border-color: rgba(239, 68, 68, 0.6);
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

  /* Divider */
  .popover-divider {
    height: 1px;
    background: var(--popover-divider, rgba(255, 255, 255, 0.06));
    margin: 0 0.75rem;
  }

  /* Body */
  .popover-body {
    padding: 0.75rem 1rem;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
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

  /* Banners — actionable state callouts (setup / auth / error) */
  .banner {
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
    padding: 0.625rem 0.75rem;
    border-radius: 10px;
    border: 1px solid transparent;
  }

  .banner-info {
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border-color: var(--popover-border, rgba(255, 255, 255, 0.18));
  }

  .banner-error {
    background: rgba(239, 68, 68, 0.08);
    border-color: rgba(239, 68, 68, 0.25);
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

  /* Update banner: horizontal layout — text on the left, Install on the right */
  .banner-update {
    flex-direction: row;
    align-items: center;
    gap: 0.625rem;
  }

  .banner-update-text {
    min-width: 0;
    flex: 1;
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
