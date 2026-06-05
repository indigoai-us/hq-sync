<script lang="ts">
  /**
   * ModerationPanel — the desktop-alt moderation surface (US-022 launch-critical
   * emergency kill switch).
   *
   * MINIMAL by design: this is the SCAFFOLD that US-012 grows into the full
   * moderation queue UI. For launch it ships exactly ONE capability — the
   * emergency YANK / takedown action — because no public marketplace ships
   * without a kill switch. It accepts a target listing (id + display name; in
   * v1 the admin pastes/passes a listing id, US-012 will replace this with the
   * live queue list) and exposes a reason-gated, confirm-gated Yank action.
   *
   * The Yank action calls the `yank_marketplace_listing` Tauri command, which
   * forwards an AUTHED `POST /v1/moderation/listings/{id}/yank` to hq-pro. The
   * admin gate is enforced SERVER-SIDE (`@getindigo.ai` id_token) — a non-admin
   * gets a 403, never a yank. This panel never makes its own authz decision.
   *
   * A yank is a runtime status flip on the server (NO deploy): the listing
   * leaves the public `approved#<type>` partition instantly, so it disappears
   * from public browse + detail and `hq install` refuses it.
   *
   * V1 LIMITATION (AC3): a yank removes the listing from the directory and
   * refuses NEW installs, but users who ALREADY installed the pack are NOT
   * auto-removed (there is no remote uninstall in v1). This is surfaced to the
   * admin BOTH before the action (a standing note) and in the success result.
   *
   * Mirrors MarketplacePanel.svelte (US-008) conventions: Svelte 5 runes, the
   * shared desktop-alt CSS variables, the same slide-over-free flat layout.
   * Structured so US-012 can drop a queue list above the action without
   * reworking the yank flow.
   */
  import {
    yankMarketplaceListing,
    type YankResult,
  } from '../lib/marketplace';

  /**
   * The listing to act on. US-012 sets this from the live queue selection; for
   * the minimal launch panel the host passes a `{ id, name }` (id is the only
   * load-bearing field — name is display-only).
   */
  let {
    listing = null,
  }: {
    listing?: { id: string; name?: string } | null;
  } = $props();

  // In the minimal launch panel the admin can also type/paste an id directly,
  // so the kill switch works even before US-012's queue exists.
  let manualId = $state('');
  const targetId = $derived((listing?.id ?? manualId).trim());
  const targetLabel = $derived(listing?.name?.trim() || targetId || '—');

  let reason = $state('');
  let confirmArmed = $state(false);
  let yanking = $state(false);
  let result = $state<YankResult | null>(null);
  let error = $state<string | null>(null);

  // The standing v1-limitation note, shown BEFORE the action so the admin knows
  // the blast radius (AC3 surfacing). The server also returns this on success.
  const ALREADY_INSTALLED_NOTE =
    'Already-installed users are NOT auto-removed in v1. Yanking removes the ' +
    'listing from the directory and refuses new installs, but anyone who ' +
    'already installed this pack keeps it until they remove it themselves.';

  const canYank = $derived(
    targetId.length > 0 && reason.trim().length > 0 && !yanking,
  );

  function armConfirm(): void {
    error = null;
    if (targetId.length === 0) {
      error = 'Enter a listing id to yank.';
      return;
    }
    if (reason.trim().length === 0) {
      error = 'A reason is required to yank a listing.';
      return;
    }
    confirmArmed = true;
  }

  function cancelConfirm(): void {
    confirmArmed = false;
  }

  async function runYank(): Promise<void> {
    if (!canYank) return;
    yanking = true;
    error = null;
    result = null;
    try {
      result = await yankMarketplaceListing(targetId, reason.trim());
      confirmArmed = false;
      // Clear the reason so a second accidental submit can't re-fire silently.
      reason = '';
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
      confirmArmed = false;
    } finally {
      yanking = false;
    }
  }
</script>

<div class="moderation" data-testid="moderation-panel">
  <header class="head">
    <h2 class="title">Moderation</h2>
    <p class="subtitle">
      Emergency takedown. Yanking a listing pulls it from public browse, detail,
      and install immediately — a runtime change, no deploy.
    </p>
  </header>

  <section class="section" data-testid="moderation-yank-section">
    <h3 class="section-title">Yank a listing</h3>

    {#if listing}
      <p class="target" data-testid="moderation-target">
        Target: <strong>{targetLabel}</strong>
        <span class="target-id">{targetId}</span>
      </p>
    {:else}
      <label class="field-label" for="moderation-listing-id">Listing id</label>
      <input
        id="moderation-listing-id"
        class="text-input"
        type="text"
        placeholder="lst_…"
        autocomplete="off"
        spellcheck="false"
        data-testid="moderation-listing-id"
        bind:value={manualId}
        disabled={yanking}
      />
    {/if}

    <label class="field-label" for="moderation-reason">Reason (required)</label>
    <input
      id="moderation-reason"
      class="text-input"
      type="text"
      placeholder="Why is this being taken down?"
      autocomplete="off"
      data-testid="moderation-reason"
      bind:value={reason}
      disabled={yanking}
    />

    <!-- AC3: the already-installed limitation is surfaced BEFORE the action. -->
    <p class="limitation-note" data-testid="moderation-limitation-note">
      {ALREADY_INSTALLED_NOTE}
    </p>

    {#if !confirmArmed}
      <button
        type="button"
        class="yank-button"
        data-testid="moderation-yank-button"
        disabled={!canYank}
        onclick={armConfirm}
      >
        Yank listing
      </button>
    {:else}
      <div class="confirm-row" data-testid="moderation-confirm-row">
        <p class="confirm-text">
          Yank <strong>{targetLabel}</strong> now? This is a public takedown.
        </p>
        <div class="confirm-actions">
          <button
            type="button"
            class="confirm-yank"
            data-testid="moderation-confirm-yank"
            disabled={yanking}
            onclick={runYank}
          >
            {yanking ? 'Yanking…' : 'Confirm yank'}
          </button>
          <button
            type="button"
            class="confirm-cancel"
            data-testid="moderation-cancel-yank"
            disabled={yanking}
            onclick={cancelConfirm}
          >
            Cancel
          </button>
        </div>
      </div>
    {/if}

    {#if error}
      <p class="result fail" role="alert" data-testid="moderation-error">
        ✗ {error}
      </p>
    {/if}

    {#if result}
      <div class="result ok" role="status" data-testid="moderation-result">
        <p class="result-line">✓ Yanked. It's gone from public browse and install.</p>
        {#if result.note}
          <p class="result-note" data-testid="moderation-result-note">{result.note}</p>
        {/if}
      </div>
    {/if}
  </section>
</div>

<style>
  .moderation {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .head {
    min-width: 0;
  }

  .title {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 680;
  }

  .subtitle {
    margin: var(--space-1) 0 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
  }

  .section-title {
    margin: 0 0 var(--space-1);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .target {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
  }

  .target-id {
    display: block;
    margin-top: 2px;
    color: var(--muted-3);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
  }

  .field-label {
    display: block;
    margin-top: var(--space-1);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .text-input {
    width: 100%;
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
  }

  .text-input::placeholder {
    color: var(--muted-3);
  }

  .text-input:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 1px;
  }

  .text-input:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .limitation-note {
    margin: var(--space-1) 0 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid color-mix(in srgb, var(--amber) 34%, transparent);
    border-radius: 4px;
    background: color-mix(in srgb, var(--amber) 8%, transparent);
    color: var(--muted-2);
    font-size: var(--text-micro);
    line-height: 16px;
  }

  .yank-button {
    margin-top: var(--space-2);
    width: 100%;
    height: 34px;
    border: 1px solid var(--amber);
    border-radius: 4px;
    background: var(--amber);
    color: #1a1205;
    font: inherit;
    font-size: var(--text-base);
    font-weight: 680;
    cursor: pointer;
    transition: filter 140ms ease;
  }

  .yank-button:hover:not(:disabled) {
    filter: brightness(1.06);
  }

  .yank-button:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .yank-button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .confirm-row {
    margin-top: var(--space-2);
    padding: var(--space-3);
    border: 1px solid color-mix(in srgb, var(--amber) 50%, transparent);
    border-radius: 4px;
    background: color-mix(in srgb, var(--amber) 10%, transparent);
  }

  .confirm-text {
    margin: 0 0 var(--space-2);
    color: var(--fg);
    font-size: var(--text-base);
  }

  .confirm-actions {
    display: flex;
    gap: var(--space-2);
  }

  .confirm-yank {
    flex: 1 1 auto;
    height: 32px;
    border: 1px solid var(--amber);
    border-radius: 4px;
    background: var(--amber);
    color: #1a1205;
    font: inherit;
    font-weight: 680;
    cursor: pointer;
  }

  .confirm-yank:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .confirm-cancel {
    flex: 0 0 auto;
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    cursor: pointer;
  }

  .confirm-cancel:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .result {
    margin: var(--space-2) 0 0;
    font-size: var(--text-base);
  }

  .result.fail {
    color: var(--amber);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .result.ok {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .result-line {
    margin: 0;
    color: var(--green, #2faf6a);
    font-weight: 600;
  }

  .result-note {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--muted-2);
    font-size: var(--text-micro);
    line-height: 16px;
  }
</style>
