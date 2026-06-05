<script lang="ts">
  /**
   * ModerationPanel — the desktop-alt moderation reviewer surface.
   *
   * Two capabilities:
   *
   *   1. QUEUE (US-012). An admin-only review queue of `pending_review` listings
   *      (`GET /v1/moderation/queue`). Each row shows author + name + version +
   *      contributes summary + submittedAt. Selecting a row opens a REVIEW view:
   *        (a) the tarball-contents preview (the pack's file manifest) + the
   *            contributes summary — the "what's in the box" / code side, and
   *        (b) the natural-language instructions (SKILL.md / worker prose) WITH
   *            the backend `injectionScan` flagged spans highlighted inline.
   *      The reviewer MUST tick an explicit acknowledgement — "I reviewed the
   *      instructions for prompt-injection" — which GATES the Approve button
   *      (Approve is disabled until acked). Reject takes a note. Approve/Reject
   *      call `POST /v1/moderation/listings/{id}` and, on success, the item is
   *      removed from the local queue (it's no longer pending). The decide call
   *      forwards the item's optimistic-lock token so a concurrent approve+reject
   *      can't race (a second writer gets a 409, surfaced as an error).
   *
   *   2. YANK (US-022). The launch-critical emergency takedown kill switch for an
   *      already-approved listing — preserved below the queue.
   *
   * ADMIN GATE (UX only — the server is the sole authorization boundary). The
   * panel calls `desktop_alt_enabled` (true iff the signed-in email ends in
   * `@getindigo.ai`) to decide whether to render the moderation surface at all.
   * DEFAULT-DENY: until that check resolves, and on ANY error, the panel renders
   * LOCKED. A non-admin who somehow reaches the commands still gets a 403 from
   * the server (the queue/decide parsers map 403 → a clear "admin only" error).
   *
   * Svelte 5 runes + the shared desktop-alt CSS variables, mirroring
   * MarketplacePanel (US-008) conventions.
   */
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import {
    canApprove,
    decideModerationListing,
    highlightInstruction,
    loadModerationQueue,
    yankMarketplaceListing,
    type ModerationQueueItem,
    type YankResult,
  } from '../lib/marketplace';

  // ── Admin gate (UX only; default-deny) ─────────────────────────────────────
  // `null` = unknown (still checking) → treated as LOCKED. Only an explicit
  // `true` unlocks the surface. Any error → false (locked).
  let isAdmin = $state<boolean | null>(null);

  // ── Queue state ────────────────────────────────────────────────────────────
  let queue = $state<ModerationQueueItem[]>([]);
  let queueLoading = $state(true);
  let queueError = $state<string | null>(null);
  let selectedId = $state<string | null>(null);

  const selected = $derived(queue.find((q) => q.id === selectedId) ?? null);

  // Per-item review state, keyed by listing id, so switching selection doesn't
  // leak one item's ack/note onto another.
  let acknowledged = $state(false);
  let rejectNote = $state('');
  let deciding = $state(false);
  let decideError = $state<string | null>(null);
  let lastDecision = $state<{ id: string; status: string } | null>(null);

  const approveEnabled = $derived(canApprove({ acknowledged, busy: deciding }));

  // The highlighted instruction segments for the selected item, per doc.
  const instructionViews = $derived(
    (selected?.instructions ?? []).map((doc) => ({
      path: doc.path,
      segments: highlightInstruction(doc, selected?.injectionScan ?? []),
    })),
  );

  // Flags that couldn't be sliced inline (no doc text / out-of-range) still get
  // listed so a reviewer never misses a flag.
  const flagList = $derived(selected?.injectionScan ?? []);

  function selectItem(id: string): void {
    selectedId = id;
    acknowledged = false;
    rejectNote = '';
    decideError = null;
  }

  function backToList(): void {
    selectedId = null;
    acknowledged = false;
    rejectNote = '';
    decideError = null;
  }

  async function loadQueue(): Promise<void> {
    queueLoading = true;
    queueError = null;
    try {
      queue = await loadModerationQueue();
    } catch (err) {
      queueError = err instanceof Error ? err.message : String(err);
      queue = [];
    } finally {
      queueLoading = false;
    }
  }

  async function decide(decision: 'approve' | 'reject'): Promise<void> {
    const item = selected;
    if (!item) return;
    if (decision === 'approve' && !approveEnabled) return;
    if (decision === 'reject' && rejectNote.trim().length === 0) {
      decideError = 'A note is required to reject a listing.';
      return;
    }
    deciding = true;
    decideError = null;
    try {
      const res = await decideModerationListing(
        item.id,
        decision,
        decision === 'reject' ? rejectNote.trim() : null,
        item.versionLock ?? null,
      );
      // On success the item is no longer pending — drop it from the local queue.
      queue = queue.filter((q) => q.id !== item.id);
      lastDecision = { id: item.id, status: res.status };
      backToList();
    } catch (err) {
      decideError = err instanceof Error ? err.message : String(err);
    } finally {
      deciding = false;
    }
  }

  function fmtDate(iso: string): string {
    if (!iso) return '—';
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  onMount(async () => {
    try {
      // Default-deny: only an explicit true unlocks. Any error → locked.
      isAdmin = (await invoke<boolean>('desktop_alt_enabled')) === true;
    } catch {
      isAdmin = false;
    }
    if (isAdmin) {
      await loadQueue();
    } else {
      queueLoading = false;
    }
  });

  // ── Yank (US-022 emergency takedown) — preserved ───────────────────────────
  let {
    listing = null,
  }: {
    listing?: { id: string; name?: string } | null;
  } = $props();

  let manualId = $state('');
  const targetId = $derived((listing?.id ?? manualId).trim());
  const targetLabel = $derived(listing?.name?.trim() || targetId || '—');

  let reason = $state('');
  let confirmArmed = $state(false);
  let yanking = $state(false);
  let yankResult = $state<YankResult | null>(null);
  let yankError = $state<string | null>(null);

  const ALREADY_INSTALLED_NOTE =
    'Already-installed users are NOT auto-removed in v1. Yanking removes the ' +
    'listing from the directory and refuses new installs, but anyone who ' +
    'already installed this pack keeps it until they remove it themselves.';

  const canYank = $derived(
    targetId.length > 0 && reason.trim().length > 0 && !yanking,
  );

  function armConfirm(): void {
    yankError = null;
    if (targetId.length === 0) {
      yankError = 'Enter a listing id to yank.';
      return;
    }
    if (reason.trim().length === 0) {
      yankError = 'A reason is required to yank a listing.';
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
    yankError = null;
    yankResult = null;
    try {
      yankResult = await yankMarketplaceListing(targetId, reason.trim());
      confirmArmed = false;
      reason = '';
    } catch (err) {
      yankError = err instanceof Error ? err.message : String(err);
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
      Review submitted packs before they go public, and pull approved listings in
      an emergency. Admin-only — the server enforces it.
    </p>
  </header>

  {#if isAdmin !== true}
    <!-- AC3: default-deny. Unknown (still checking) and non-admin both lock. -->
    <section class="section locked" data-testid="moderation-locked">
      <h3 class="section-title">Locked</h3>
      <p class="locked-text">
        {#if isAdmin === null}
          Checking your access…
        {:else}
          Moderation is restricted to <strong>@getindigo.ai</strong> reviewers.
          You don't have access to this surface.
        {/if}
      </p>
    </section>
  {:else}
    <!-- ── Review queue (US-012) ─────────────────────────────────────────── -->
    <section class="section" data-testid="moderation-queue-section">
      <div class="queue-head">
        <h3 class="section-title">Review queue</h3>
        <button
          type="button"
          class="refresh"
          data-testid="moderation-refresh"
          onclick={loadQueue}
          disabled={queueLoading}
        >
          {queueLoading ? 'Loading…' : 'Refresh'}
        </button>
      </div>

      {#if lastDecision}
        <p class="decided" role="status" data-testid="moderation-last-decision">
          ✓ Listing {lastDecision.id} {lastDecision.status}.
        </p>
      {/if}

      {#if queueLoading}
        <p class="muted-line" data-testid="moderation-queue-loading">Loading queue…</p>
      {:else if queueError}
        <p class="result fail" role="alert" data-testid="moderation-queue-error">
          ✗ {queueError}
        </p>
      {:else if selected}
        <!-- ── Review view for the selected item ──────────────────────────── -->
        <div class="review" data-testid="moderation-review">
          <button
            type="button"
            class="back"
            data-testid="moderation-back"
            onclick={backToList}
          >
            ← Back to queue
          </button>

          <h4 class="review-title">{selected.name} <span class="ver">v{selected.version}</span></h4>
          <p class="review-meta">
            by <strong>{selected.author || 'unknown'}</strong>
            · {selected.type}
            · submitted {fmtDate(selected.submittedAt)}
          </p>
          {#if selected.contributes}
            <p class="contributes" data-testid="moderation-contributes">
              Contributes: {selected.contributes}
            </p>
          {/if}

          <!-- (a) Tarball-contents preview / "what's in the box". -->
          <div class="review-block" data-testid="moderation-files">
            <h5 class="block-title">Tarball contents</h5>
            {#if selected.files.length > 0}
              <ul class="file-list">
                {#each selected.files as f (f)}
                  <li class="file">{f}</li>
                {/each}
              </ul>
            {:else}
              <p class="muted-line">
                No file manifest returned. Inspect the full tarball via download.
              </p>
            {/if}
          </div>

          <!-- Injection flags surfaced prominently. -->
          {#if flagList.length > 0}
            <div class="injection-banner" data-testid="moderation-injection-banner">
              ⚠ {flagList.length} potential prompt-injection
              {flagList.length === 1 ? 'flag' : 'flags'} in the instructions —
              read carefully before approving.
            </div>
          {/if}

          <!-- (b) Natural-language instructions with flagged spans highlighted. -->
          <div class="review-block" data-testid="moderation-instructions">
            <h5 class="block-title">Instructions (prompt-injection review)</h5>
            {#if instructionViews.length > 0}
              {#each instructionViews as view (view.path)}
                <p class="doc-path">{view.path}</p>
                <pre class="doc-text">{#each view.segments as seg, i (i)}{#if seg.flagged}<mark
                      class="flagged"
                      data-testid="moderation-flag"
                      title={seg.reason}>{seg.text}</mark>{:else}{seg.text}{/if}{/each}</pre>
              {/each}
            {:else}
              <p class="muted-line">
                No instruction prose returned. The pack may be code-only — still
                acknowledge that you reviewed it.
              </p>
            {/if}

            {#if flagList.length > 0}
              <ul class="flag-reasons" data-testid="moderation-flag-reasons">
                {#each flagList as flag, i (i)}
                  <li>
                    <span class="flag-file">{flag.file}</span>
                    {flag.reason || 'flagged'}
                    {#if flag.snippet}— <em>“{flag.snippet}”</em>{/if}
                  </li>
                {/each}
              </ul>
            {/if}
          </div>

          <!-- AC4: explicit acknowledgement GATES Approve. -->
          <label class="ack" data-testid="moderation-ack-label">
            <input
              type="checkbox"
              data-testid="moderation-ack"
              bind:checked={acknowledged}
              disabled={deciding}
            />
            I reviewed the instructions for prompt-injection
          </label>

          {#if decideError}
            <p class="result fail" role="alert" data-testid="moderation-decide-error">
              ✗ {decideError}
            </p>
          {/if}

          <div class="decide-row">
            <button
              type="button"
              class="approve"
              data-testid="moderation-approve"
              disabled={!approveEnabled}
              title={approveEnabled
                ? 'Approve and publish'
                : 'Acknowledge the instruction review to enable Approve'}
              onclick={() => decide('approve')}
            >
              {deciding ? 'Working…' : 'Approve'}
            </button>
          </div>

          <label class="field-label" for="moderation-reject-note">Reject note (required to reject)</label>
          <input
            id="moderation-reject-note"
            class="text-input"
            type="text"
            placeholder="Why is this being rejected?"
            autocomplete="off"
            data-testid="moderation-reject-note"
            bind:value={rejectNote}
            disabled={deciding}
          />
          <button
            type="button"
            class="reject"
            data-testid="moderation-reject"
            disabled={deciding || rejectNote.trim().length === 0}
            onclick={() => decide('reject')}
          >
            {deciding ? 'Working…' : 'Reject'}
          </button>
        </div>
      {:else if queue.length === 0}
        <p class="muted-line" data-testid="moderation-queue-empty">
          The queue is empty — no packs are waiting for review.
        </p>
      {:else}
        <!-- ── Queue list ────────────────────────────────────────────────── -->
        <ul class="queue-list" data-testid="moderation-queue-list">
          {#each queue as item (item.id)}
            <li>
              <button
                type="button"
                class="queue-row"
                data-testid="moderation-queue-row"
                onclick={() => selectItem(item.id)}
              >
                <span class="row-main">
                  <span class="row-name">{item.name}</span>
                  <span class="row-ver">v{item.version}</span>
                  {#if item.injectionScan.length > 0}
                    <span class="row-flag" data-testid="moderation-row-flag" title="prompt-injection flags">⚠ {item.injectionScan.length}</span>
                  {/if}
                </span>
                <span class="row-sub">
                  by {item.author || 'unknown'}
                  {#if item.contributes}· {item.contributes}{/if}
                  · {fmtDate(item.submittedAt)}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <!-- ── Emergency yank (US-022) — preserved ───────────────────────────── -->
    <section class="section" data-testid="moderation-yank-section">
      <h3 class="section-title">Yank an approved listing</h3>
      <p class="yank-sub">
        Emergency takedown. Yanking pulls an already-approved listing from public
        browse, detail, and install immediately — a runtime change, no deploy.
      </p>

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

      {#if yankError}
        <p class="result fail" role="alert" data-testid="moderation-error">
          ✗ {yankError}
        </p>
      {/if}

      {#if yankResult}
        <div class="result ok" role="status" data-testid="moderation-result">
          <p class="result-line">✓ Yanked. It's gone from public browse and install.</p>
          {#if yankResult.note}
            <p class="result-note" data-testid="moderation-result-note">{yankResult.note}</p>
          {/if}
        </div>
      {/if}
    </section>
  {/if}
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

  .section.locked {
    border-style: dashed;
  }

  .locked-text {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .section-title {
    margin: 0 0 var(--space-1);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .queue-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .refresh {
    height: 26px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-micro);
    cursor: pointer;
  }

  .refresh:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .muted-line {
    margin: var(--space-1) 0;
    color: var(--muted-2);
    font-size: var(--text-base);
  }

  .decided {
    margin: 0;
    color: var(--green, #2faf6a);
    font-size: var(--text-micro);
    font-weight: 600;
  }

  /* Queue list */
  .queue-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .queue-row {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .queue-row:hover {
    border-color: var(--blue);
  }

  .row-main {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }

  .row-name {
    font-size: var(--text-base);
    font-weight: 640;
  }

  .row-ver {
    color: var(--muted-3);
    font-size: var(--text-micro);
  }

  .row-flag {
    margin-left: auto;
    color: var(--amber);
    font-size: var(--text-micro);
    font-weight: 700;
  }

  .row-sub {
    color: var(--muted-2);
    font-size: var(--text-micro);
  }

  /* Review view */
  .review {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .back {
    align-self: flex-start;
    padding: 0;
    border: none;
    background: none;
    color: var(--blue);
    font: inherit;
    font-size: var(--text-micro);
    cursor: pointer;
  }

  .review-title {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 680;
  }

  .ver {
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 500;
  }

  .review-meta {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-micro);
  }

  .contributes {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
  }

  .review-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    min-width: 0;
  }

  .block-title {
    margin: 0;
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .file-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .file {
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    overflow-wrap: anywhere;
  }

  .doc-path {
    margin: var(--space-1) 0 0;
    color: var(--muted-3);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
  }

  .doc-text {
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--fg);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    line-height: 16px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 220px;
    overflow: auto;
  }

  .flagged {
    background: color-mix(in srgb, var(--amber) 38%, transparent);
    color: var(--fg);
    border-radius: 2px;
    padding: 0 1px;
  }

  .injection-banner {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--amber);
    border-radius: 4px;
    background: color-mix(in srgb, var(--amber) 12%, transparent);
    color: var(--fg);
    font-size: var(--text-micro);
    font-weight: 600;
    line-height: 16px;
  }

  .flag-reasons {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-4);
    color: var(--muted-2);
    font-size: var(--text-micro);
    line-height: 16px;
  }

  .flag-file {
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    color: var(--muted-3);
  }

  .ack {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin-top: var(--space-1);
    color: var(--fg);
    font-size: var(--text-base);
    cursor: pointer;
  }

  .ack input {
    margin-top: 2px;
  }

  .decide-row {
    display: flex;
    gap: var(--space-2);
  }

  .approve {
    flex: 1 1 auto;
    height: 34px;
    border: 1px solid var(--green, #2faf6a);
    border-radius: 4px;
    background: var(--green, #2faf6a);
    color: #05140b;
    font: inherit;
    font-weight: 680;
    cursor: pointer;
    transition: filter 140ms ease;
  }

  .approve:hover:not(:disabled) {
    filter: brightness(1.06);
  }

  .approve:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .reject {
    height: 32px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    cursor: pointer;
  }

  .reject:disabled {
    opacity: 0.55;
    cursor: not-allowed;
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

  /* Yank */
  .yank-sub {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-micro);
    line-height: 16px;
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
