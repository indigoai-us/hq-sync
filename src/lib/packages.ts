/**
 * Type mirrors for the Packages window. These match the JSON shapes produced
 * by the `hq` CLI (`hq packs list --json`, `hq packages list --json`) and the
 * `packages.rs` Tauri command merge. Kept loose where the CLI payload is large
 * — only the fields the UI renders are typed.
 */

export type LinkCounts = {
  live: number;
  broken: number;
  missing: number;
  foreign: number;
};

/**
 * A pack's optional post-install `initialization` block (declared in
 * `package.yaml`, validated by the `hq` CLI on install — US-004). `entrypoint`
 * is a safe, author-declared skill/command name (resolves to a `contributes.*`
 * entry); `prompt` is OPTIONAL author free-text.
 *
 * The safe entrypoint-derived "get started" line is ALWAYS shown (Phase 1). The
 * free-text `prompt` is the pack author's full setup prose — an UNTRUSTED,
 * highest-trust-sensitivity blob the user is told to paste into their agent, so
 * it is a prime prompt-injection vector. It is rendered for copy/paste ONLY when
 * it is provably SAFE (US-009): the pack came from the moderated marketplace/
 * registry origin AND the prose carries an explicit, server-set
 * `promptModerated === true` approval signal (see `isPromptRenderable`).
 *
 * Surfaced to the frontend only once `hq packs list --json` includes it; when
 * absent (legacy packs, or an older CLI that doesn't emit it) the field is
 * `undefined` and the UI renders exactly as before.
 */
export interface PackInitialization {
  entrypoint: string;
  prompt?: string;
  /**
   * Server-set moderation-approval signal for `prompt` (US-009). When the
   * marketplace moderator approves a listing whose `initialization.prompt` was
   * injection-scanned (US-008), the server stamps this `true` on the pack's
   * surfaced init block so the Installed panel may offer the prose for
   * copy/paste.
   *
   * CONSERVATIVE DEFAULT: this enforcement is a known server-side follow-up and
   * is NOT emitted by the CLI today, so the field is OPTIONAL and treated as
   * `false` whenever absent. We never invent an "approved" assumption — absent
   * or non-`true` means the prose stays SUPPRESSED. Net effect today: prose is
   * suppressed for every pack until the server starts emitting this flag, which
   * is the safe default.
   */
  promptModerated?: boolean;
}

export interface InstalledPack {
  name: string;
  version?: string;
  publisher?: string;
  source?: string;
  transport: string | null;
  requiresHqCore?: string;
  hqCoreSatisfied: boolean | null;
  contributes: Record<string, number>;
  links: LinkCounts;
  brokenLinks: Array<{ key: string; item: string; dst: string }>;
  inCatalog: boolean;
  updateAvailable: boolean | null;
  initialization?: PackInitialization;
  error?: string;
}

export interface AvailablePack {
  source: string;
  description?: string;
  installed: false;
  conditional?: string;
  conditionalStatus: 'pass' | 'fail' | 'unevaluated' | 'none';
}

export interface PacksList {
  hqRoot: string;
  hqVersion: string | null;
  installed: InstalledPack[];
  available: AvailablePack[];
  warnings: string[];
}

export interface RegistryEntry {
  name?: string;
  slug: string;
  version?: string;
  scope?: string;
}

export interface RegistryAvailable {
  slug: string;
  tier?: string;
}

export interface RegistryList {
  installed: RegistryEntry[];
  available: RegistryAvailable[];
  offline: boolean;
}

/** The merged payload `list_packages` returns (and the `packages:updates` event). */
export interface PackagesView {
  packs: PacksList | null;
  registry: RegistryList | null;
  error: string | null;
}

/** Progress line emitted during install/update. */
export interface PackagesProgress {
  op: 'install' | 'update';
  name: string;
  line: string;
}

export interface PackagesDone {
  op: string;
  name: string;
  message?: string;
}

/** A short, human label for a pack source string (drops the long git prefix). */
export function shortSource(source: string | undefined): string {
  if (!source) return 'unknown source';
  // github:owner/repo#packages/hq-pack-x  ->  hq-pack-x
  const hashIdx = source.lastIndexOf('/');
  if (hashIdx >= 0 && source.includes('hq-pack-')) {
    const tail = source.slice(hashIdx + 1);
    return tail.split('@')[0];
  }
  return source;
}

/**
 * Was this pack installed from the MODERATED marketplace/registry origin?
 *
 * Only these origins go through marketplace moderation (injection scan +
 * reviewer approval), so only they are eligible to surface author prose. The
 * moderated install paths produce a `source` string with a `marketplace:` or
 * `registry:` scheme prefix (see `marketplace_source` in the Rust
 * `install_marketplace_pack`, which builds `marketplace:<slug>[@version]`, and
 * the registry install path).
 *
 * Everything else — a LOCAL filesystem path or a GIT URL (`github:…`,
 * `git+…`, `https://…`, a bare path) — was installed OUTSIDE moderation and is
 * NOT eligible. This is allowlist-by-scheme on purpose: anything we don't
 * positively recognise as a moderated origin is treated as un-moderated.
 *
 * Pure + DOM-free so the gate is unit-testable.
 */
export function isMarketplaceOrigin(source: string | undefined): boolean {
  const s = (source ?? '').trim().toLowerCase();
  return s.startsWith('marketplace:') || s.startsWith('registry:');
}

/**
 * Is a pack's author-written `initialization.prompt` SAFE to render for
 * copy/paste in the Installed panel? This is the heart of US-009's safety
 * contract — a pure predicate so the suppress/show decision is unit-tested
 * independent of Svelte.
 *
 * Renderable ONLY when ALL hold:
 *   1. The pack declares a non-empty `initialization.prompt` (there is prose).
 *   2. The pack came from the moderated marketplace/registry origin
 *      (`isMarketplaceOrigin`) — a local-path / git-URL install never qualifies.
 *   3. The prose carries the explicit server-set `promptModerated === true`
 *      approval signal.
 *
 * DEFAULT-SUPPRESS: any missing piece → `false`. Because the server does not
 * emit `promptModerated` yet (a known follow-up), condition 3 is false for
 * every pack today, so prose stays suppressed everywhere — the SAFE default.
 * We deliberately do NOT infer approval from origin alone.
 */
export function isPromptRenderable(pack: InstalledPack | undefined): boolean {
  const init = pack?.initialization;
  if (!init) return false;
  if (!init.prompt || init.prompt.trim().length === 0) return false;
  if (!isMarketplaceOrigin(pack?.source)) return false;
  return init.promptModerated === true;
}
