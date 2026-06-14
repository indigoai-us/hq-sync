<script lang="ts">
  /**
   * MarketplacePanel — the desktop-alt **Marketplace** tab body (US-008).
   *
   * Browses APPROVED creator-marketplace listings via the public hq-pro
   * `GET /v1/listings` route (no auth — see `lib/marketplace.ts`), rendering a
   * Foundry-style card grid (name · author handle · description · version),
   * with a text search box and a right-side detail slide-over that shows the
   * contributes summary + author. Explicit loading / empty / error states.
   *
   * Mirrors LibraryBrowser/LibraryList/LibraryDetailPanel conventions: Svelte 5
   * runes, the shared desktop-alt CSS variables, and the same slide-over layout.
   *
   * The search term is BOTH forwarded to the backend (`?q=`, debounced) AND
   * applied client-side over the fetched set for instant feedback while the
   * round-trip is in flight.
   */
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import {
    companyInstallTargets,
    filterListings,
    installMarketplacePack,
    listingDisplayName,
    loadMarketplaceListings,
    recordMarketplaceInstall,
    type InstallScope,
    type InstallTarget,
    type MarketplaceListing,
  } from '../lib/marketplace';
  import { coverForListing, coverFallback } from '../lib/pack-covers';
  import type { WorkspacesResult } from '../../lib/workspaces';

  let listings = $state<MarketplaceListing[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let query = $state('');
  let selected = $state<MarketplaceListing | null>(null);

  // ── Install scope picker (US-009) ──────────────────────────────────────────
  //
  // The detail slide-over offers an Install action with a scope picker: Personal
  // plus each company the user can ADMIN. Companies the user can't admin render
  // DISABLED with a reason (default-deny — see `companyInstallTargets`). The Rust
  // command re-verifies admin against vault truth and confines a company install
  // to companies/{co}/, so this UI gate is convenience, not the security boundary.
  let installTargets = $state<InstallTarget[]>([{ scope: { kind: 'personal' }, label: 'Personal', enabled: true }]);
  let chosenScope = $state<InstallScope>({ kind: 'personal' });
  let installing = $state(false);
  let installLog = $state<string[]>([]);
  let installResult = $state<{ ok: boolean; message: string } | null>(null);

  // The picker maps option index → target (so disabled options can't be chosen).
  let scopeIndex = $state(0);
  const chosenTarget = $derived(installTargets[scopeIndex] ?? installTargets[0]);

  $effect(() => {
    // Keep the canonical scope in sync with the selected (enabled) option.
    const t = chosenTarget;
    if (t && t.enabled) chosenScope = t.scope;
  });

  // Load the user's workspaces once to compute admin-eligible company targets.
  onMount(() => {
    let unlistenProgress: UnlistenFn | undefined;
    let unlistenComplete: UnlistenFn | undefined;
    let unlistenError: UnlistenFn | undefined;

    void (async () => {
      try {
        const result = await invoke<WorkspacesResult>('list_syncable_workspaces');
        installTargets = companyInstallTargets(result.workspaces ?? []);
        // Default to the first enabled target (Personal is always first + enabled).
        scopeIndex = installTargets.findIndex((t) => t.enabled);
        if (scopeIndex < 0) scopeIndex = 0;
      } catch (err) {
        console.error('list_syncable_workspaces (marketplace install targets) failed:', err);
        // Fall back to Personal-only — never block a personal install on a
        // company-list outage, and never silently enable a company install.
        installTargets = [{ scope: { kind: 'personal' }, label: 'Personal', enabled: true }];
        scopeIndex = 0;
      }
    })();

    // Stream install progress + terminal result from the Rust command. Hook-
    // consent prompts the CLI prints flow through as progress lines.
    void (async () => {
      unlistenProgress = await listen<{ line: string }>('marketplace:install-progress', (e) => {
        if (e.payload?.line) installLog = [...installLog, e.payload.line];
      });
      unlistenComplete = await listen('marketplace:install-complete', () => {
        installResult = { ok: true, message: 'Installed.' };
      });
      unlistenError = await listen<{ message: string }>('marketplace:install-error', (e) => {
        installResult = { ok: false, message: e.payload?.message ?? 'Install failed.' };
      });
    })();

    return () => {
      unlistenProgress?.();
      unlistenComplete?.();
      unlistenError?.();
    };
  });

  async function runInstall(): Promise<void> {
    if (!selected || installing) return;
    const target = chosenTarget;
    if (!target || !target.enabled) return; // default-deny: never install a disabled scope
    installing = true;
    installResult = null;
    installLog = [];
    try {
      await installMarketplacePack(selected.slug, selected.version, target.scope);
      // The success event also sets installResult; set here too in case the
      // command resolves before the event lands.
      if (!installResult) installResult = { ok: true, message: 'Installed.' };

      // ── US-019 install-metrics (best-effort, fire-and-forget) ───────────────
      //
      // After a SUCCESSFUL install, record an install event so the marketplace
      // metrics can count installer-vs-author installs (`POST /v1/listings/{id}/
      // installs`, JWT — installer uid = the caller's Cognito sub; body = { scope,
      // companySlug? }). The authed Rust `record_marketplace_install` command
      // forwards the bearer token (mirroring yank / decide).
      //
      // This is STRICTLY best-effort: it runs only after the install already
      // succeeded, with the scope the user installed with, and a metrics failure
      // must NEVER fail or block the install — so it's fire-and-forget and we
      // swallow any error (`.catch(() => {})`). We do NOT await it.
      void recordMarketplaceInstall(selected.id, target.scope).catch(() => {});
    } catch (err) {
      installResult = { ok: false, message: err instanceof Error ? err.message : String(err) };
    } finally {
      installing = false;
    }
  }

  // Debounced server query: re-fetch with `?q=` shortly after the user stops
  // typing, so the approved set narrows server-side too (not just client-side).
  let serverQuery = $state('');
  let debounceHandle: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const next = query;
    clearTimeout(debounceHandle);
    debounceHandle = setTimeout(() => {
      serverQuery = next;
    }, 220);
    return () => clearTimeout(debounceHandle);
  });

  // Load (and reload on serverQuery change). A cancel flag guards against an
  // out-of-order completion when queries change quickly.
  $effect(() => {
    const q = serverQuery;
    loading = true;
    error = null;
    let cancelled = false;

    void (async () => {
      try {
        const result = await loadMarketplaceListings(q);
        if (!cancelled) listings = result;
      } catch (err) {
        console.error('loadMarketplaceListings failed:', err);
        if (!cancelled) {
          error = 'Marketplace unavailable. Check your connection and try again.';
          listings = [];
        }
      } finally {
        if (!cancelled) loading = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  // Client-side filter for instant feedback while the debounced fetch is pending.
  const visible = $derived(filterListings(listings, query));

  function authorLabel(listing: MarketplaceListing): string {
    return listing.author ? `@${listing.author}` : 'unknown';
  }

  // ── US-019: attribution byline links to the creator profile ────────────────
  //
  // The @handle byline on each card + in the detail slide-over LINKS to the
  // creator's PUBLIC profile (the US-018 marketing directory page at
  // `https://hq.getindigo.ai/creators/<handle>`). An external link to the
  // marketing profile is the simplest fit — it opens in the system browser via
  // a plain anchor (mirrors ProfilePanel's preview links). A listing with no
  // author handle has no profile to link to, so it renders as plain text.
  const CREATOR_PROFILE_BASE = 'https://hq.getindigo.ai/creators';

  /** The creator-profile URL for a handle, or null when there's no handle. */
  function creatorProfileHref(listing: MarketplaceListing): string | null {
    const handle = listing.author?.trim();
    if (!handle) return null;
    return `${CREATOR_PROFILE_BASE}/${encodeURIComponent(handle)}`;
  }

  function resetInstallState(): void {
    installLog = [];
    installResult = null;
  }

  function select(listing: MarketplaceListing): void {
    selected = listing;
    resetInstallState();
  }
  function closeDetail(): void {
    selected = null;
    resetInstallState();
  }

  const selectedScopeLabel = $derived(
    chosenTarget?.scope.kind === 'company'
      ? `${chosenTarget.label} (shared with your team)`
      : 'Personal (only you)',
  );

  function handleKeydown(event: KeyboardEvent, listing: MarketplaceListing): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      select(listing);
    }
  }

  function handleDetailKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.stopPropagation();
      closeDetail();
    }
  }
</script>

<svelte:window onkeydown={selected ? handleDetailKeydown : undefined} />

<div class="marketplace" data-testid="marketplace-panel">
  <div class="toolbar">
    <p class="count" aria-live="polite">
      {#if loading}
        Loading…
      {:else}
        {visible.length}
        {visible.length === 1 ? 'listing' : 'listings'}
      {/if}
    </p>
    <input
      class="search"
      type="search"
      placeholder="Search the marketplace…"
      aria-label="Search marketplace listings"
      data-testid="marketplace-search"
      bind:value={query}
    />
  </div>

  <section class="your-listings" data-testid="marketplace-your-listings">
    <div>
      <h2>YOUR LISTINGS</h2>
      <p>Published packs you own will appear here with install metrics and review status.</p>
    </div>
    <span>{listings.filter((listing) => listing.author).length} tracked</span>
  </section>

  {#if error}
    <div class="state-error" role="alert" data-testid="marketplace-error">{error}</div>
  {:else if loading}
    <div class="grid-skeleton" aria-busy="true">
      {#each [0, 1, 2, 3, 4, 5] as cell (cell)}
        <div class="card-skeleton"></div>
      {/each}
    </div>
  {:else if visible.length === 0}
    <div class="state-empty" data-testid="marketplace-empty">
      <p>No listings</p>
      <span>
        {#if listings.length === 0}
          Nothing has been published to the marketplace yet.
        {:else}
          Try a different search.
        {/if}
      </span>
    </div>
  {:else}
    <div class="grid" aria-label="Marketplace listings">
      {#each visible as listing (listing.id)}
        {@const cover = coverForListing(listing)}
        <!--
          The card is keyboard-focusable + clickable (role="button") to open the
          detail slide-over, but it is a <div> rather than a <button> so the
          author byline can be a real nested <a> link to the creator profile
          (US-019) — a <button> can't legally contain an <a>. The card keeps full
          button semantics (role, tabindex, Enter/Space handler).

          The card leads with a full-bleed piece of cover art (unique per pack)
          with the pack name overlaid on a scrim, so each listing reads as a
          distinct object. Below the art sits a compact body (author + summary).
        -->
        <div
          role="button"
          tabindex="0"
          class="card"
          data-testid="marketplace-card"
          aria-label={`${listing.type} ${listingDisplayName(listing)} by ${authorLabel(listing)}`}
          onclick={() => select(listing)}
          onkeydown={(event) => handleKeydown(event, listing)}
        >
          <span class="accent" aria-hidden="true"></span>
          <div class="cover" data-testid="marketplace-cover">
            {#if cover}
              <img class="cover-img" src={cover} alt="" loading="lazy" decoding="async" />
            {:else}
              {@const fb = coverFallback(listing)}
              <div class="cover-fallback" style={`background:${fb.gradient}`} aria-hidden="true">
                <span class="cover-monogram">{fb.monogram}</span>
              </div>
            {/if}
            <div class="cover-scrim" aria-hidden="true"></div>
            <span class="kind-tag kind-chip">
              <span class="kind-dot" aria-hidden="true"></span>
              {listing.type}
            </span>
            <span class="pill version cover-version" data-testid="marketplace-version">v{listing.version}</span>
            <h3 class="card-name" title={listingDisplayName(listing)}>{listingDisplayName(listing)}</h3>
          </div>
          <div class="card-body">
            {#if creatorProfileHref(listing)}
              <!-- Byline → creator profile (US-019). stopPropagation so clicking
                   the link opens the profile instead of selecting the card. -->
              <a
                class="author author-link"
                href={creatorProfileHref(listing)}
                target="_blank"
                rel="noreferrer noopener"
                data-testid="marketplace-author"
                title={`View ${authorLabel(listing)} on the creator directory`}
                onclick={(event) => event.stopPropagation()}
              >{authorLabel(listing)}</a>
            {:else}
              <span class="author" data-testid="marketplace-author">{authorLabel(listing)}</span>
            {/if}
            {#if listing.summary}
              <p class="card-desc">{listing.summary}</p>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if selected}
  {@const detailCover = coverForListing(selected)}
  <div
    class="detail-backdrop"
    data-testid="marketplace-detail-backdrop"
    onclick={closeDetail}
    aria-hidden="true"
  ></div>

  <div
    class="detail-panel"
    role="dialog"
    aria-modal="true"
    aria-label={`Listing: ${listingDisplayName(selected)}`}
    data-testid="marketplace-detail-panel"
  >
    <div class="detail-cover" data-testid="marketplace-detail-cover">
      {#if detailCover}
        <img class="detail-cover-img" src={detailCover} alt="" />
      {:else}
        {@const fb = coverFallback(selected)}
        <div class="detail-cover-fallback" style={`background:${fb.gradient}`} aria-hidden="true">
          <span class="cover-monogram">{fb.monogram}</span>
        </div>
      {/if}
      <div class="detail-cover-scrim" aria-hidden="true"></div>
    </div>
    <header class="detail-header">
      <div class="header-text">
        <span class="kind-tag detail-kind">{selected.type}</span>
        <h2 class="detail-title">{listingDisplayName(selected)}</h2>
        <div class="badges">
          <span class="pill version">v{selected.version}</span>
          <span class="scope-badge">{selected.slug}</span>
        </div>
      </div>
      <button
        type="button"
        class="close-button"
        data-testid="marketplace-detail-close"
        aria-label="Close details"
        onclick={closeDetail}
      >
        <span aria-hidden="true">×</span>
      </button>
    </header>

    <div class="detail-body">
      <section class="detail-section">
        <h3 class="section-title">Author</h3>
        {#if creatorProfileHref(selected)}
          <!-- Byline → creator profile (US-019). Opens the public directory
               profile (US-018) in the system browser. -->
          <p class="section-body">
            <a
              class="author-link"
              href={creatorProfileHref(selected)}
              target="_blank"
              rel="noreferrer noopener"
              data-testid="marketplace-detail-author"
              title={`View ${authorLabel(selected)} on the creator directory`}
            >{authorLabel(selected)}</a>
          </p>
        {:else}
          <p class="section-body" data-testid="marketplace-detail-author">{authorLabel(selected)}</p>
        {/if}
      </section>

      {#if selected.summary}
        <section class="detail-section">
          <h3 class="section-title">Description</h3>
          <p class="section-body">{selected.summary}</p>
        </section>
      {/if}

      <section class="detail-section">
        <h3 class="section-title">Contributes</h3>
        <p class="section-body" data-testid="marketplace-detail-contributes">
          {selected.contributes ?? 'Not specified.'}
        </p>
      </section>

      <section class="detail-section readme-preview" data-testid="marketplace-readme-preview">
        <h3 class="section-title">README preview</h3>
        <p class="section-body">
          {selected.summary ?? selected.contributes ?? 'No README preview is available for this listing yet.'}
        </p>
      </section>

      <!-- Install action + scope picker (US-009) -->
      <section class="detail-section" data-testid="marketplace-install-section">
        <h3 class="section-title">Install</h3>

        <label class="scope-label" for="marketplace-scope">Scope</label>
        <select
          id="marketplace-scope"
          class="scope-select"
          data-testid="marketplace-scope-select"
          bind:value={scopeIndex}
          disabled={installing}
        >
          {#each installTargets as target, i (i)}
            <option
              value={i}
              disabled={!target.enabled}
              data-testid="marketplace-scope-option"
              data-enabled={target.enabled}
              data-slug={target.scope.kind === 'company' ? target.scope.slug : 'personal'}
            >
              {target.label}{target.enabled ? '' : ` — ${target.reason ?? 'unavailable'}`}
            </option>
          {/each}
        </select>

        <p class="scope-hint" data-testid="marketplace-scope-hint">
          {selectedScopeLabel}
        </p>

        {#if chosenTarget?.scope.kind === 'company'}
          <p class="consent-note" data-testid="marketplace-consent-note">
            This pack will sync to everyone on this team. Any hooks or scripts it
            contains stay scoped to this company and ask each teammate for consent
            on their machine before running — nothing is wired silently.
          </p>
        {/if}

        <button
          type="button"
          class="install-button"
          data-testid="marketplace-install-button"
          disabled={installing || !chosenTarget?.enabled}
          onclick={runInstall}
        >
          {#if installing}
            Installing…
          {:else if chosenTarget?.scope.kind === 'company'}
            Install to {chosenTarget.label}
          {:else}
            Install for me
          {/if}
        </button>

        {#if installResult}
          <p
            class="install-result"
            class:ok={installResult.ok}
            class:fail={!installResult.ok}
            role="status"
            data-testid="marketplace-install-result"
          >
            {installResult.ok ? '✓ Installed.' : `✗ ${installResult.message}`}
          </p>
        {/if}

        {#if installLog.length > 0}
          <pre class="install-log" data-testid="marketplace-install-log">{installLog.join('\n')}</pre>
        {/if}
      </section>
    </div>
  </div>
{/if}

<style>
  .marketplace {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-width: 0;
  }

  .your-listings {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-subtle);
  }

  .your-listings div {
    min-width: 0;
  }

  .your-listings h2,
  .your-listings p {
    margin: 0;
  }

  .your-listings h2 {
    color: var(--muted-2);
    font-size: var(--text-micro);
    font-weight: 700;
    line-height: 14px;
  }

  .your-listings p,
  .your-listings span {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  .count {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
  }

  .search {
    flex: 1 1 200px;
    max-width: 280px;
    min-width: 0;
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
  }

  .search::placeholder {
    color: var(--muted-3);
  }

  .search:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 1px;
  }

  /* ---- card grid (mirrors LibraryList) ---------------------------------- */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(272px, 1fr));
    align-items: start;
    gap: var(--space-3);
    min-width: 0;
  }

  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--row-active);
    text-align: left;
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      transform 160ms ease,
      box-shadow 160ms ease;
  }

  .card:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    transform: translateY(-2px);
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.3);
  }

  .card:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  /* Amber brand spine — runs the full card height, over the art's left edge. */
  .accent {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    z-index: 4;
    width: 3px;
    background: var(--amber);
    opacity: 0.6;
    transition: opacity 140ms ease;
  }
  .card:hover .accent {
    opacity: 1;
  }

  /* ---- cover art (the visual hero of each card) ------------------------- */
  .cover {
    position: relative;
    aspect-ratio: 16 / 9;
    width: 100%;
    overflow: hidden;
    background: var(--bg-subtle);
  }

  .cover-img,
  .cover-fallback {
    position: absolute;
    inset: 0;
    z-index: 1;
    width: 100%;
    height: 100%;
  }

  .cover-img {
    object-fit: cover;
    object-position: center;
    transition: transform 240ms ease;
  }
  .card:hover .cover-img {
    transform: scale(1.04);
  }

  .cover-fallback {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .cover-monogram {
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: 46px;
    font-weight: 700;
    color: rgba(255, 255, 255, 0.88);
    text-shadow: 0 2px 10px rgba(0, 0, 0, 0.45);
  }

  /* Bottom-up scrim so the overlaid name stays legible over any art. */
  .cover-scrim {
    position: absolute;
    inset: 0;
    z-index: 2;
    pointer-events: none;
    background: linear-gradient(
      to top,
      rgba(8, 8, 10, 0.88) 0%,
      rgba(8, 8, 10, 0.5) 30%,
      rgba(8, 8, 10, 0) 58%
    );
  }

  /* Frosted corner chips over the art (type + version). */
  .kind-chip,
  .cover-version {
    position: absolute;
    z-index: 3;
    top: var(--space-2);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }

  .kind-chip {
    inset-inline-start: var(--space-2);
    padding: 3px 9px;
    border-radius: 999px;
    background: rgba(8, 8, 10, 0.5);
    color: rgba(255, 255, 255, 0.92);
  }

  .cover-version {
    inset-inline-end: var(--space-2);
    background: rgba(8, 8, 10, 0.5);
    border-color: color-mix(in srgb, var(--amber) 48%, transparent);
  }

  .kind-tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0;
    text-transform: uppercase;
  }

  .kind-dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--amber);
  }

  /* Pack name overlaid on the bottom of the art (over the scrim). */
  .card-name {
    position: absolute;
    z-index: 3;
    inset-inline: var(--space-3) var(--space-3);
    bottom: var(--space-2);
    margin: 0;
    overflow: hidden;
    color: #fff;
    font-size: 15px;
    font-weight: 680;
    line-height: 19px;
    text-shadow: 0 1px 8px rgba(0, 0, 0, 0.65);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  /* Card body beneath the art — author byline + summary. */
  .card-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    padding: var(--space-3);
  }

  .author {
    color: var(--blue);
    font-size: var(--text-base);
    font-weight: 600;
  }

  /* US-019 — the byline link to the creator profile. Inherits the @handle
     blue; underlines on hover/focus so it reads as a clickable link. align-self
     keeps the card byline link hugging its text (not stretching the grid cell). */
  .author-link {
    align-self: flex-start;
    color: var(--blue);
    text-decoration: none;
    cursor: pointer;
  }

  .author-link:hover,
  .author-link:focus-visible {
    text-decoration: underline;
  }

  .author-link:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
    border-radius: 2px;
  }

  .pill {
    display: inline-flex;
    align-items: center;
    max-width: 100%;
    overflow: hidden;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--row-hover);
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.05em;
    line-height: 15px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pill.version {
    flex: 0 0 auto;
    border-color: color-mix(in srgb, var(--amber) 34%, transparent);
    color: var(--amber);
  }

  .card-desc {
    margin: 4px 0 0;
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  /* ---- states ----------------------------------------------------------- */
  .state-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-base);
  }

  .state-empty {
    padding: var(--space-6);
    border: 1px dashed var(--border-strong);
    border-radius: 4px;
    background: var(--row-active);
    text-align: center;
  }

  .state-empty p {
    margin: 0 0 var(--space-1);
    color: var(--fg);
    font-weight: 650;
  }

  .state-empty span {
    color: var(--muted);
    font-size: var(--text-base);
  }

  .grid-skeleton {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(272px, 1fr));
    gap: var(--space-3);
  }

  .card-skeleton {
    height: 212px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--row-active);
    animation: mk-skeleton-pulse 1.3s ease-in-out infinite;
  }

  @keyframes mk-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  /* ---- detail slide-over (mirrors LibraryDetailPanel) ------------------- */
  .detail-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(0, 0, 0, 0.45);
    animation: backdrop-fade 160ms ease;
  }

  .detail-panel {
    position: fixed;
    inset-block: 0;
    inset-inline-end: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    width: 520px;
    max-width: 94vw;
    border-left: 1px solid var(--border);
    background: var(--bg);
    box-shadow: -8px 0 32px rgba(0, 0, 0, 0.45);
    animation: panel-slide-in 200ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  /* Cover-art hero at the top of the detail slide-over. */
  .detail-cover {
    position: relative;
    flex-shrink: 0;
    width: 100%;
    height: 172px;
    overflow: hidden;
    background: var(--bg-subtle);
  }

  .detail-cover-img,
  .detail-cover-fallback {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .detail-cover-img {
    object-fit: cover;
    object-position: center;
  }

  .detail-cover-fallback {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  /* Fade the art into the panel surface so the header reads as one piece. */
  .detail-cover-scrim {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: linear-gradient(
      to bottom,
      rgba(0, 0, 0, 0) 42%,
      color-mix(in srgb, var(--bg) 94%, transparent) 100%
    );
  }

  .detail-header {
    display: flex;
    flex-shrink: 0;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-5) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  .header-text {
    min-width: 0;
  }

  .detail-kind {
    color: var(--muted);
  }

  .detail-title {
    margin: var(--space-1) 0 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 680;
    line-height: 22px;
    overflow-wrap: anywhere;
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-top: var(--space-2);
  }

  .scope-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-3);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 16px;
  }

  .close-button {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .close-button:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  .close-button:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .detail-body {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: var(--space-5);
    min-height: 0;
    padding: var(--space-5);
    overflow-y: auto;
  }

  .detail-section {
    min-width: 0;
  }

  .section-title {
    margin: 0 0 var(--space-2);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .section-body {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
    line-height: 19px;
    overflow-wrap: anywhere;
  }

  /* ---- install action + scope picker (US-009) --------------------------- */
  .scope-label {
    display: block;
    margin-bottom: var(--space-1);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .scope-select {
    width: 100%;
    height: 32px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
  }

  .scope-select:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 1px;
  }

  .scope-select:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .scope-hint {
    margin: var(--space-2) 0 0;
    color: var(--muted);
    font-size: var(--text-micro);
  }

  .consent-note {
    margin: var(--space-2) 0 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid color-mix(in srgb, var(--amber) 34%, transparent);
    border-radius: 4px;
    background: color-mix(in srgb, var(--amber) 8%, transparent);
    color: var(--muted-2);
    font-size: var(--text-micro);
    line-height: 16px;
  }

  .install-button {
    margin-top: var(--space-3);
    width: 100%;
    height: 34px;
    border: 1px solid var(--blue);
    border-radius: 4px;
    background: var(--blue);
    color: #fff;
    font: inherit;
    font-size: var(--text-base);
    font-weight: 650;
    cursor: pointer;
    transition:
      opacity 140ms ease,
      filter 140ms ease;
  }

  .install-button:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  .install-button:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .install-button:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .install-result {
    margin: var(--space-2) 0 0;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .install-result.ok {
    color: var(--green, #2faf6a);
  }

  .install-result.fail {
    color: var(--amber);
    overflow-wrap: anywhere;
  }

  .install-log {
    margin: var(--space-2) 0 0;
    max-height: 160px;
    padding: var(--space-2) var(--space-3);
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    line-height: 15px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  @keyframes backdrop-fade {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes panel-slide-in {
    from {
      transform: translateX(16px);
      opacity: 0;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .card,
    .accent,
    .close-button {
      transition: none;
    }
    .card:hover {
      transform: none;
    }
    .card-skeleton,
    .detail-backdrop,
    .detail-panel {
      animation: none;
    }
  }
</style>
