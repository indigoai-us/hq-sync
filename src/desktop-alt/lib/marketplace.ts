/**
 * Thin adapter over the Marketplace Rust commands (`list_marketplace_listings`
 * / `get_marketplace_listing`), which call the PUBLIC hq-pro listings routes
 * (US-005). The Rust structs are camelCase-serialised, so the wire shapes map
 * 1:1 to these TS types.
 *
 * No Svelte runes here — just data + a client-side text filter, so it stays
 * trivially unit-testable. Mirrors the structure of `lib/library.ts`.
 */

import { invoke } from '@tauri-apps/api/core';

/** One approved listing row (`MarketplaceListing` wire shape, US-005 public). */
export interface MarketplaceListing {
  /** Stable listing id — the detail key. */
  id: string;
  /** What the pack contains (`skill` | `worker`). */
  type: string;
  /** Human-readable listing name. */
  name: string;
  /** Pack slug — the install identifier. */
  slug: string;
  /** Published semantic version. */
  version: string;
  /** Author's PUBLIC handle (a string — never the internal creator uid). */
  author: string;
  /** Short directory description, when present. */
  summary?: string | null;
  /** Human-readable summary of what the pack contributes, when present. */
  contributes?: string | null;
  /** ISO-8601 publish timestamp. */
  createdAt: string;
}

/** Listing detail (`get_marketplace_listing`) — adds the presigned download URL. */
export interface MarketplaceListingDetail extends MarketplaceListing {
  /** Presigned GET URL for the pack tarball (24h expiry). */
  downloadUrl: string;
}

/** Browse approved listings, optionally forwarding a `?q=` search term. */
export async function loadMarketplaceListings(
  query?: string,
): Promise<MarketplaceListing[]> {
  const trimmed = query?.trim();
  return invoke<MarketplaceListing[]>('list_marketplace_listings', {
    query: trimmed ? trimmed : null,
  });
}

/** Fetch one listing's public detail (incl. the presigned download URL). */
export async function loadMarketplaceListing(
  id: string,
): Promise<MarketplaceListingDetail> {
  return invoke<MarketplaceListingDetail>('get_marketplace_listing', { id });
}

/** Lowercased haystack for the client-side text filter. */
export function listingHaystack(listing: MarketplaceListing): string {
  return [
    listing.name,
    listing.slug,
    listing.author,
    listing.summary ?? '',
    listing.contributes ?? '',
    listing.type,
    listing.version,
  ]
    .join(' ')
    .toLowerCase();
}

/** Filter listings by a free-text query (name/slug/author/summary/contributes). */
export function filterListings(
  listings: MarketplaceListing[],
  query: string,
): MarketplaceListing[] {
  const q = query.trim().toLowerCase();
  if (q === '') return listings;
  return listings.filter((l) => listingHaystack(l).includes(q));
}
