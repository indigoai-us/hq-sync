<script lang="ts">
  import type { Workspace } from '../../lib/workspaces';
  import type {
    ActivityEntry,
    DaemonStatus,
    SyncCompanyRef,
    SyncProgress,
    SyncState,
    SyncStatus,
    WorkspaceSyncStats,
  } from '../lib/sync-model';
  import { formatRelativeTime } from '../route';
  import ActivityDigest from '../v4/ActivityDigest.svelte';
  import NeedsYouCard from '../v4/NeedsYouCard.svelte';
  import {
    formatClock,
    getConflictCardModel,
    getDriftCardModel,
    getHomeDigestGroups,
    getHomeErrorModel,
    getHomeMetaLine,
    getHomeProgressModel,
    getNeedsYouCount,
    type HomeConflict,
    type HomeCoreState,
  } from '../v4/home-model';

  /**
   * V4 Home — the exception-based surface (SPEC section 5,
   * home-healthy/syncing/error.png). Presentational: DesktopApp owns the sync
   * event stream, the conflict queue, and the core-drift state; this page
   * renders the meta line, the NEEDS YOU queue, the syncing progress card,
   * the error card, and the actor-grouped digest. Supersedes SyncPage
   * (US-003) — the sources-table mental model is gone.
   */
  interface Props {
    syncState: SyncState;
    /** False during the first real-state fetch. */
    ready?: boolean;
    workspaces: Workspace[];
    progress: SyncProgress | null;
    companies: SyncCompanyRef[];
    statsBySlug: Record<string, WorkspaceSyncStats>;
    status: SyncStatus | null;
    daemon: DaemonStatus | null;
    activity: ActivityEntry[];
    syncErrorMessage: string;
    /** Company the failing run reported, when the error event carried one. */
    syncErrorCompany?: string | null;
    syncFilesProgressed: number;
    syncTotalFiles: number;
    transferredBytes: number;
    /** Epoch ms when the running sync started (for the meta line). */
    syncStartedAt?: number | null;
    /** `realtimeSync` preference; null while loading. */
    autoSyncOn?: boolean | null;
    /** Local hq-core version ("15.0.15"); null when unreadable. */
    hqVersion?: string | null;
    conflicts: HomeConflict[];
    coreState?: HomeCoreState | null;
    driftDismissed?: boolean;
    driftRestoring?: boolean;
    onresolveconflict?: (path: string, strategy: 'keep-local' | 'keep-remote') => void;
    oncompareconflict?: (path: string) => void;
    onrestoredrift?: () => void;
    onkeepdrift?: () => void;
    onviewdrift?: () => void;
    onsignin?: () => void;
    onretry?: () => void;
    onopenlog?: () => void;
  }

  let {
    syncState,
    ready = true,
    workspaces,
    progress,
    companies,
    statsBySlug,
    status,
    daemon,
    activity,
    syncErrorMessage,
    syncErrorCompany = null,
    syncFilesProgressed,
    syncTotalFiles,
    transferredBytes,
    syncStartedAt = null,
    autoSyncOn = null,
    hqVersion = null,
    conflicts,
    coreState = null,
    driftDismissed = false,
    driftRestoring = false,
    onresolveconflict,
    oncompareconflict,
    onrestoredrift,
    onkeepdrift,
    onviewdrift,
    onsignin,
    onretry,
    onopenlog,
  }: Props = $props();

  let techOpen = $state(false);

  const lastSyncLabel = $derived(formatRelativeTime(status?.lastSyncAt ?? null));
  const metaLine = $derived(
    getHomeMetaLine({
      syncState,
      autoSyncOn,
      daemonRunning: daemon?.running ?? null,
      lastSyncLabel,
      hqVersion,
      syncStartedLabel: syncStartedAt ? formatClock(syncStartedAt) : null,
    }),
  );

  const syncing = $derived(syncState === 'syncing');
  const errorModel = $derived(
    getHomeErrorModel({
      syncState,
      syncErrorMessage,
      errorCompany: syncErrorCompany,
      workspaces,
      companies,
      appVersion: __APP_VERSION__,
      lastSyncLabel,
    }),
  );
  const driftCard = $derived(
    coreState && !driftDismissed ? getDriftCardModel(coreState, driftRestoring) : null,
  );
  const needsYouCount = $derived(getNeedsYouCount(conflicts, coreState, driftDismissed));
  const progressModel = $derived(
    getHomeProgressModel({
      filesProgressed: syncFilesProgressed,
      totalFiles: syncTotalFiles,
      transferredBytes,
      progress,
      companies,
      statsBySlug,
      workspaces,
    }),
  );
  const digestGroups = $derived(getHomeDigestGroups(activity, workspaces, companies));

  function handleConflictAction(path: string, actionId: string) {
    if (actionId === 'compare') oncompareconflict?.(path);
    else onresolveconflict?.(path, actionId as 'keep-local' | 'keep-remote');
  }

  function handleDriftAction(actionId: string) {
    if (actionId === 'restore') onrestoredrift?.();
    else if (actionId === 'keep-edit') onkeepdrift?.();
    else if (actionId === 'view-diff') onviewdrift?.();
  }

  function handleErrorAction(actionId: string) {
    if (actionId === 'sign-in') onsignin?.();
    else onretry?.();
  }
</script>

<section class="home" aria-label="Home">
  <header class="home-header">
    <h1 class="home-title">Home</h1>
    <p class="home-meta">{metaLine}</p>
  </header>

  {#if errorModel}
    <div class="home-section" aria-label="Sync failed">
      <h2 class="home-label error">
        <span class="home-label-dot error" aria-hidden="true"></span>
        Sync failed{syncErrorCompany ? ' · 1 company' : ''}
      </h2>
      <NeedsYouCard
        card={{
          title: errorModel.title,
          sub: errorModel.sub,
          tone: 'error',
          actions: [
            ...(errorModel.showSignIn
              ? [{ id: 'sign-in', label: 'Sign in again', kind: 'primary' as const }]
              : []),
            {
              id: 'retry',
              label: 'Retry',
              kind: errorModel.showSignIn ? ('secondary' as const) : ('primary' as const),
            },
          ],
        }}
        onaction={handleErrorAction}
      >
        <div class="home-tech">
          <button
            type="button"
            class="home-tech-toggle"
            aria-expanded={techOpen}
            onclick={() => (techOpen = !techOpen)}
          >
            {techOpen ? '⌄' : '›'} Technical details
          </button>
          {#if techOpen}
            <div class="home-tech-body">
              {#each errorModel.techLines as line (line)}
                <p class="home-tech-line">{line}</p>
              {/each}
            </div>
          {/if}
        </div>
      </NeedsYouCard>
    </div>
  {/if}

  {#if syncing}
    <div class="home-section" aria-label="Sync in progress">
      <h2 class="home-label">
        <span class="home-label-dot ok" aria-hidden="true"></span>
        Sync in progress
      </h2>
      <div class="home-progress" data-testid="home-progress-card">
        <div class="home-progress-head">
          <span class="home-progress-headline">{progressModel.headline}</span>
          <span class="home-progress-meta">{progressModel.meta}</span>
        </div>
        <div
          class="home-progress-track"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={progressModel.pct == null ? undefined : Math.round(progressModel.pct)}
        >
          <div
            class="home-progress-fill"
            class:indeterminate={progressModel.pct == null}
            style={progressModel.pct == null ? undefined : `width: ${progressModel.pct}%`}
          ></div>
        </div>
        <ol class="home-fanout">
          {#each progressModel.rows as row (row.slug)}
            <li class="home-fanout-row">
              <span
                class={`home-fanout-dot ${row.state === 'queued' ? 'idle' : 'ok'}`}
                aria-hidden="true"
              ></span>
              <span class="home-fanout-name" class:active={row.state === 'active'}>
                {row.name}
              </span>
              <span class="home-fanout-detail">{row.detail}</span>
            </li>
          {/each}
          {#if progressModel.queued}
            <li class="home-fanout-row">
              <span class="home-fanout-dot idle" aria-hidden="true"></span>
              <span class="home-fanout-name queued">
                {progressModel.queued.count} more queued
              </span>
              <span class="home-fanout-detail">{progressModel.queued.names}…</span>
            </li>
          {/if}
        </ol>
      </div>
    </div>
  {/if}

  {#if !syncing && !errorModel && needsYouCount > 0}
    <div class="home-section" aria-label="Needs you">
      <h2 class="home-label warn">
        <span class="home-label-dot warn" aria-hidden="true"></span>
        Needs you · {needsYouCount}
      </h2>
      <div class="home-queue">
        {#each conflicts as conflict (conflict.path)}
          <NeedsYouCard
            card={getConflictCardModel(conflict)}
            onaction={(id) => handleConflictAction(conflict.path, id)}
          />
        {/each}
        {#if driftCard}
          <NeedsYouCard card={driftCard} onaction={handleDriftAction} />
        {/if}
      </div>
    </div>
  {/if}

  {#if ready}
    <ActivityDigest groups={digestGroups} {onopenlog} />
  {:else}
    <div class="home-skeleton" aria-busy="true">
      {#each [0, 1, 2] as row (row)}
        <span class="home-skeleton-bar" style={`width: ${78 - row * 14}%`}></span>
      {/each}
    </div>
  {/if}
</section>

<style>
  .home {
    display: grid;
    gap: 18px;
    align-content: start;
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .home-header {
    display: grid;
    gap: 4px;
  }

  .home-title {
    margin: 0;
    color: var(--v4-text-1);
    font-size: 14px;
    font-weight: 500;
    line-height: 1.3;
  }

  .home-meta {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.4;
  }

  .home-section {
    display: grid;
    gap: 8px;
  }

  .home-label {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .home-label-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .home-label-dot.ok {
    background: var(--v4-ok);
  }

  .home-label-dot.warn {
    background: var(--v4-warn);
  }

  .home-label-dot.error {
    background: var(--v4-error);
  }

  .home-queue {
    display: grid;
    gap: 8px;
  }

  /* ── Syncing progress card ─────────────────────────────────────────────── */
  .home-progress {
    display: grid;
    gap: 10px;
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
  }

  .home-progress-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .home-progress-headline {
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 500;
  }

  .home-progress-meta {
    color: var(--v4-text-3);
    font-size: 11px;
  }

  .home-progress-track {
    height: 3px;
    border-radius: 999px;
    background: var(--v4-control-faint);
    overflow: hidden;
  }

  .home-progress-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--v4-text-1);
    transition: width 200ms ease;
  }

  .home-progress-fill.indeterminate {
    width: 30%;
    animation: home-progress-slide 1.2s ease-in-out infinite;
  }

  @keyframes home-progress-slide {
    0% {
      transform: translateX(-100%);
    }

    100% {
      transform: translateX(360%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .home-progress-fill {
      transition: none;
    }

    .home-progress-fill.indeterminate {
      animation: none;
    }
  }

  .home-fanout {
    display: grid;
    gap: 0;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .home-fanout-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 5px 0;
  }

  .home-fanout-dot {
    flex: 0 0 6px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    align-self: center;
  }

  .home-fanout-dot.ok {
    background: var(--v4-ok);
  }

  .home-fanout-dot.idle {
    background: var(--v4-idle);
  }

  .home-fanout-name {
    flex: 0 0 auto;
    min-width: 110px;
    color: var(--v4-text-2);
    font-size: 13px;
  }

  .home-fanout-name.active {
    color: var(--v4-text-1);
    font-weight: 500;
  }

  .home-fanout-name.queued {
    color: var(--v4-text-3);
  }

  .home-fanout-detail {
    overflow: hidden;
    min-width: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── Error technical details inset ─────────────────────────────────────── */
  .home-tech {
    margin-top: 10px;
  }

  .home-tech-toggle {
    padding: 0;
    border: none;
    background: none;
    color: var(--v4-text-3);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }

  .home-tech-toggle:hover {
    color: var(--v4-text-2);
  }

  .home-tech-body {
    display: grid;
    gap: 4px;
    margin-top: 8px;
    padding: 10px 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-inset);
  }

  .home-tech-line {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  /* ── First-load skeleton ───────────────────────────────────────────────── */
  .home-skeleton {
    display: grid;
    gap: 10px;
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
  }

  .home-skeleton-bar {
    display: block;
    height: 10px;
    border-radius: 999px;
    background: var(--v4-control-faint);
    animation: home-skeleton-pulse 1.2s ease-in-out infinite;
  }

  @keyframes home-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }

    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .home-skeleton-bar {
      animation: none;
    }
  }
</style>
