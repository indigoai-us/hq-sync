<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { companyStore } from '../lib/company-store.svelte';
  import Sparkline from '../components/Sparkline.svelte';
  import StatTile from '../components/StatTile.svelte';
  import OpenFileInClaudeCode from '../components/OpenFileInClaudeCode.svelte';

  interface Props {
    slug: string;
  }

  interface ActivityStats {
    files7: number;
    edits7: number;
    members: number;
    vaultSize: string;
  }

  interface ActivityEntry {
    who: string;
    what: string;
    file: string;
    when: string;
  }

  interface ActivityContributor {
    who: string;
    edits: number;
  }

  interface CompanyActivity {
    stats: ActivityStats;
    sparkline: number[];
    recent: ActivityEntry[];
    top: ActivityContributor[];
  }

  let { slug }: Props = $props();

  const emptyStats = (): ActivityStats => ({
    files7: 0,
    edits7: 0,
    members: 0,
    vaultSize: '',
  });

  const emptyActivity = (): CompanyActivity => ({
    stats: emptyStats(),
    sparkline: [],
    recent: [],
    top: [],
  });

  let activity = $state<CompanyActivity>(emptyActivity());
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadToken = $state(0);

  // HQ root for the Claude Code drill-in (US-012). Loaded lazily via get_config
  // (same command App.svelte uses; Tauri caches the read). Empty until loaded —
  // at which point each activity entry's "Open in Claude Code" affordance
  // suppresses itself. Best-effort: a failure leaves it empty and rows simply
  // render without the drill-in.
  let hqFolderPath = $state('');

  $effect(() => {
    let cancelled = false;
    void invoke<{ hqFolderPath?: string }>('get_config')
      .then((config) => {
        if (!cancelled) hqFolderPath = config?.hqFolderPath ?? '';
      })
      .catch((err) => {
        console.error('ActivityPanel get_config failed:', err);
      });
    return () => {
      cancelled = true;
    };
  });

  const sparklineMax = $derived(Math.max(1, ...activity.sparkline));
  const contributorMax = $derived(Math.max(1, ...activity.top.map((contributor) => contributor.edits)));
  const recentCount = $derived(activity.recent.length);

  $effect(() => {
    reloadToken;
    activity = emptyActivity();
    error = null;

    if (!slug) {
      loading = false;
      return;
    }

    let cancelled = false;

    const warm = companyStore.activity(slug);
    activity = warm != null ? normalizeActivity(warm as Partial<CompanyActivity>) : emptyActivity();
    loading = warm == null;

    void invoke<Partial<CompanyActivity>>('get_company_activity', { slug })
      .then((result) => {
        if (!cancelled) {
          activity = normalizeActivity(result);
          companyStore.setActivity(slug, result);
        }
      })
      .catch((err) => {
        console.error('get_company_activity failed:', err);
        if (!cancelled) {
          error = String(err);
          activity = emptyActivity();
        }
      })
      .finally(() => {
        if (!cancelled) {
          loading = false;
        }
      });

    return () => {
      cancelled = true;
    };
  });

  function normalizeActivity(result: Partial<CompanyActivity> | null | undefined): CompanyActivity {
    const stats = result?.stats ?? emptyStats();
    return {
      stats: {
        files7: numberOrZero(stats.files7),
        edits7: numberOrZero(stats.edits7),
        members: numberOrZero(stats.members),
        vaultSize: typeof stats.vaultSize === 'string' ? stats.vaultSize : '',
      },
      sparkline: Array.isArray(result?.sparkline) ? result.sparkline.map(numberOrZero) : [],
      recent: Array.isArray(result?.recent) ? result.recent.map(normalizeEntry) : [],
      top: Array.isArray(result?.top) ? result.top.map(normalizeContributor) : [],
    };
  }

  function numberOrZero(value: unknown): number {
    return typeof value === 'number' && Number.isFinite(value) ? value : 0;
  }

  function normalizeEntry(entry: Partial<ActivityEntry>): ActivityEntry {
    return {
      who: typeof entry.who === 'string' ? entry.who : '',
      what: typeof entry.what === 'string' ? entry.what : '',
      file: typeof entry.file === 'string' ? entry.file : 'Untitled file',
      when: typeof entry.when === 'string' ? entry.when : '',
    };
  }

  function normalizeContributor(contributor: Partial<ActivityContributor>): ActivityContributor {
    return {
      who: typeof contributor.who === 'string' ? contributor.who : 'Unknown',
      edits: numberOrZero(contributor.edits),
    };
  }

  function barHeight(value: number): string {
    return `${(value / sparklineMax) * 100}%`;
  }

  function contributorWidth(edits: number): string {
    return `${(edits / contributorMax) * 100}%`;
  }

  function initialFor(who: string): string {
    return who.trim().charAt(0).toUpperCase() || '?';
  }

  function retry() {
    reloadToken += 1;
  }
</script>

<section class="activity-panel" aria-labelledby="activity-panel-title">
  <header class="activity-toolbar">
    <div class="activity-title">
      <h2 id="activity-panel-title">Activity</h2>
      <span>{loading ? 'Loading activity' : 'Last 14 days'}</span>
    </div>
  </header>

  {#if error}
    <div class="activity-error" role="alert">
      <div>
        <strong>Activity unavailable</strong>
        <span>{error}</span>
      </div>
      <button type="button" onclick={retry}>Retry</button>
    </div>
  {/if}

  <div class="stats-grid" aria-busy={loading}>
    <StatTile label="New files · 14d" value={activity.stats.files7} {loading} />
    <StatTile label="Edits · 14d" value={activity.stats.edits7} {loading} />
    <StatTile label="Members" value={activity.stats.members} {loading} />
    <StatTile label="Vault size" value={activity.stats.vaultSize || '0'} {loading} />
  </div>

  <div class="activity-grid">
    <section class="activity-card edits-card" aria-labelledby="edits-over-time-title" aria-busy={loading}>
      <header class="card-header">
        <h3 id="edits-over-time-title">Edits over time</h3>
        {#if activity.sparkline.length > 0}
          <span class="sparkline-wrap">
            <Sparkline data={activity.sparkline} width={120} height={20} />
          </span>
        {/if}
      </header>

      {#if loading}
        <div class="chart-skeleton" aria-label="Loading edits over time">
          {#each Array(14) as _, index (index)}
            <span style={`height: ${24 + (index % 5) * 13}%`}></span>
          {/each}
        </div>
      {:else if activity.sparkline.length > 0}
        <div class="bar-chart" aria-label="Edits over time bar chart">
          {#each activity.sparkline as value, index (index)}
            <span
              class="activity-bar"
              style={`height: ${barHeight(value)}`}
              title={`${value} edits`}
            ></span>
          {/each}
        </div>
        <div class="chart-scale" aria-hidden="true">
          <span>14 days ago</span>
          <span>today</span>
        </div>
      {:else}
        <div class="empty-state">No activity yet</div>
      {/if}
    </section>

    <section class="activity-card" aria-labelledby="top-contributors-title" aria-busy={loading}>
      <header class="card-header">
        <h3 id="top-contributors-title">Top contributors</h3>
        <span>last 14 days</span>
      </header>

      {#if loading}
        <div class="contributor-skeleton" aria-label="Loading top contributors">
          {#each Array(4) as _, index (index)}
            <span style={`width: ${84 - index * 13}%`}></span>
          {/each}
        </div>
      {:else if activity.top.length > 0}
        <div class="contributors-list">
          {#each activity.top as contributor, index (`${contributor.who}:${index}`)}
            <div class="contributor-row">
              <div class="contributor-body">
                <span>{contributor.who}</span>
                <div class="contributor-track" aria-hidden="true">
                  <span
                    class="contributor-fill"
                    style={`width: ${contributorWidth(contributor.edits)}`}
                  ></span>
                </div>
              </div>
              <strong>{contributor.edits}</strong>
            </div>
          {/each}
        </div>
      {:else}
        <div class="empty-state">No activity yet</div>
      {/if}
    </section>
  </div>

  <section class="activity-card recent-card" aria-labelledby="recent-files-title" aria-busy={loading}>
    <header class="card-header">
      <h3 id="recent-files-title">Recent files</h3>
      <span>{recentCount} of {activity.stats.files7}</span>
    </header>

    {#if loading}
      <div class="recent-skeleton" aria-label="Loading recent files">
        {#each Array(5) as _, index (index)}
          <span style={`width: ${92 - index * 7}%`}></span>
        {/each}
      </div>
    {:else if activity.recent.length > 0}
      <div class="recent-list" data-testid="activity-recent-list">
        {#each activity.recent as entry, index (`${entry.file}:${index}`)}
          <div class="recent-row" data-testid="activity-row">
            <span class="avatar" title={entry.who}>{initialFor(entry.who)}</span>
            <div class="recent-copy">
              <strong title={entry.file}>{entry.file}</strong>
              <span>{entry.who} · {entry.what}</span>
            </div>
            <time>{entry.when}</time>
            <!--
              Drill-in (US-012): the only path the activity data carries is
              `entry.file`, so the entry drills into that file via the shared
              Claude Code link util — same mechanism as the story-files
              affordance. We surface it only for entries that actually name a
              file (the normalizer falls back to "Untitled file" when absent).
            -->
            {#if entry.file && entry.file !== 'Untitled file'}
              <OpenFileInClaudeCode
                file={entry.file}
                folder={hqFolderPath}
                label="Open"
              />
            {/if}
          </div>
        {/each}
      </div>
    {:else}
      <div class="empty-state">No activity yet</div>
    {/if}
  </section>
</section>

<style>
  .activity-panel {
    display: grid;
    gap: 14px;
    min-width: 0;
  }

  .activity-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-width: 0;
  }

  .activity-title {
    min-width: 0;
  }

  .activity-title h2 {
    margin: 0;
    color: var(--fg);
    font-size: 16px;
    font-weight: 680;
    line-height: 22px;
  }

  .activity-title span,
  .card-header span,
  .chart-scale,
  .empty-state {
    color: var(--muted);
    font-size: 12px;
    line-height: 16px;
  }

  .activity-title span {
    display: block;
    margin-top: 2px;
  }

  .activity-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    min-width: 0;
    padding: 12px;
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: 8px;
    background: rgba(245, 158, 11, 0.1);
    color: var(--amber);
  }

  .activity-error div {
    display: grid;
    gap: 3px;
    min-width: 0;
  }

  .activity-error strong,
  .activity-error span {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .activity-error strong {
    font-size: 13px;
    line-height: 18px;
  }

  .activity-error span {
    font-size: 12px;
    line-height: 16px;
  }

  .activity-error button {
    height: 30px;
    padding: 0 11px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    white-space: nowrap;
    cursor: default;
  }

  .stats-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(140px, 1fr));
    gap: 12px;
    min-width: 0;
  }

  .activity-grid {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(260px, 0.8fr);
    gap: 12px;
    min-width: 0;
  }

  .activity-card {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }

  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
    padding: 11px 13px;
    border-bottom: 1px solid var(--border);
  }

  .card-header h3 {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--muted-2);
    font-size: 13px;
    font-weight: 700;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sparkline-wrap {
    flex: 0 0 auto;
    color: var(--muted-3);
  }

  .bar-chart,
  .chart-skeleton {
    display: flex;
    align-items: flex-end;
    gap: 4px;
    height: 120px;
    min-width: 0;
    padding: 14px 16px 0;
  }

  .activity-bar {
    flex: 1 1 0;
    min-width: 4px;
    border-top: 1px solid var(--muted-3);
    background: rgba(255, 255, 255, 0.12);
    transition: height 300ms ease;
  }

  .chart-scale {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 16px 14px;
  }

  .contributors-list {
    display: grid;
    gap: 3px;
    padding: 6px;
  }

  .contributor-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
    min-width: 0;
    padding: 7px 10px;
  }

  .contributor-body {
    min-width: 0;
  }

  .contributor-body > span {
    display: block;
    min-width: 0;
    overflow: hidden;
    color: var(--muted-2);
    font-size: 13px;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .contributor-row strong {
    color: var(--muted-3);
    font-family: 'Geist Mono', ui-monospace, SFMono-Regular, monospace;
    font-size: 12px;
    font-weight: 650;
    line-height: 16px;
  }

  .contributor-track {
    position: relative;
    height: 3px;
    margin-top: 5px;
    overflow: hidden;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
  }

  .contributor-fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: inherit;
    background: var(--fg);
    opacity: 0.62;
    transition: width 380ms ease;
  }

  .recent-list {
    display: grid;
  }

  .recent-row {
    display: grid;
    grid-template-columns: 28px minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 10px;
    min-width: 0;
    padding: 9px 13px;
    border-top: 1px solid var(--border);
  }

  .recent-row:first-child {
    border-top: 0;
  }

  .avatar {
    width: 28px;
    height: 28px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-active);
    color: var(--fg);
    font-size: 10px;
    font-weight: 750;
    line-height: 28px;
    text-align: center;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .recent-copy {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  .recent-copy strong,
  .recent-copy span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .recent-copy strong {
    color: var(--fg);
    font-size: 13px;
    font-weight: 650;
    line-height: 18px;
  }

  .recent-copy span,
  .recent-row time {
    color: var(--muted);
    font-size: 12px;
    line-height: 16px;
  }

  .recent-row time {
    font-family: 'Geist Mono', ui-monospace, SFMono-Regular, monospace;
    white-space: nowrap;
  }

  /* The recent-row drill-in (OpenFileInClaudeCode) reveals on row hover /
     keyboard focus — matching the affordance language of the board +
     deployments rows. */
  .recent-row :global(.open-claude-btn) {
    opacity: 0;
    transition: opacity 140ms ease;
  }

  .recent-row:hover :global(.open-claude-btn),
  .recent-row :global(.open-claude-btn:focus-visible) {
    opacity: 1;
  }

  .empty-state {
    padding: 16px;
  }

  .chart-skeleton span,
  .contributor-skeleton span,
  .recent-skeleton span {
    display: block;
    overflow: hidden;
    border-radius: 999px;
    background: linear-gradient(
      90deg,
      rgba(255, 255, 255, 0.05) 0%,
      rgba(255, 255, 255, 0.1) 46%,
      rgba(255, 255, 255, 0.05) 100%
    );
    background-size: 180% 100%;
    animation: skeleton-pulse 1100ms ease-in-out infinite;
  }

  .chart-skeleton span {
    flex: 1 1 0;
    min-width: 4px;
    border-radius: 4px 4px 0 0;
  }

  .contributor-skeleton,
  .recent-skeleton {
    display: grid;
    gap: 12px;
    padding: 16px;
  }

  .contributor-skeleton span {
    height: 21px;
  }

  .recent-skeleton span {
    height: 28px;
  }

  @keyframes skeleton-pulse {
    from {
      background-position: 100% 0;
    }

    to {
      background-position: 0 0;
    }
  }

  @media (max-width: 980px) {
    .stats-grid {
      grid-template-columns: repeat(2, minmax(140px, 1fr));
    }

    .activity-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  @media (max-width: 680px) {
    .stats-grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .recent-row {
      grid-template-columns: 28px minmax(0, 1fr) auto;
    }

    .recent-row :global(.open-claude-btn) {
      display: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .activity-bar,
    .contributor-fill,
    .chart-skeleton span,
    .contributor-skeleton span,
    .recent-skeleton span {
      transition: none;
      animation: none;
    }

    .recent-row :global(.open-claude-btn) {
      transition: none;
    }
  }
</style>
