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
  import {
    filterListings,
    loadMarketplaceListings,
    type MarketplaceListing,
  } from '../lib/marketplace';

  let listings = $state<MarketplaceListing[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let query = $state('');
  let selected = $state<MarketplaceListing | null>(null);

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

  function select(listing: MarketplaceListing): void {
    selected = listing;
  }
  function closeDetail(): void {
    selected = null;
  }

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
        <button
          type="button"
          class="card"
          data-testid="marketplace-card"
          aria-label={`${listing.type} ${listing.name} by ${authorLabel(listing)}`}
          onclick={() => select(listing)}
          onkeydown={(event) => handleKeydown(event, listing)}
        >
          <span class="accent" aria-hidden="true"></span>
          <div class="card-head">
            <span class="kind-tag">
              <span class="kind-dot" aria-hidden="true"></span>
              {listing.type}
            </span>
            <span class="pill version" data-testid="marketplace-version">v{listing.version}</span>
          </div>
          <h3 class="card-name" title={listing.name}>{listing.name}</h3>
          <span class="author" data-testid="marketplace-author">{authorLabel(listing)}</span>
          {#if listing.summary}
            <p class="card-desc">{listing.summary}</p>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

{#if selected}
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
    aria-label={`Listing: ${selected.name}`}
    data-testid="marketplace-detail-panel"
  >
    <header class="detail-header">
      <div class="header-text">
        <span class="kind-tag detail-kind">{selected.type}</span>
        <h2 class="detail-title">{selected.name}</h2>
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
        <p class="section-body" data-testid="marketplace-detail-author">{authorLabel(selected)}</p>
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
    gap: var(--space-2);
    min-width: 0;
  }

  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    padding: var(--space-3) var(--space-3) var(--space-3) calc(var(--space-3) + 4px);
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    text-align: left;
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      transform 140ms ease;
  }

  .card:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    transform: translateY(-1px);
  }

  .card:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .accent {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    width: 3px;
    background: var(--amber);
    opacity: 0.55;
    transition: opacity 140ms ease;
  }
  .card:hover .accent {
    opacity: 1;
  }

  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-width: 0;
  }

  .kind-tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.09em;
    text-transform: uppercase;
  }

  .kind-dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--amber);
  }

  .card-name {
    margin: 2px 0 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .author {
    color: var(--blue);
    font-size: var(--text-base);
    font-weight: 600;
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
    gap: var(--space-2);
  }

  .card-skeleton {
    height: 104px;
    border: 1px solid var(--border);
    border-radius: 4px;
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
