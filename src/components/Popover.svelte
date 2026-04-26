<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import SyncStats from './SyncStats.svelte';
  import SyncButton from './SyncButton.svelte';
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
  function truncatePath(p: string, max = 36): string {
    if (p.length <= max) return p;
    return '…' + p.slice(-(max - 1));
  }

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

      <SyncStats bind:this={statsEl} />

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

      <!-- Live progress detail — renders only while actively syncing. -->
      {#if syncState === 'syncing'}
        <div class="live-progress">
          {#if fanoutTotal > 0}
            <p class="live-line muted">
              Syncing {fanoutDoneCount + 1} of {fanoutTotal}
              {fanoutTotal === 1 ? 'company' : 'companies'}
            </p>
          {/if}
          {#if progress}
            <p class="live-line">
              <span class="live-company">{progress.company}</span>
              <span class="live-sep">·</span>
              <span class="live-path" title={progress.path}>{truncatePath(progress.path)}</span>
            </p>
            <p class="live-line muted">
              <span>↓ {formatBytes(progress.bytes)}</span>
            </p>
          {/if}
        </div>
      {:else if lastSummary && syncState === 'idle'}
        {#if lastSummary.filesDownloaded > 0}
          <p class="summary-line">
            Last sync · {lastSummary.filesDownloaded} file{lastSummary.filesDownloaded !== 1 ? 's' : ''} ·
            {formatBytes(lastSummary.bytesDownloaded)}
            {#if lastSummary.companiesAttempted > 1}
              across {lastSummary.companiesAttempted} companies
            {/if}
          </p>
        {:else if lastSummary.filesSkipped > 0}
          <p class="summary-line">
            Up to date · {lastSummary.filesSkipped} file{lastSummary.filesSkipped !== 1 ? 's' : ''}
            {#if lastSummary.companiesAttempted > 1}
              across {lastSummary.companiesAttempted} companies
            {/if}
          </p>
        {/if}
      {/if}

      <div class="sync-button-area">
        <SyncButton {syncState} {progress} onclick={onsync} />
      </div>
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

  .sync-button-area {
    margin-top: auto;
    padding-top: 0.25rem;
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

  /* Live progress — shown while actively syncing */
  .live-progress {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.625rem;
    border-radius: 6px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
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

  .live-company {
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    flex-shrink: 0;
  }

  .live-sep {
    color: var(--popover-text-muted, #a0a0b0);
    flex-shrink: 0;
  }

  .live-path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  /* Summary line — "Last sync · X files · Y MB" */
  .summary-line {
    margin: 0;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.4;
  }
</style>
