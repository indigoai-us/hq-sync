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

import type { Workspace } from '../../lib/workspaces';

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

// ---------------------------------------------------------------------------
// US-009 — install scope (personal vs. company), tenant-isolated.
// ---------------------------------------------------------------------------

/**
 * Where an install lands. Mirrors the Rust `InstallScope` tagged enum 1:1 —
 * `{ kind: 'personal' }` or `{ kind: 'company', slug }`.
 */
export type InstallScope = { kind: 'personal' } | { kind: 'company'; slug: string };

/**
 * A selectable Install target for the scope picker. `enabled === false` carries
 * a human `reason` (e.g. "requires company-admin") and renders disabled.
 *
 * IMPORTANT (default-deny): a company option is only enabled when the user is
 * positively known to be admin/owner of that company. Anything else — member,
 * viewer, pending, or an unknown/absent role — yields a DISABLED option. This is
 * convenience only; the Rust `install_marketplace_pack` command re-verifies
 * admin against vault membership truth before any install.
 */
export interface InstallTarget {
  scope: InstallScope;
  /** Picker label (e.g. "Personal", company display name). */
  label: string;
  enabled: boolean;
  /** When disabled, why (shown as a hint / tooltip). */
  reason?: string;
}

/** Roles that grant company-admin authority. Kept in lockstep with Rust. */
function roleIsAdmin(role: string | null | undefined): boolean {
  const r = (role ?? '').trim().toLowerCase();
  return r === 'admin' || r === 'owner';
}

/**
 * Build the scope-picker targets from the user's workspaces.
 *
 * Always includes a Personal target (always enabled). Then one target per
 * COMPANY workspace (the `personal` pseudo-company is excluded — it's the
 * Personal target). A company is ENABLED only when the membership is active AND
 * the role is admin/owner; otherwise it's disabled with a reason. Default-deny:
 * a company whose role is null/unknown is disabled ("requires company-admin").
 */
export function companyInstallTargets(workspaces: Workspace[]): InstallTarget[] {
  const personal: InstallTarget = {
    scope: { kind: 'personal' },
    label: 'Personal',
    enabled: true,
  };

  const companies = workspaces
    .filter((w) => w.kind === 'company' && w.slug !== 'personal')
    .map((w): InstallTarget => {
      const label = w.displayName?.trim() || w.slug;
      const active = (w.membershipStatus ?? '').toLowerCase() === 'active';
      const admin = roleIsAdmin(w.role);
      if (admin && active) {
        return { scope: { kind: 'company', slug: w.slug }, label, enabled: true };
      }
      let reason = 'Requires company-admin';
      if (!active && w.membershipStatus) {
        reason = `Requires company-admin (membership ${w.membershipStatus.toLowerCase()})`;
      } else if (!w.role) {
        // Unknown role → default-deny.
        reason = 'Requires company-admin (your role is unknown)';
      }
      return { scope: { kind: 'company', slug: w.slug }, label, enabled: false, reason };
    })
    // Stable order: enabled (admin) companies first, then alphabetical.
    .sort((a, b) => {
      if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
      return a.label.localeCompare(b.label);
    });

  return [personal, ...companies];
}

// ---------------------------------------------------------------------------
// US-022 — emergency yank / takedown (admin-gated kill switch).
// ---------------------------------------------------------------------------

/** Result of a successful yank — mirrors the Rust `YankResult` 1:1. */
export interface YankResult {
  /** The listing id that was yanked. */
  id: string;
  /** New status — always `"yanked"` on success. */
  status: string;
  /**
   * Server note describing the v1 limitation (already-installed users are NOT
   * auto-removed). The ModerationPanel renders this to the admin.
   */
  note: string;
}

/**
 * Yank (emergency takedown) a marketplace listing. Admin-gated on the SERVER
 * (`@getindigo.ai` id_token) — the Rust command forwards the caller's bearer
 * token and the server is the sole authorization boundary. A non-empty `reason`
 * is required (recorded for the audit trail; the Rust side also validates).
 *
 * On success the listing flips to `status = yanked` server-side and instantly
 * disappears from public browse + detail + install (a runtime status flip, no
 * deploy). Already-installed users are NOT auto-removed in v1.
 */
export async function yankMarketplaceListing(
  id: string,
  reason: string,
): Promise<YankResult> {
  return invoke<YankResult>('yank_marketplace_listing', { id, reason });
}

/**
 * Install a marketplace pack into the chosen scope. Streams progress via the
 * `marketplace:install-progress` event and resolves/rejects on the terminal
 * `marketplace:install-complete` / `marketplace:install-error` (surfaced as the
 * promise outcome here). The Rust side enforces admin + path containment and
 * never bypasses the hook-consent gate.
 */
export async function installMarketplacePack(
  slug: string,
  version: string | null | undefined,
  scope: InstallScope,
): Promise<void> {
  return invoke<void>('install_marketplace_pack', {
    slug,
    version: version?.trim() ? version.trim() : null,
    scope,
  });
}
