<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import SyncStats from './SyncStats.svelte';
  import SyncButton from './SyncButton.svelte';
  import ConflictModal from './ConflictModal.svelte';
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
    lastSummary?: {
      companiesAttempted: number;
      filesDownloaded: number;
      bytesDownloaded: number;
    } | null;
    errorMessage?: string;
    conflicts?: ConflictFile[];
    showConflictModal?: boolean;
    onsync: () => void;
    onsettings: () => void;
    onsignout: () => void;
    onresolve?: (path: string, strategy: 'keep-local' | 'keep-remote') => void;
    onopen?: (path: string) => void;
    ondismissconflicts?: () => void;
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
    lastSummary = null,
    errorMessage = '',
    conflicts = [],
    showConflictModal = false,
    onsync,
    onsettings,
    onsignout,
    onresolve,
    onopen,
    ondismissconflicts,
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
  <header class="popover-header">
    <div class="header-icon">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M12 2L2 7l10 5 10-5-10-5Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M2 17l10 5 10-5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M2 12l10 5 10-5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
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
      <!-- Runner state banners — setup / auth surfaces as actionable banners,
           not as silent error states. These short-circuit the usual stats view. -->
      {#if syncState === 'setup-needed'}
        <div class="banner banner-warn">
          <p class="banner-title">Finish setting up HQ</p>
          <p class="banner-body">Visit <strong>onboarding.indigo-hq.com</strong> to complete your profile before syncing.</p>
        </div>
      {:else if syncState === 'auth-error'}
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
        <p class="summary-line">
          Last sync · {lastSummary.filesDownloaded} files ·
          {formatBytes(lastSummary.bytesDownloaded)}
          {#if lastSummary.companiesAttempted > 1}
            across {lastSummary.companiesAttempted} companies
          {/if}
        </p>
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
    width: 320px;
    max-height: 400px;
    background: var(--popover-bg, #1a1a2e);
    color: var(--popover-text, #e0e0e0);
    overflow: hidden;
    /* Rounded corners — requires tauri window transparent:true +
       decorations:false + macOSPrivateApi:true for the OS to honor
       transparency outside the radius. */
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35),
                0 2px 6px rgba(0, 0, 0, 0.25);
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
    border-radius: 8px;
    background: rgba(99, 102, 241, 0.12);
    color: var(--popover-primary, #6366f1);
    flex-shrink: 0;
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
    border-radius: 6px;
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
    border-radius: 8px;
    border: 1px solid transparent;
  }

  .banner-warn {
    background: rgba(234, 179, 8, 0.08);
    border-color: rgba(234, 179, 8, 0.25);
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

  .banner-body strong {
    color: var(--popover-text, #e0e0e0);
    font-weight: 600;
  }

  /* Live progress — shown while actively syncing */
  .live-progress {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.625rem;
    border-radius: 6px;
    background: rgba(99, 102, 241, 0.06);
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
    color: var(--popover-primary, #6366f1);
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
