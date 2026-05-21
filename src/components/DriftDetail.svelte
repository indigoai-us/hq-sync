<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { open as openInBrowser } from '@tauri-apps/plugin-shell';
  import CopyPromptButton from './CopyPromptButton.svelte';

  // Mirrors `DriftEntry` / `DriftReport` from src-tauri/src/commands/hq_core_drift.rs.
  // Camel-cased by serde's `rename_all = "camelCase"`.
  interface DriftEntry {
    path: string;
    size: number;
    gitShaLocal: string | null;
    gitShaUpstream: string | null;
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
          v{report.hqVersion} · {report.count} file{report.count === 1 ? '' : 's'} differ
        </span>
      {/if}
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
      <p>No drift detected. Locked core files match upstream v{report.hqVersion}.</p>
    </div>
  {:else}
    <div class="drift-body">
      {#if report.modified.length > 0}
        <section class="drift-section">
          <h2 class="drift-section-title">
            Modified <span class="drift-section-count">{report.modified.length}</span>
          </h2>
          <p class="drift-section-desc">
            Local content differs from what v{report.hqVersion} shipped. Either restore upstream, or
            review with an agent to decide if the edit should be lifted into a <code>personal/</code> overlay.
          </p>
          {#each report.modified as entry}
            {@const key = `modified:${entry.path}`}
            <div class="drift-row">
              <div class="drift-row-main">
                <span class="drift-row-path" title={entry.path}>{entry.path}</span>
                <span class="drift-row-meta">{formatBytes(entry.size)}</span>
              </div>
              <div class="drift-row-actions">
                <button class="drift-action" onclick={() => openLocal(entry)} title="Open the local file in your editor">
                  Open local
                </button>
                <button class="drift-action" onclick={() => viewUpstream(entry)} title="View the upstream version on GitHub">
                  View upstream
                </button>
                <button
                  class="drift-action drift-action-danger"
                  onclick={() => restore(entry, 'modified')}
                  disabled={restoreState[key] === 'in-flight' || restoreState[key] === 'done'}
                  title="Overwrite the local file with the upstream content (destructive)"
                >
                  {restoreState[key] === 'in-flight'
                    ? 'Restoring…'
                    : restoreState[key] === 'done'
                    ? 'Restored'
                    : 'Restore'}
                </button>
                <CopyPromptButton
                  variant="compact"
                  label="Review with agent"
                  issue={{ kind: 'hq-core-drift', payload: { path: entry.path, kind: 'modified', hqVersion: report.hqVersion } }}
                />
              </div>
              {#if typeof restoreState[key] === 'string' && restoreState[key] !== 'in-flight' && restoreState[key] !== 'done'}
                <p class="drift-row-error">Restore failed: {restoreState[key]}</p>
              {/if}
            </div>
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
            {@const key = `missing:${entry.path}`}
            <div class="drift-row">
              <div class="drift-row-main">
                <span class="drift-row-path" title={entry.path}>{entry.path}</span>
                <span class="drift-row-meta">{formatBytes(entry.size)} upstream</span>
              </div>
              <div class="drift-row-actions">
                <button class="drift-action" onclick={() => viewUpstream(entry)} title="View the upstream version on GitHub">
                  View upstream
                </button>
                <button
                  class="drift-action drift-action-danger"
                  onclick={() => restore(entry, 'missing')}
                  disabled={restoreState[key] === 'in-flight' || restoreState[key] === 'done'}
                  title="Create the file locally with upstream content"
                >
                  {restoreState[key] === 'in-flight'
                    ? 'Restoring…'
                    : restoreState[key] === 'done'
                    ? 'Restored'
                    : 'Restore'}
                </button>
                <CopyPromptButton
                  variant="compact"
                  label="Review with agent"
                  issue={{ kind: 'hq-core-drift', payload: { path: entry.path, kind: 'missing', hqVersion: report.hqVersion } }}
                />
              </div>
              {#if typeof restoreState[key] === 'string' && restoreState[key] !== 'in-flight' && restoreState[key] !== 'done'}
                <p class="drift-row-error">Restore failed: {restoreState[key]}</p>
              {/if}
            </div>
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
            <div class="drift-row">
              <div class="drift-row-main">
                <span class="drift-row-path" title={entry.path}>{entry.path}</span>
                <span class="drift-row-meta">{formatBytes(entry.size)} local</span>
              </div>
              <div class="drift-row-actions">
                <button class="drift-action" onclick={() => openLocal(entry)} title="Open the local file in your editor">
                  Open local
                </button>
                <CopyPromptButton
                  variant="compact"
                  label="Review with agent"
                  issue={{ kind: 'hq-core-drift', payload: { path: entry.path, kind: 'added', hqVersion: report.hqVersion } }}
                />
              </div>
            </div>
          {/each}
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
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
  }

  .drift-header {
    display: flex;
    align-items: center;
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
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    margin: 0;
  }

  .drift-meta {
    font-size: 0.6875rem;
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
    font-size: 0.8125rem;
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
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .drift-section-count {
    font-size: 0.6875rem;
    font-weight: 500;
    padding: 0.0625rem 0.375rem;
    border-radius: 999px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    color: var(--popover-text-muted, #a0a0b0);
  }

  .drift-section-desc {
    margin: 0 0 0.25rem 0;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.4;
  }

  .drift-section-desc code {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.625rem;
    padding: 0.0625rem 0.25rem;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border-radius: 3px;
  }

  .drift-row {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.625rem;
    border-radius: 6px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.05));
  }

  .drift-row-main {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    min-width: 0;
  }

  .drift-row-path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.75rem;
    color: var(--popover-text, #e0e0e0);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  .drift-row-meta {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    flex-shrink: 0;
  }

  .drift-row-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem;
  }

  .drift-action {
    font-family: inherit;
    font-size: 0.6875rem;
    font-weight: 500;
    padding: 0.1875rem 0.5rem;
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border: none;
    border-radius: 999px;
    cursor: pointer;
    white-space: nowrap;
    transition: background-color 0.1s ease, color 0.1s ease, opacity 0.1s ease;
  }

  .drift-action:hover:not(:disabled) {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text-heading, #ffffff);
  }

  .drift-action:disabled {
    opacity: 0.6;
    cursor: default;
  }

  /* Destructive action (Restore overwrites local) — same surface treatment
     as the rest, danger semantic conveyed via icon-less label + a confirm
     dialog at click time. No red — the menubar's notice-tone language
     intentionally avoids severity colour. */
  .drift-action-danger {
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
  }

  .drift-row-error {
    margin: 0;
    font-size: 0.6875rem;
    color: var(--popover-danger, #ef4444);
    line-height: 1.3;
  }
</style>
