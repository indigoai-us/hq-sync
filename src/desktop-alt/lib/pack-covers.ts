/**
 * Pack cover art for the Marketplace cards.
 *
 * Each listing gets a unique, on-brand piece of cover art (the Indigo Midjourney
 * "moodboard" style shared with getindigo.ai + the email headers) so a card reads
 * as a distinct object, not a row of text. Covers are bundled with the app and
 * keyed by the pack `slug` (the stable, per-creator install identifier).
 *
 * FORWARD-COMPATIBLE: `coverForListing` prefers a server-provided
 * `coverImageUrl` when the backend starts serving one (per-listing, scales to any
 * creator's pack), and only falls back to the bundled map for the packs we ship
 * art for today. A pack with neither resolves to `null`, and the card renders a
 * deterministic branded gradient placeholder instead (see `coverFallback`).
 *
 * Kept rune-free + asset-import-only so it's trivially unit-testable.
 */

import type { MarketplaceListing } from './marketplace';

// Vite resolves each import to a hashed asset URL string at build time.
import engineeringCover from '../assets/pack-covers/engineering.jpg';
import gstackCover from '../assets/pack-covers/gstack.jpg';
import pocockCover from '../assets/pack-covers/pocock-skills.jpg';
import impeccableCover from '../assets/pack-covers/impeccable.jpg';
import magicpathCover from '../assets/pack-covers/magicpath-agent-skills.jpg';

/**
 * Bundled cover art, keyed by pack slug. Add an entry here (and the asset under
 * `assets/pack-covers/`) when a new pack ships with first-party art; everything
 * else falls back to the branded gradient placeholder until the backend serves a
 * per-listing `coverImageUrl`.
 */
export const BUNDLED_PACK_COVERS: Readonly<Record<string, string>> = {
  engineering: engineeringCover,
  gstack: gstackCover,
  'pocock-skills': pocockCover,
  impeccable: impeccableCover,
  'magicpath-agent-skills': magicpathCover,
};

/**
 * Resolve the cover-art URL for a listing, or `null` when none is available.
 *
 * Precedence: a server-provided `coverImageUrl` (future backend) wins over the
 * bundled-by-slug art, so hosted covers seamlessly take over once they exist.
 */
export function coverForListing(listing: MarketplaceListing): string | null {
  const hosted = listing.coverImageUrl?.trim();
  if (hosted) return hosted;
  return BUNDLED_PACK_COVERS[listing.slug] ?? null;
}

/**
 * A deterministic, branded fallback for a listing with no cover art: a hue
 * derived from the slug (so a given pack always gets the same placeholder color)
 * plus the pack's leading initial. Pure + DOM-free so it's unit-testable.
 */
export interface CoverFallback {
  /** A CSS `linear-gradient(...)` background derived from the slug. */
  gradient: string;
  /** A short monogram (uppercased first letter of the name/slug). */
  monogram: string;
}

/** FNV-1a-ish string hash → stable non-negative integer (for hue selection). */
function hashString(value: string): number {
  let h = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    h ^= value.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** Build the deterministic gradient + monogram placeholder for a listing. */
export function coverFallback(listing: MarketplaceListing): CoverFallback {
  const key = listing.slug || listing.name || '';
  const hue = hashString(key) % 360;
  const hue2 = (hue + 38) % 360;
  // Warm, low-saturation duotone in the moodboard family — never garish.
  const gradient = `linear-gradient(135deg, hsl(${hue} 42% 32%), hsl(${hue2} 38% 18%))`;
  const source = (listing.name || listing.slug || '?').trim();
  const monogram = (source.charAt(0) || '?').toUpperCase();
  return { gradient, monogram };
}
