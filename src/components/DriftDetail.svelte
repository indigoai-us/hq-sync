<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { open as openInBrowser } from '@tauri-apps/plugin-shell';
  import CopyPromptButton from './CopyPromptButton.svelte';
  import type { Issue } from '../lib/copy-prompts';

  // Mirrors `DriftEntry` / `DriftReport` from src-tauri/src/commands/hq_core_drift.rs.
  // Camel-cased by serde's `rename_all = "camelCase"`.
  interface DriftEntry {
    path: string;
    size: number;
    gitShaLocal: string | null;
    gitShaUpstream: string | null;
    // Staging-classification tag (@getindigo.ai builders only). Absent for
    // ineligible users — see src-tauri/src/commands/hq_core_staging.rs.
    // Wire forms: 'staging-main' | 'pr:182' | 'unaccounted'.
    stagingStatus?: string | null;
  }
  interface DriftReport {
    count: number;
    modified: DriftEntry[];
    missing: DriftEntry[];
    added: DriftEntry[];
    scannedAt: string;
    hqVersion: string;
  }

  let report = $state<DriftReport | null>(null);
  // Per-row in-flight + result state for the Restore button. Keyed by
  // `${kind}:${path}` because Modified and Added cases use the same path
  // for different intents in principle (Added never restores, but the
  // composite key keeps the map shape consistent).
  let restoreState = $state<Record<string, 'idle' | 'in-flight' | 'done' | string>>({});

  // Header/help baseline label. Release reports carry a bare semver
  // (`14.2.1`) that reads naturally with a `v` prefix; staging reports
  // carry an `owner/repo@ref` string (e.g. `indigoai-us/hq-core-staging@main`)
  // where a `v` prefix produces the nonsense `vindigoai-us/...`. The `@` is
  // the unambiguous tell (same one `recheck()` routes on), so prefix `v`
  // only for release reports.
  const versionLabel = $derived(
    report ? (report.hqVersion.includes('@') ? report.hqVersion : `v${report.hqVersion}`) : '',
  );

  // Manual recheck. The background loop only re-scans every 6h (see
  // hq_core_drift.rs CHECK_INTERVAL), so after resolving drift — or when
  // staging PRs have moved — the report on screen can be stale. This
  // button forces a fresh check; both Rust commands re-emit `drift:report`
  // back to this window so the $effect listener applies it + resets
  // restoreState, updating the window in place.
  //
  // Route by report origin so we don't re-check release drift when the
  // window is showing staging drift (or vice versa). Staging reports carry
  // an `owner/repo@ref`-shaped `hqVersion`; release reports are a bare
  // version string like `14.2.1`. The `@` is the unambiguous tell.
  let rechecking = $state(false);
  async function recheck() {
    if (rechecking) return;
    rechecking = true;
    try {
      const isStaging = !!report?.hqVersion?.includes('@');
      const cmd = isStaging ? 'check_staging_drift' : 'check_hq_core_drift';
      await invoke(cmd);
    } catch (e) {
      console.error('recheck failed:', e);
    } finally {
      rechecking = false;
    }
  }

  // One bulk "Copy prompt for all" issue covering every drifted file, so the
  // user can resolve the whole report in a single agent session. `isBuilder`
  // is inferred from the presence of any staging classification — staging
  // tags only appear for @getindigo.ai builders, so a report with no tags is
  // treated as a regular user (who gets the personal/-overlay framing, with
  // no mention of hq-core-staging they can't access).
  const allDriftIssue = $derived.by<Issue | null>(() => {
    if (!report || report.count === 0) return null;
    const files = [
      ...report.modified.map((e) => ({ path: e.path, kind: 'modified', staging: e.stagingStatus ?? null })),
      ...report.missing.map((e) => ({ path: e.path, kind: 'missing', staging: null })),
      ...report.added.map((e) => ({ path: e.path, kind: 'added', staging: e.stagingStatus ?? null })),
    ];
    const isBuilder = files.some((f) => f.staging != null);
    return {
      kind: 'hq-core-drift-all',
      payload: { hqVersion: report.hqVersion, isBuilder, files },
    };
  });

  // Render a staging-classification tag into a short badge label. Returns
  // null when there's no status (ineligible user / unclassified) so the
  // badge is simply omitted.
  function stagingLabel(status: string | null | undefined): string | null {
    if (!status) return null;
    if (status === 'staging-main') return 'staging main';
    if (status === 'unaccounted') return 'unaccounted';
    const pr = status.startsWith('pr:') ? status.slice(3) : null;
    return pr ? `PR #${pr}` : status;
  }

  // Coarse variant used for badge colour. 'main' = settled (green),
  // 'pr' = in-flight (blue), 'unaccounted' = needs action (amber).
  function stagingVariant(
    status: string | null | undefined,
  ): 'main' | 'pr' | 'unaccounted' | null {
    if (!status) return null;
    if (status === 'staging-main') return 'main';
    if (status === 'unaccounted') return 'unaccounted';
    if (status.startsWith('pr:')) return 'pr';
    return null;
  }

  function stagingTitle(status: string | null | undefined): string {
    return status === 'unaccounted'
      ? 'Not yet in hq-core-staging (main or any open PR) — a real, unpromoted edit'
      : `Already in hq-core-staging (${stagingLabel(status)}) — waiting for the next release`;
  }

  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function upstreamBlobUrl(entry: DriftEntry): string {
    return `https://github.com/indigoai-us/hq-core/blob/v${report?.hqVersion ?? 'main'}/${entry.path}`;
  }

  async function viewUpstream(entry: DriftEntry) {
    try {
      await openInBrowser(upstreamBlobUrl(entry));
    } catch (e) {
      console.error('open upstream failed:', e);
    }
  }

  async function openLocal(entry: DriftEntry) {
    // Delegate to the existing conflict-resolution editor opener — it
    // already handles `code`/`subl`/`open -t` fallbacks. Path is
    // relative to the HQ root, so the Rust side prefixes with the
    // resolved HQ folder.
    try {
      await invoke('open_in_editor', { path: entry.path });
    } catch (e) {
      console.error('open_in_editor failed:', e);
    }
  }

  async function restore(entry: DriftEntry, kind: 'modified' | 'missing') {
    const key = `${kind}:${entry.path}`;
    // Browser confirm() lives inside the webview — fine for a
    // diagnostic window. A native dialog would be nicer; keeping it
    // simple here so the destructive-action surface ships in one pass.
    const ok = confirm(
      `Restore ${entry.path} from upstream v${report?.hqVersion}?\n\n` +
      (kind === 'missing'
        ? 'This will create the file with the upstream content.'
        : 'This will overwrite your local edits with the upstream content. Cannot be undone except by re-editing.'),
    );
    if (!ok) return;
    restoreState = { ...restoreState, [key]: 'in-flight' };
    try {
      await invoke('restore_from_upstream', {
        path: entry.path,
        expectedUpstreamSha: entry.gitShaUpstream,
      });
      restoreState = { ...restoreState, [key]: 'done' };
    } catch (e) {
      const msg = String(e);
      console.error('restore_from_upstream failed:', msg);
      restoreState = { ...restoreState, [key]: msg };
    }
  }

  $effect(() => {
    let unlisten: (() => void) | undefined;
    listen<DriftReport>('drift:report', (event) => {
      report = event.payload;
      // Reset per-row state on a fresh report (e.g. after a Refresh
      // button or background re-check) so done/error flags don't bleed
      // across scans.
      restoreState = {};
    }).then((fn) => {
      unlisten = fn;
      // Race-free handshake: Rust waits for this invoke before emitting
      // the report + showing the window. Same pattern as new-files.
      invoke('drift_window_ready');
    });
    return () => {
      unlisten?.();
    };
  });
</script>

<!-- ── Reusable row pieces (snippets) ──────────────────────────────────────── -->
{#snippet stagingBadge(entry: DriftEntry)}
  {#if stagingVariant(entry.stagingStatus)}
    <span
      class="drift-staging-badge"
      class:is-main={stagingVariant(entry.stagingStatus) === 'main'}
      class:is-pr={stagingVariant(entry.stagingStatus) === 'pr'}
      class:is-unaccounted={stagingVariant(entry.stagingStatus) === 'unaccounted'}
      title={stagingTitle(entry.stagingStatus)}
    >{stagingLabel(entry.stagingStatus)}</span>
  {/if}
{/snippet}

<!-- One flat row: path (truncates, full text on hover via title) · badge ·
     size — with the action cluster overlaid on the right, revealed on row
     hover / keyboard focus so the resting state stays calm. -->
{#snippet driftRow(
  entry: DriftEntry,
  sizeText: string,
  actions: import('svelte').Snippet<[DriftEntry]>,
  errKey?: string,
)}
  <div class="drift-row">
    <div class="drift-row-line">
      <span class="drift-row-path" title={entry.path}>{entry.path}</span>
      <div class="drift-row-end">
        <span class="drift-meta-cluster">
          {@render stagingBadge(entry)}
          <span class="drift-row-size">{sizeText}</span>
        </span>
        <div class="drift-row-actions">{@render actions(entry)}</div>
      </div>
    </div>
    {#if errKey}{@render restoreErr(errKey)}{/if}
  </div>
{/snippet}

{#snippet modifiedActions(entry: DriftEntry)}
  {@render openBtn(entry)}{@render viewBtn(entry)}{@render restoreBtn(entry, 'modified')}{@render reviewBtn(entry, 'modified')}
{/snippet}
{#snippet missingActions(entry: DriftEntry)}
  {@render viewBtn(entry)}{@render restoreBtn(entry, 'missing')}{@render reviewBtn(entry, 'missing')}
{/snippet}
{#snippet addedActions(entry: DriftEntry)}
  {@render openBtn(entry)}{@render reviewBtn(entry, 'added')}
{/snippet}

{#snippet openBtn(entry: DriftEntry)}
  <button
    class="drift-icon-btn"
    onclick={() => openLocal(entry)}
    title="Open the local file in your editor"
    aria-label="Open the local file in your editor"
  >
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <path d="M11 2.6l2.4 2.4M3 13l.6-2.5 7-7 1.9 1.9-7 7L3 13z" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>
{/snippet}

{#snippet viewBtn(entry: DriftEntry)}
  <button
    class="drift-icon-btn"
    onclick={() => viewUpstream(entry)}
    title="View the upstream version on GitHub"
    aria-label="View the upstream version on GitHub"
  >
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <path d="M9 3h4v4M13 3l-6.5 6.5M11.5 9.2v2.8a1.7 1.7 0 0 1-1.7 1.7H4A1.7 1.7 0 0 1 2.3 12V6.2A1.7 1.7 0 0 1 4 4.5h2.8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>
{/snippet}

{#snippet restoreBtn(entry: DriftEntry, kind: 'modified' | 'missing')}
  {@const key = `${kind}:${entry.path}`}
  {@const st = restoreState[key]}
  <button
    class="drift-icon-btn is-danger"
    class:is-done={st === 'done'}
    onclick={() => restore(entry, kind)}
    disabled={st === 'in-flight' || st === 'done'}
    title={st === 'done'
      ? 'Restored from upstream'
      : kind === 'missing'
      ? 'Create the file locally with the upstream content'
      : 'Overwrite the local file with the upstream content (destructive)'}
    aria-label="Restore from upstream"
  >
    {#if st === 'in-flight'}
      <span class="drift-mini-spinner" aria-hidden="true"></span>
    {:else if st === 'done'}
      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M3.5 8.4l3 3 6-6.6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    {:else}
      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M8 2.8v6.4M5.2 6.6L8 9.4l2.8-2.8M3 12.6h10" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    {/if}
  </button>
{/snippet}

{#snippet reviewBtn(entry: DriftEntry, kind: 'modified' | 'missing' | 'added')}
  <CopyPromptButton
    variant="compact"
    label="Review with agent"
    issue={{ kind: 'hq-core-drift', payload: { path: entry.path, kind, hqVersion: report?.hqVersion } }}
  />
{/snippet}

{#snippet restoreErr(key: string)}
  {#if typeof restoreState[key] === 'string' && restoreState[key] !== 'in-flight' && restoreState[key] !== 'done'}
    <p class="drift-row-error">Restore failed: {restoreState[key]}</p>
  {/if}
{/snippet}

<div class="drift-window">
  <!-- Compact title bar — sits in the overlay zone next to the traffic
       lights, ~28px tall (matches native macOS title-bar height). No
       border-bottom: the section structure below carries its own visual
       grouping, and a hard line under the title clashes with the
       continuous backdrop blur. -->
  <header class="drift-header">
    <div class="drift-title-block">
      <h1>Core Drift</h1>
      {#if report}
        <span class="drift-meta">
          {versionLabel} · {report.count} file{report.count === 1 ? '' : 's'} differ
        </span>
      {/if}
    </div>
    <div class="drift-header-actions">
      {#if allDriftIssue}
        <CopyPromptButton variant="inline" label="Copy prompt for all" issue={allDriftIssue} />
      {/if}
      <!-- Recheck stays available even at zero drift, so the user can
           confirm a reconciliation landed without waiting up to 6h for
           the background loop. -->
      <button
        class="drift-recheck-btn"
        onclick={recheck}
        disabled={rechecking}
        title="Re-scan locked core files against upstream now"
      >
        {#if rechecking}
          <span class="drift-mini-spinner" aria-hidden="true"></span>
          Rechecking…
        {:else}
          Recheck
        {/if}
      </button>
    </div>
  </header>

  {#if !report}
    <!-- Initial load: managed-state handshake hasn't fired yet (or is
         in flight). Centered spinner + label rather than a stray
         "Loading…" string against an empty body. Matches the popover
         header-sync-spinner so the visual language is consistent across
         windows. -->
    <div class="drift-loading">
      <span class="drift-spinner" aria-hidden="true"></span>
      <p>Scanning locked core files…</p>
    </div>
  {:else if report.count === 0}
    <div class="drift-empty">
      <span class="drift-empty-check" aria-hidden="true">
        <svg width="32" height="32" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="8" cy="8" r="7" stroke="currentColor" stroke-width="1.2" opacity="0.4" />
          <path d="M5 8.2l2.2 2.2L11 6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </span>
      <p>No drift detected. Locked core files match upstream {versionLabel}.</p>
    </div>
  {:else}
    <div class="drift-body">
      {#if report.modified.length > 0}
        <section class="drift-section">
          <h2 class="drift-section-title">
            Modified <span class="drift-section-count">{report.modified.length}</span>
          </h2>
          <p class="drift-section-desc">
            Local content differs from what {versionLabel} shipped. Either restore upstream, or
            review with an agent to decide if the edit should be lifted into a <code>personal/</code> overlay.
          </p>
          {#each report.modified as entry}
            {@render driftRow(entry, formatBytes(entry.size), modifiedActions, `modified:${entry.path}`)}
          {/each}
        </section>
      {/if}

      {#if report.missing.length > 0}
        <section class="drift-section">
          <h2 class="drift-section-title">
            Missing <span class="drift-section-count">{report.missing.length}</span>
          </h2>
          <p class="drift-section-desc">
            Upstream ships these but they're absent locally. Likely deleted by hand or never extracted.
          </p>
          {#each report.missing as entry}
            {@render driftRow(entry, `${formatBytes(entry.size)} upstream`, missingActions, `missing:${entry.path}`)}
          {/each}
        </section>
      {/if}

      {#if report.added.length > 0}
        <section class="drift-section">
          <h2 class="drift-section-title">
            Added <span class="drift-section-count">{report.added.length}</span>
          </h2>
          <p class="drift-section-desc">
            Local files under a locked-path scope that aren't part of upstream. No Restore — upstream
            has nothing to write back. Review with an agent to decide if they should move out of the
            locked scope (e.g. into <code>personal/</code>).
          </p>
          {#each report.added as entry}
            {@render driftRow(entry, `${formatBytes(entry.size)} local`, addedActions)}
          {/each}
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* Scoped via `data-window` (set in main.ts) so this reset can't bleed
     into other windows. Default UA `body { margin: 8px }` was leaving a
     grey gutter on the top + left edges where the OS window backing showed
     through; zero it out and keep the backing transparent so only
     `.drift-window` paints. */
  :global(html[data-window='drift-detail']),
  :global(html[data-window='drift-detail'] body) {
    margin: 0;
    padding: 0;
    height: 100vh;
    background: transparent;
    overflow: hidden;
  }
  :global(html[data-window='drift-detail'] #app) {
    height: 100vh;
  }

  .drift-window {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    background: var(--popover-bg, rgba(18, 18, 20, 0.68));
    color: var(--popover-text, #e0e0e0);
    font-family: system-ui, -apple-system, BlinkMacSystemFont, sans-serif;
    overflow: hidden;
    /* Three-step type scale — every text element below picks one of these
       so sizes stay consistent: 13px headings, 12px body/paths, 11px meta. */
    --fs-lg: 0.8125rem;
    --fs-md: 0.75rem;
    --fs-sm: 0.6875rem;
  }

  .drift-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    /* Left padding reserves space for the macOS traffic-light buttons
       (overlaid title bar — see drift_detail.rs WebviewWindowBuilder).
       The buttons live at ~12px x 12px starting at ~7px from the left,
       spanning to ~70px total. Padding pushes the title block clear
       with a 12px gutter. */
    padding: 0 1rem 0 5.25rem;
    /* Native macOS title-bar height. Heading sits vertically centered
       with the traffic lights so the chrome reads as one row, not two
       stacked. */
    height: 38px;
    flex-shrink: 0;
    /* Header is draggable so users can move the window by grabbing the
       title-bar zone — matches native macOS behaviour. */
    -webkit-app-region: drag;
  }

  /* Re-enable click capture on anything interactive inside the draggable
     header (currently nothing — the header is text-only — but future
     refresh / close buttons would need this). */
  .drift-header :global(button),
  .drift-header :global(a) {
    -webkit-app-region: no-drag;
  }

  /* Bulk "Copy prompt for all" lives at the right of the title bar. The
     shared CopyPromptButton's inline variant already matches our pill sizing;
     just keep it from shrinking and from being swallowed by the drag region. */
  .drift-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }

  /* Text pill (not the round icon-btn) — sits beside "Copy prompt for all"
     in the header. Same neutral-at-rest / brighten-on-hover palette as
     .drift-icon-btn so the header action cluster reads as one family. */
  .drift-recheck-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    padding: 4px 10px;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.14));
    border-radius: 6px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.06));
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.6));
    font-size: var(--fs-sm);
    font-weight: 500;
    line-height: 1;
    cursor: pointer;
    transition: background-color 0.12s ease, color 0.12s ease, border-color 0.12s ease,
      opacity 0.12s ease;
  }

  .drift-recheck-btn:hover:not(:disabled) {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text-heading, #ffffff);
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.3));
  }

  .drift-recheck-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  /* Title + meta stacked tightly so the whole block fits inside the
     38px title-bar zone. Heading uses 13px (canonical primary size),
     meta uses 11px (canonical micro). No vertical gap — line-height
     handles the rhythm. */
  .drift-title-block {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.15;
  }

  .drift-header h1 {
    font-size: var(--fs-lg);
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    margin: 0;
  }

  .drift-meta {
    font-size: var(--fs-sm);
    color: var(--popover-text-muted, #a0a0b0);
  }

  /* Loading + empty share centered-column layout — different inner content
     (spinner vs. check icon) but the same calm framing so the window
     doesn't feel jarring as it transitions from loading → resolved. */
  .drift-loading,
  .drift-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    gap: 0.75rem;
    padding: 2rem;
    color: var(--popover-text-muted, #a0a0b0);
    font-size: var(--fs-lg);
    text-align: center;
  }

  .drift-loading p,
  .drift-empty p {
    margin: 0;
    max-width: 36ch;
    line-height: 1.45;
  }

  /* Spinner — visual language matched to the popover `.header-sync-spinner`
     (white ring with single coloured top arc, 0.6s linear rotate) so the
     two surfaces feel like one app. Slightly larger here because we have
     the room and it's the focal element of the loading state. */
  .drift-spinner {
    width: 28px;
    height: 28px;
    border: 2.5px solid var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    border-top-color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border-radius: 50%;
    animation: drift-spin 0.7s linear infinite;
  }

  @keyframes drift-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Empty-state checkmark — muted text colour, no green-success accent
     (consistent with the menubar's no-severity-colour stance). The
     ring's lower opacity carries the "calm, just confirming" tone. */
  .drift-empty-check {
    color: var(--popover-text-muted, #a0a0b0);
  }

  .drift-body {
    flex: 1;
    overflow-y: auto;
    padding: 0.75rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .drift-section {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .drift-section-title {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .drift-section-count {
    font-size: var(--fs-sm);
    font-weight: 500;
    padding: 0.125rem 0.5rem;
    border-radius: 999px;
    line-height: 1;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    color: var(--popover-text-muted, #a0a0b0);
  }

  .drift-section-desc {
    margin: 0 0 0.125rem 0;
    font-size: var(--fs-sm);
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.4;
  }

  .drift-section-desc code {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: var(--fs-sm);
    padding: 0.0625rem 0.25rem;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border-radius: 3px;
  }

  /* Flat rows — no card chrome. One line per file; the optional restore-error
     note stacks beneath. Separation comes from the section gap + row height,
     not a boxed background. */
  .drift-row {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .drift-row-line {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    min-width: 0;
    min-height: 28px;
    padding: 0 0.125rem;
  }

  .drift-row-path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: var(--fs-md);
    color: var(--popover-text, #e0e0e0);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  /* Right cluster: badge + size, then the inline action row. */
  .drift-row-end {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    flex-shrink: 0;
  }

  .drift-meta-cluster {
    display: inline-flex;
    align-items: center;
    gap: 0.4375rem;
    font-size: var(--fs-md);
    color: var(--popover-text-muted, #a0a0b0);
  }

  .drift-row-size {
    font-variant-numeric: tabular-nums;
  }

  /* Staging-classification badge. Tinted by pipeline state so the eye can
     triage at a glance: green = settled on staging main, blue = in an open
     PR, amber = unaccounted (the only real action item). Low-saturation
     fills tuned for the dark glass backdrop. */
  .drift-staging-badge {
    font-size: var(--fs-md);
    font-weight: 600;
    padding: 0.125rem 0.5rem;
    border-radius: 999px;
    line-height: 1;
    letter-spacing: 0.01em;
    white-space: nowrap;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    color: var(--popover-text-muted, #a0a0b0);
  }

  .drift-staging-badge.is-main {
    color: #7ee0b8;
    background: rgba(52, 211, 153, 0.14);
  }

  .drift-staging-badge.is-pr {
    color: #9cc7ff;
    background: rgba(96, 165, 250, 0.16);
  }

  .drift-staging-badge.is-unaccounted {
    color: #f6c560;
    background: rgba(245, 180, 70, 0.16);
  }

  /* Actions sit inline at the right edge, dim at rest and lift to full on
     row hover / keyboard focus so a wall of buttons doesn't shout when the
     user is just scanning. */
  .drift-row-actions {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    flex-shrink: 0;
    opacity: 0.38;
    transition: opacity 0.12s ease;
  }

  .drift-row-line:hover .drift-row-actions,
  .drift-row-line:focus-within .drift-row-actions {
    opacity: 1;
  }

  /* Circular icon buttons — label lives in the tooltip (title/aria-label).
     Compact + consistent so a dense row stays calm. */
  .drift-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    flex-shrink: 0;
    padding: 0;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.14));
    border-radius: 50%;
    background: var(--popover-surface, rgba(255, 255, 255, 0.06));
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.6));
    cursor: pointer;
    transition: background-color 0.12s ease, color 0.12s ease, border-color 0.12s ease,
      opacity 0.12s ease;
  }

  .drift-icon-btn:hover:not(:disabled) {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text-heading, #ffffff);
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.3));
  }

  .drift-icon-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  /* Restore is destructive — stays neutral at rest, warms to red only on
     hover so the danger reads at the moment of intent, not as constant noise. */
  .drift-icon-btn.is-danger:hover:not(:disabled) {
    color: #fca5a5;
    border-color: rgba(248, 113, 113, 0.5);
    background: rgba(248, 113, 113, 0.12);
  }

  /* Post-restore confirmation — green check, disabled. */
  .drift-icon-btn.is-done {
    color: #7ee0b8;
    border-color: rgba(110, 231, 183, 0.45);
    opacity: 1;
  }

  .drift-mini-spinner {
    width: 13px;
    height: 13px;
    border: 2px solid var(--popover-surface-strong, rgba(255, 255, 255, 0.18));
    border-top-color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border-radius: 50%;
    animation: drift-spin 0.7s linear infinite;
  }

  /* Make the shared CopyPromptButton's compact variant match the circular
     icon buttons — scoped under `.drift-row-actions` so other surfaces
     (SyncStats) keep its rounded-square look. */
  .drift-row-actions :global(.copy-prompt-btn.compact) {
    width: 26px;
    height: 26px;
    padding: 0;
    border-radius: 50%;
    justify-content: center;
  }

  .drift-row-error {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--popover-danger, #ef4444);
    line-height: 1.3;
  }
</style>
