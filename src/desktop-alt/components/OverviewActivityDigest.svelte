<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { companyStore } from '../lib/company-store.svelte';
  import Sparkline from './Sparkline.svelte';

  /**
   * Compact team-activity digest for the company Overview. Reuses the exact
   * data the Activity tab already loads (`get_company_activity`, warmed through
   * `companyStore`), so the Overview becomes a real at-a-glance dashboard with
   * NO extra backend work — it just surfaces signals the desktop already fetched
   * but only showed on a separate tab: 7-day edits, members, vault size, the
   * edits-over-time trend, and the top contributors. All values are real;
   * empty/zero states render honestly.
   */
  interface Props {
    slug: string;
    cloudBacked?: boolean;
  }

  interface ActivityStats {
    files7: number;
    edits7: number;
    members: number;
    vaultSize: string;
  }
  interface ActivityContributor {
    who: string;
    edits: number;
  }
  interface CompanyActivity {
    stats: ActivityStats;
    sparkline: number[];
    top: ActivityContributor[];
  }

  let { slug, cloudBacked = true }: Props = $props();

  const emptyStats = (): ActivityStats => ({ files7: 0, edits7: 0, members: 0, vaultSize: '' });
  const emptyActivity = (): CompanyActivity => ({ stats: emptyStats(), sparkline: [], top: [] });

  let activity = $state<CompanyActivity>(emptyActivity());
  let loading = $state(false);

  const numberOrZero = (value: unknown): number =>
    typeof value === 'number' && Number.isFinite(value) ? value : 0;

  function normalize(result: Partial<CompanyActivity> | null | undefined): CompanyActivity {
    const stats = result?.stats ?? emptyStats();
    return {
      stats: {
        files7: numberOrZero(stats.files7),
        edits7: numberOrZero(stats.edits7),
        members: numberOrZero(stats.members),
        vaultSize: typeof stats.vaultSize === 'string' ? stats.vaultSize : '',
      },
      sparkline: Array.isArray(result?.sparkline) ? result.sparkline.map(numberOrZero) : [],
      top: Array.isArray(result?.top)
        ? result.top.map((c) => ({
            who: typeof c?.who === 'string' ? c.who : 'Unknown',
            edits: numberOrZero(c?.edits),
          }))
        : [],
    };
  }

  $effect(() => {
    activity = emptyActivity();
    if (!slug || !cloudBacked) {
      loading = false;
      return;
    }
    let cancelled = false;

    // Warm from the shared store so switching between Overview and Activity is
    // instant and the network round-trip is shared, not duplicated.
    const warm = companyStore.activity(slug);
    activity = warm != null ? normalize(warm as Partial<CompanyActivity>) : emptyActivity();
    loading = warm == null;

    void invoke<Partial<CompanyActivity>>('get_company_activity', { slug })
      .then((result) => {
        if (!cancelled) {
          activity = normalize(result);
          companyStore.setActivity(slug, result);
        }
      })
      .catch((err) => {
        // Non-fatal: the Overview's board/goals still render. A failed activity
        // fetch just leaves the digest empty rather than erroring the page.
        console.warn(`get_company_activity(${slug}) failed:`, err);
      })
      .finally(() => {
        if (!cancelled) loading = false;
      });

    return () => {
      cancelled = true;
    };
  });

  const sparklineMax = $derived(Math.max(1, ...activity.sparkline));
  const contributorMax = $derived(Math.max(1, ...activity.top.map((c) => c.edits)));
  const hasActivity = $derived(
    activity.top.length > 0 || activity.sparkline.some((v) => v > 0) || activity.stats.edits7 > 0,
  );
  const barHeight = (value: number): string => `${(value / sparklineMax) * 100}%`;
  const contributorWidth = (value: number): string => `${(value / contributorMax) * 100}%`;
</script>

<section class="digest" aria-labelledby="overview-activity-title" aria-busy={loading}>
  <header class="digest-header">
    <h2 id="overview-activity-title">TEAM ACTIVITY</h2>
    <span class="digest-sub">last 14 days</span>
  </header>

  <div class="digest-stats">
    <div class="digest-stat"><strong>{activity.stats.edits7}</strong><span>edits · 7d</span></div>
    <div class="digest-stat"><strong>{activity.stats.files7}</strong><span>files · 7d</span></div>
    <div class="digest-stat"><strong>{activity.stats.members}</strong><span>members</span></div>
    <div class="digest-stat"><strong>{activity.stats.vaultSize || '0'}</strong><span>vault size</span></div>
  </div>

  {#if !cloudBacked}
    <p class="digest-empty">Connect this company to see team activity.</p>
  {:else if loading && !hasActivity}
    <div class="digest-skeleton" aria-hidden="true">
      {#each [0, 1, 2] as row (row)}<span style={`width: ${78 - row * 18}%`}></span>{/each}
    </div>
  {:else if !hasActivity}
    <p class="digest-empty">No activity yet — it appears here after files sync.</p>
  {:else}
    <div class="digest-grid">
      <div class="digest-card">
        <header class="digest-card-head">
          <h3>Edits over time</h3>
          {#if activity.sparkline.length > 0}
            <Sparkline data={activity.sparkline} width={110} height={18} />
          {/if}
        </header>
        {#if activity.sparkline.length > 0}
          <div class="digest-bars" aria-label="Edits over time">
            {#each activity.sparkline as value, index (index)}
              <span class="digest-bar" style={`height: ${barHeight(value)}`} title={`${value} edits`}></span>
            {/each}
          </div>
        {/if}
      </div>

      <div class="digest-card">
        <header class="digest-card-head"><h3>Top contributors</h3></header>
        <div class="digest-contributors">
          {#each activity.top.slice(0, 4) as c, index (`${c.who}:${index}`)}
            <div class="digest-contributor">
              <div class="digest-contributor-body">
                <span class="digest-contributor-name">{c.who}</span>
                <span class="digest-track" aria-hidden="true">
                  <span class="digest-fill" style={`width: ${contributorWidth(c.edits)}`}></span>
                </span>
              </div>
              <strong>{c.edits}</strong>
            </div>
          {/each}
        </div>
      </div>
    </div>
  {/if}
</section>

<style>
  .digest {
    display: grid;
    gap: 12px;
  }
  .digest-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .digest-header h2 {
    margin: 0;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .digest-sub {
    color: var(--v4-text-3);
    font-size: var(--text-base);
  }
  .digest-stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
  }
  .digest-stat {
    display: grid;
    gap: 2px;
    padding: 10px 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
  }
  .digest-stat strong {
    color: var(--v4-text-1);
    font-size: var(--text-lg, 16px);
    font-weight: 600;
    line-height: 1.1;
  }
  .digest-stat span {
    color: var(--v4-text-3);
    font-size: var(--text-base);
  }
  .digest-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .digest-card {
    display: grid;
    gap: 10px;
    align-content: start;
    padding: 12px 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
  }
  .digest-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .digest-card-head h3 {
    margin: 0;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 500;
  }
  .digest-bars {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 48px;
  }
  .digest-bar {
    flex: 1 1 0;
    min-height: 2px;
    border-radius: 2px 2px 0 0;
    /* Neutral white-alpha to match the app's restrained Activity bars (no
       introduced accent colour). */
    background: rgba(255, 255, 255, 0.14);
  }
  .digest-contributors {
    display: grid;
    gap: 8px;
  }
  .digest-contributor {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .digest-contributor-body {
    display: grid;
    gap: 4px;
    flex: 1 1 auto;
    min-width: 0;
  }
  .digest-contributor-name {
    overflow: hidden;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .digest-track {
    height: 5px;
    border-radius: 999px;
    background: var(--v4-control-faint);
    overflow: hidden;
  }
  .digest-fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.32);
  }
  .digest-contributor strong {
    flex: 0 0 auto;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
  }
  .digest-empty {
    margin: 0;
    padding: 12px 14px;
    border: 1px dashed var(--v4-hairline);
    border-radius: 8px;
    color: var(--v4-text-3);
    font-size: var(--text-base);
  }
  .digest-skeleton {
    display: grid;
    gap: 8px;
    padding: 4px 0;
  }
  .digest-skeleton span {
    height: 10px;
    border-radius: 999px;
    background: var(--v4-control-faint);
    animation: digest-pulse 1.2s ease-in-out infinite;
  }
  @keyframes digest-pulse {
    0%, 100% { opacity: 0.5; }
    50% { opacity: 1; }
  }
  @container (max-width: 560px) {
    .digest-stats { grid-template-columns: repeat(2, 1fr); }
    .digest-grid { grid-template-columns: 1fr; }
  }
  @media (prefers-reduced-motion: reduce) {
    .digest-skeleton span { animation: none; }
  }
</style>
