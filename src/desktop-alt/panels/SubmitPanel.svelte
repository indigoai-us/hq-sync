<script lang="ts">
  /**
   * SubmitPanel — the desktop-alt **Submit** tab body (US-013).
   *
   * Lets a creator pick a local skill/worker directory and submit it to the
   * marketplace via the US-004 `hq publish` flow (shelled out by the Rust
   * `publish_marketplace_pack` command). It renders:
   *   • a native folder picker (`pick_pack_directory`) + Submit button,
   *   • streaming publish progress lines (`marketplace:publish-progress`),
   *   • inline validation errors (AC2), and the resulting `pending_review`
   *     listing id on success (AC2),
   *   • a request-access prompt for UNVERIFIED users (AC3).
   *
   * Verification gate (architecture note): hq-pro exposes NO cheap "am I a
   * verified creator?" GET — only an admin grant/revoke and a request-access
   * POST (US-011). So this panel is OPTIMISTIC: it always shows the Submit form,
   * runs the publish, and renders the server's not-verified outcome (the publish
   * 403 → `NOT_VERIFIED_CREATOR`, surfaced by the CLI as a clear error and
   * classified by the Rust command's `notVerified` flag) as the request-access
   * prompt. The prompt's button calls `request_creator_access` so the affordance
   * is actionable from this same surface. The server is the real authority — the
   * UI never claims verification it can't prove.
   *
   * Mirrors MarketplacePanel/ModerationPanel conventions: Svelte 5 runes, the
   * shared desktop-alt CSS variables, and explicit idle/busy/error/success states.
   */
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import {
    pickPackDirectory,
    publishMarketplacePack,
    requestCreatorAccess,
    toPublishError,
    type PublishResult,
  } from '../lib/marketplace';

  // ── State ──────────────────────────────────────────────────────────────────
  let selectedPath = $state<string | null>(null);
  let submitting = $state(false);
  let progress = $state<string[]>([]);
  /** Success outcome (listing id + pending_review status). */
  let result = $state<PublishResult | null>(null);
  /** Inline error for an ordinary validation / network failure (AC2). */
  let errorMessage = $state<string | null>(null);
  /** True when the failure is the verified-creator gate → show request-access. */
  let notVerified = $state(false);

  // Request-access sub-flow state (AC3).
  let requesting = $state(false);
  let requestNote = $state<string | null>(null);

  const canSubmit = $derived(!!selectedPath && !submitting);

  function resetOutcome(): void {
    result = null;
    errorMessage = null;
    notVerified = false;
    requestNote = null;
    progress = [];
  }

  async function choose(): Promise<void> {
    if (submitting) return;
    try {
      const picked = await pickPackDirectory();
      if (picked) {
        selectedPath = picked;
        resetOutcome();
      }
    } catch (err) {
      // A picker failure is non-fatal — surface it inline so the user can retry.
      errorMessage = err instanceof Error ? err.message : String(err);
    }
  }

  async function submit(): Promise<void> {
    if (!selectedPath || submitting) return;
    submitting = true;
    resetOutcome();
    try {
      result = await publishMarketplacePack(selectedPath);
    } catch (err) {
      const pe = toPublishError(err);
      if (pe.notVerified) {
        notVerified = true;
      } else {
        errorMessage = pe.message;
      }
    } finally {
      submitting = false;
    }
  }

  async function requestAccess(): Promise<void> {
    if (requesting) return;
    requesting = true;
    try {
      requestNote = await requestCreatorAccess(null);
    } catch (err) {
      requestNote = err instanceof Error ? err.message : String(err);
    } finally {
      requesting = false;
    }
  }

  // Stream publish progress lines from the Rust command for live feedback.
  onMount(() => {
    let unlisten: UnlistenFn | undefined;
    void (async () => {
      unlisten = await listen<{ stream?: string; line?: string }>(
        'marketplace:publish-progress',
        (event) => {
          const line = event.payload?.line;
          if (line) progress = [...progress, line];
        },
      );
    })();
    return () => unlisten?.();
  });

  /** Short, friendly basename for the selected path (full path shown as title). */
  const selectedName = $derived(
    selectedPath ? (selectedPath.split('/').filter(Boolean).pop() ?? selectedPath) : null,
  );
</script>

<div class="submit" data-testid="submit-panel">
  <header class="submit-head">
    <h2 class="submit-title">Submit a pack</h2>
    <p class="submit-sub">
      Publish one of your local skills or workers to the marketplace. Submissions
      enter review before they appear publicly.
    </p>
  </header>

  <!-- Picker + Submit (AC1) -->
  <section class="picker" data-testid="submit-picker">
    <div class="picker-row">
      <button
        type="button"
        class="btn btn-secondary"
        data-testid="submit-choose"
        onclick={choose}
        disabled={submitting}
      >
        {selectedPath ? 'Change folder…' : 'Choose folder…'}
      </button>

      {#if selectedPath}
        <span class="chosen" data-testid="submit-chosen" title={selectedPath}>
          {selectedName}
        </span>
      {:else}
        <span class="chosen muted">No folder selected</span>
      {/if}
    </div>

    <button
      type="button"
      class="btn btn-primary"
      data-testid="submit-publish"
      onclick={submit}
      disabled={!canSubmit}
    >
      {submitting ? 'Submitting…' : 'Submit for review'}
    </button>
  </section>

  <!-- Live progress -->
  {#if submitting && progress.length > 0}
    <section class="progress" data-testid="submit-progress" aria-live="polite">
      <pre>{progress.join('\n')}</pre>
    </section>
  {/if}

  <!-- Success: pending_review listing (AC2) -->
  {#if result}
    <section class="state-success" data-testid="submit-success" role="status">
      <p class="success-line">
        Submitted — listing
        <code data-testid="submit-listing-id">{result.listingId}</code>
        is now
        <span class="pill status-pending" data-testid="submit-status">{result.status}</span>.
      </p>
      <p class="success-sub">An Indigo moderator will review it shortly.</p>
    </section>
  {/if}

  <!-- Inline validation / generic error (AC2) -->
  {#if errorMessage}
    <section class="state-error" data-testid="submit-error" role="alert">
      <p class="error-title">Couldn’t submit</p>
      <p class="error-body">{errorMessage}</p>
    </section>
  {/if}

  <!-- Request-access prompt for unverified users (AC3) -->
  {#if notVerified}
    <section class="request-access" data-testid="submit-request-access" role="alert">
      <h3 class="ra-title">Verified creators only</h3>
      <p class="ra-body">
        Publishing to the marketplace is limited to verified creators right now.
        Request access and an Indigo admin will review it.
      </p>
      {#if requestNote}
        <p class="ra-note" data-testid="submit-request-note">{requestNote}</p>
      {:else}
        <button
          type="button"
          class="btn btn-primary"
          data-testid="submit-request-access-button"
          onclick={requestAccess}
          disabled={requesting}
        >
          {requesting ? 'Requesting…' : 'Request access'}
        </button>
      {/if}
    </section>
  {/if}
</div>

<style>
  .submit {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .submit-head {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .submit-title {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-lg, 15px);
    font-weight: 700;
  }

  .submit-sub {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
    max-width: 56ch;
  }

  /* ---- picker row -------------------------------------------------------- */
  .picker {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
  }

  .picker-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .chosen {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 280px;
  }

  .chosen.muted {
    color: var(--muted-3);
    font-weight: 500;
  }

  /* ---- buttons ----------------------------------------------------------- */
  .btn {
    display: inline-flex;
    align-items: center;
    height: 32px;
    padding: 0 var(--space-3);
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      opacity 140ms ease;
  }

  .btn:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .btn-primary {
    border-color: color-mix(in srgb, var(--blue) 55%, transparent);
    background: var(--blue);
    color: #fff;
  }

  .btn-primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--blue) 88%, #000);
  }

  .btn-secondary {
    background: var(--bg);
  }

  /* ---- progress ---------------------------------------------------------- */
  .progress {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    max-height: 220px;
    overflow: auto;
  }

  .progress pre {
    margin: 0;
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro, 11px);
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* ---- success ----------------------------------------------------------- */
  .state-success {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3);
    border: 1px solid color-mix(in srgb, var(--emerald) 45%, transparent);
    border-radius: 4px;
    background: color-mix(in srgb, var(--emerald) 10%, transparent);
  }

  .success-line {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
  }

  .success-line code {
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--row-active);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro, 11px);
  }

  .success-sub {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
  }

  .pill {
    display: inline-flex;
    align-items: center;
    padding: 1px 7px;
    border-radius: 999px;
    font-size: var(--text-micro, 11px);
    font-weight: 700;
  }

  .status-pending {
    background: color-mix(in srgb, var(--amber) 22%, transparent);
    color: var(--amber);
  }

  /* ---- error ------------------------------------------------------------- */
  .state-error {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3);
    border: 1px solid color-mix(in srgb, var(--red, #e5484d) 45%, var(--border));
    border-radius: 4px;
    background: color-mix(in srgb, var(--red, #e5484d) 10%, transparent);
  }

  .error-title {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 700;
  }

  .error-body {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* ---- request access (unverified) -------------------------------------- */
  .request-access {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
    padding: var(--space-4);
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    background: var(--row-active);
  }

  .ra-title {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 700;
  }

  .ra-body {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
    max-width: 56ch;
  }

  .ra-note {
    margin: 0;
    color: var(--emerald);
    font-size: var(--text-base);
    font-weight: 600;
  }

  @media (prefers-reduced-motion: reduce) {
    .btn {
      transition: none;
    }
  }
</style>
