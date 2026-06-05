import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * US-019 — attribution byline links to the creator profile.
 *
 * AC1 (hq-sync slice): the author @handle byline shown on Marketplace listing
 * CARDS and in the listing DETAIL slide-over must LINK to the creator's public
 * profile (the US-018 marketing directory page at
 * `https://hq.getindigo.ai/creators/<handle>`).
 *
 * This repo's frontend has no DOM/component test harness (vitest `environment:
 * "node"`), so — like the other US-0xx story tests — this is a source-contract
 * test over the `MarketplacePanel.svelte` source: it asserts the byline renders
 * as a real anchor to the creator-profile URL on BOTH surfaces, opens safely in
 * the system browser, and degrades to plain text when there is no handle.
 */

const panel = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/panels/MarketplacePanel.svelte'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-019: attribution byline links to the creator profile', () => {
  it('builds the creator-profile URL from the US-018 marketing directory base + the @handle', () => {
    const src = normalize(panel);
    // Links to the public creator directory profile (US-018 marketing page).
    expect(panel).toContain("const CREATOR_PROFILE_BASE = 'https://hq.getindigo.ai/creators'");
    // URL = base + the (encoded) handle; a missing handle → no link.
    expect(src).toContain('function creatorProfileHref(listing: MarketplaceListing): string | null');
    expect(src).toContain('const handle = listing.author?.trim();');
    expect(src).toContain('if (!handle) return null;');
    expect(src).toContain('return `${CREATOR_PROFILE_BASE}/${encodeURIComponent(handle)}`;');
  });

  it('renders the listing CARD byline as a link to the creator profile', () => {
    const src = normalize(panel);
    // The card byline is an <a> carrying the author testid, pointed at the
    // creator-profile href and opening in the system browser.
    expect(src).toContain('class="author author-link"');
    expect(src).toContain('href={creatorProfileHref(listing)}');
    expect(src).toContain('data-testid="marketplace-author"');
    // It opens externally + safely (no opener), and a card-click is not hijacked
    // by the byline link (stopPropagation so the link opens the profile).
    expect(src).toContain('target="_blank"');
    expect(src).toContain('rel="noreferrer noopener"');
    expect(src).toContain('onclick={(event) => event.stopPropagation()}');
  });

  it('renders the DETAIL slide-over byline as a link to the creator profile', () => {
    const src = normalize(panel);
    // The detail-pane author byline is an <a> to the same creator-profile href.
    expect(src).toContain('href={creatorProfileHref(selected)}');
    expect(src).toContain('data-testid="marketplace-detail-author"');
  });

  it('degrades the byline to plain text when a listing has no author handle', () => {
    const src = normalize(panel);
    // Both surfaces gate the link on `creatorProfileHref(...)` and fall back to a
    // plain <span> byline (still carrying the testid) when there's no handle.
    expect(src).toContain('{#if creatorProfileHref(listing)}');
    expect(src).toContain('<span class="author" data-testid="marketplace-author">{authorLabel(listing)}</span>');
    expect(src).toContain('{#if creatorProfileHref(selected)}');
    expect(src).toContain('<p class="section-body" data-testid="marketplace-detail-author">{authorLabel(selected)}</p>');
  });
});

/**
 * US-019 — desktop install records install metrics (best-effort).
 *
 * After a SUCCESSFUL install, `MarketplacePanel.runInstall` must record an
 * install event via the authed `recordMarketplaceInstall` lib call (→ Rust
 * `record_marketplace_install`), passing the scope the user installed with — and
 * it must be FIRE-AND-FORGET so a metrics failure never fails or blocks the
 * install. Source-contract test (no DOM harness in this repo).
 */
describe('US-019: desktop install records install metrics (best-effort)', () => {
  it('imports the recordMarketplaceInstall lib helper', () => {
    expect(normalize(panel)).toContain('recordMarketplaceInstall');
  });

  it('records the install AFTER the install resolves, with the chosen scope, fire-and-forget', () => {
    const src = normalize(panel);
    // The recording call is sequenced inside runInstall's try block AFTER
    // `installMarketplacePack(...)` (success path), using the listing id + the
    // scope the user installed with (`target.scope`).
    const installIdx = src.indexOf('await installMarketplacePack(');
    const recordIdx = src.indexOf('recordMarketplaceInstall(selected.id, target.scope)');
    expect(installIdx).toBeGreaterThan(-1);
    expect(recordIdx).toBeGreaterThan(installIdx);
  });

  it('never awaits the metrics call and swallows its failure so it cannot block the install', () => {
    const src = normalize(panel);
    // `void` (not awaited) + `.catch(() => {})` = best-effort: a metrics write
    // failure can never reject runInstall or surface as an install error.
    expect(src).toContain('void recordMarketplaceInstall(selected.id, target.scope).catch(() => {});');
  });
});
