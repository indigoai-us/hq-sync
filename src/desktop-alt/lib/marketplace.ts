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

// ---------------------------------------------------------------------------
// US-012 — moderation queue + approve/reject (admin reviewer surface).
// ---------------------------------------------------------------------------

/** One natural-language injection-scan flag (mirrors Rust `InjectionFlag`). */
export interface InjectionFlag {
  /** Which instruction file the flag is over (e.g. `skills/foo/SKILL.md`). */
  file: string;
  /** Start char offset into the instruction text. */
  start: number;
  /** End char offset into the instruction text. */
  end: number;
  /** The flagged text itself, when the server echoes it. */
  snippet: string;
  /** Why the span was flagged (the rule that matched). */
  reason: string;
}

/** One pack instruction document (SKILL.md / worker prose) under review. */
export interface InstructionDoc {
  /** File path within the pack. */
  path: string;
  /** The instruction text to display + highlight. */
  text: string;
}

/**
 * One pending_review listing in the moderation queue (mirrors Rust
 * `ModerationQueueItem` 1:1). A superset of `MarketplaceListing` with the
 * moderation-only fields a reviewer needs.
 */
export interface ModerationQueueItem {
  id: string;
  type: string;
  name: string;
  slug: string;
  version: string;
  author: string;
  summary?: string | null;
  contributes?: string | null;
  /** ISO-8601 submission timestamp (queue order). */
  submittedAt: string;
  /** Tarball-contents preview — the pack's file paths. */
  files: string[];
  /** Natural-language instruction docs the reviewer must read for injection. */
  instructions: InstructionDoc[];
  /** Advisory natural-language injection-scan flags (over `instructions`). */
  injectionScan: InjectionFlag[];
  /** Opaque optimistic-lock token forwarded back on decide. */
  versionLock?: string | null;
}

/** Outcome of a moderation decision (mirrors Rust `ModerationDecisionResult`). */
export interface ModerationDecisionResult {
  id: string;
  /** `"approved"` | `"rejected"` on success. */
  status: string;
  note: string;
}

/** The reviewer's decision verb. */
export type ModerationDecision = 'approve' | 'reject';

/**
 * Load the moderation queue (pending_review listings). Admin-gated SERVER-SIDE;
 * a non-admin caller gets a clear "admin only" error (the panel locks). The UI
 * admin gate (`isAdminGate`) is UX only — this is not the authorization boundary.
 */
export async function loadModerationQueue(): Promise<ModerationQueueItem[]> {
  return invoke<ModerationQueueItem[]>('list_moderation_queue');
}

/**
 * Approve or reject a pending listing. `note` is optional (recorded for audit;
 * conventionally required by the UI for a reject). `versionLock` is forwarded so
 * a concurrent approve+reject race is a 409, not a silent inconsistency.
 */
export async function decideModerationListing(
  id: string,
  decision: ModerationDecision,
  note?: string | null,
  versionLock?: string | null,
): Promise<ModerationDecisionResult> {
  return invoke<ModerationDecisionResult>('decide_moderation_listing', {
    id,
    decision,
    note: note?.trim() ? note.trim() : null,
    versionLock: versionLock?.trim() ? versionLock.trim() : null,
  });
}

/**
 * UI admin gate (UX ONLY — the server is the real authority). Default-DENY:
 * returns true only when the email is positively known to end in
 * `@getindigo.ai`. An unknown / absent / malformed email → false (panel locked).
 * The leading `@` blocks look-alikes like `forgetindigo.ai`. Kept in lockstep
 * with the Rust `feature_gate::is_allowed_email`.
 */
export function isAdminGate(email: string | null | undefined): boolean {
  const e = (email ?? '').trim().toLowerCase();
  return e.length > 0 && e.endsWith('@getindigo.ai');
}

/**
 * Whether Approve is permitted for an item, given the reviewer's instruction-
 * injection acknowledgement and the in-flight state. This is the GATE for AC4:
 * Approve is DISABLED until the reviewer explicitly acks the instruction review.
 * Pure so the gate is unit-tested independent of Svelte.
 */
export function canApprove(opts: {
  /** The reviewer ticked "I reviewed the instructions for prompt-injection". */
  acknowledged: boolean;
  /** A decide call is already in flight for this item. */
  busy: boolean;
}): boolean {
  return opts.acknowledged && !opts.busy;
}

/** A contiguous run of instruction text — `flagged` runs are injection spans. */
export interface HighlightSegment {
  text: string;
  flagged: boolean;
  /** The flag reason, when this segment is flagged (for a tooltip / label). */
  reason?: string;
}

/**
 * Split an instruction document's text into highlight segments using the
 * injection-scan flags whose `file` matches `path`. Overlapping / out-of-range /
 * unordered flags are normalised (clamped, sorted, merged) so rendering can't
 * crash or mis-slice. A flag with no usable offsets (start>=end) is ignored for
 * slicing (its `snippet`/`reason` still surface in the flag list elsewhere).
 *
 * Pure + deterministic so the highlighting is unit-tested without a DOM.
 */
export function highlightInstruction(
  doc: InstructionDoc,
  flags: InjectionFlag[],
): HighlightSegment[] {
  const text = doc.text ?? '';
  const len = text.length;
  if (len === 0) return [];

  // Keep only flags over THIS doc with a usable, clamped range.
  const ranges = flags
    .filter((f) => f.file === doc.path)
    .map((f) => ({
      start: Math.max(0, Math.min(f.start | 0, len)),
      end: Math.max(0, Math.min(f.end | 0, len)),
      reason: f.reason,
    }))
    .filter((r) => r.end > r.start)
    .sort((a, b) => a.start - b.start || a.end - b.end);

  if (ranges.length === 0) {
    return [{ text, flagged: false }];
  }

  const segments: HighlightSegment[] = [];
  let cursor = 0;
  for (const r of ranges) {
    // Skip a range fully swallowed by an earlier (already-emitted) one.
    if (r.end <= cursor) continue;
    const start = Math.max(r.start, cursor);
    if (start > cursor) {
      segments.push({ text: text.slice(cursor, start), flagged: false });
    }
    segments.push({ text: text.slice(start, r.end), flagged: true, reason: r.reason });
    cursor = r.end;
  }
  if (cursor < len) {
    segments.push({ text: text.slice(cursor), flagged: false });
  }
  return segments;
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

/**
 * Record an install event for the marketplace metrics (US-019). Forwards to the
 * authed Rust `record_marketplace_install` command, which attaches the caller's
 * bearer token and POSTs `/v1/listings/{id}/installs` (the installer uid is
 * derived server-side from the token; the body carries the install scope).
 *
 * BEST-EFFORT / fire-and-forget: callers invoke this AFTER a successful install
 * and must NOT let a metrics failure fail or block the install — wrap the call in
 * `.catch(() => {})`. The promise still rejects on failure so a test can observe
 * it; production code simply ignores the outcome.
 */
export async function recordMarketplaceInstall(
  listingId: string,
  scope: InstallScope,
): Promise<void> {
  return invoke<void>('record_marketplace_install', { listingId, scope });
}

// ---------------------------------------------------------------------------
// US-013 — desktop Submit tab (publish a local pack via `hq publish`).
// ---------------------------------------------------------------------------

/** Successful publish outcome (mirrors Rust `PublishResult` 1:1, camelCase). */
export interface PublishResult {
  /** The created listing id (parsed from the CLI success notice). */
  listingId: string;
  /** Listing status — `pending_review` for a new submission. */
  status: string;
  /** Raw CLI success notice (shown as confirmation prose). */
  notice: string;
}

/**
 * Classified publish FAILURE (mirrors Rust `PublishError`). `notVerified`
 * distinguishes the verified-creator gate (→ request-access prompt) from an
 * ordinary validation / network error (shown inline as-is). The Rust command
 * rejects the IPC promise with this object as its payload.
 */
export interface PublishError {
  message: string;
  notVerified: boolean;
}

/**
 * Type-guard: was an `invoke('publish_marketplace_pack')` rejection a structured
 * `PublishError`? Tauri rejects with the serialized error value, so a typed
 * error arrives as a plain object — not an `Error` instance. Anything else
 * (string, Error, undefined) is coerced by `toPublishError`.
 */
export function isPublishError(value: unknown): value is PublishError {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { message?: unknown }).message === 'string' &&
    typeof (value as { notVerified?: unknown }).notVerified === 'boolean'
  );
}

/**
 * Pure classifier mirroring Rust `is_not_verified_error`: does this error text
 * describe the verified-creator gate (US-011)? Case-insensitive.
 */
export function looksNotVerified(message: string): boolean {
  const m = message.toLowerCase();
  return (
    m.includes('not_verified_creator') ||
    m.includes('verified creator') ||
    m.includes('creator account is verified') ||
    m.includes('not authorized to publish') ||
    (m.includes('verified') && m.includes('publish'))
  );
}

/**
 * Normalise ANY publish rejection into a `PublishError`. A structured rejection
 * passes through; a bare string / Error is wrapped (notVerified=false) unless its
 * text matches the verified-creator gate, in which case it's classified so the
 * UI still shows the request-access prompt.
 */
export function toPublishError(value: unknown): PublishError {
  if (isPublishError(value)) return value;
  const message =
    value instanceof Error
      ? value.message
      : typeof value === 'string'
        ? value
        : 'Publish failed.';
  return { message, notVerified: looksNotVerified(message) };
}

/**
 * Publish a local skill/worker directory to the marketplace via the `hq publish`
 * flow (US-004). Resolves with the listing id + `pending_review` status; rejects
 * with a structured `PublishError` (use `toPublishError` to normalise). Progress
 * lines stream via the `marketplace:publish-progress` event.
 */
export async function publishMarketplacePack(path: string): Promise<PublishResult> {
  return invoke<PublishResult>('publish_marketplace_pack', { path });
}

/**
 * Open the native folder picker for the Submit flow. Resolves to the chosen
 * directory path, or `null` if the user cancelled.
 */
export async function pickPackDirectory(): Promise<string | null> {
  return invoke<string | null>('pick_pack_directory');
}

/**
 * Request verified-creator access (the unverified Submit affordance, US-011).
 * Forwards the applicant's pitch (`reason`) and an optional `handle` to the
 * authed Rust `request_creator_access` command, which POSTs
 * `/v1/creators/request-access` (`{ reason, handle? }`). Returns the server's
 * human guidance message on success.
 *
 * A 409 `APPLICATION_PENDING` (the applicant already has a pending application)
 * is surfaced by the Rust command as a rejection whose message matches
 * `looksApplicationPending` — the panel reads that to render the duplicate state.
 */
export async function requestCreatorAccess(
  reason?: string | null,
  handle?: string | null,
): Promise<string> {
  return invoke<string>('request_creator_access', {
    reason: reason?.trim() ? reason.trim() : null,
    handle: handle?.trim() ? handle.trim() : null,
  });
}

/**
 * Pure classifier: does this error text describe the "already have a pending
 * application" (409 `APPLICATION_PENDING`) state? Lets the Submit panel render a
 * calm duplicate notice instead of an alarming error. Case-insensitive.
 */
export function looksApplicationPending(message: string): boolean {
  const m = message.toLowerCase();
  return m.includes('application_pending') || m.includes('pending application');
}

// ---------------------------------------------------------------------------
// Creator-application review funnel — admin queue + approve/deny.
// ---------------------------------------------------------------------------

/** One pending creator-access application (mirrors Rust `CreatorApplication`). */
export interface CreatorApplication {
  /** Stable application id — the decide key. */
  applicationId: string;
  /** The applicant's internal person uid (opaque to the UI). */
  applicantUid: string;
  /** The applicant's email (the primary display key in the queue row). */
  applicantEmail: string;
  /** The handle the applicant wants, when supplied. */
  handle: string;
  /** The applicant's pitch (why they want creator access). */
  reason: string;
  /** Application status — `pending` for everything in this queue. */
  status: string;
  /** ISO-8601 submission timestamp (queue order, oldest-first). */
  submittedAt: string;
}

/** Outcome of an application decision (mirrors Rust `ApplicationDecisionResult`). */
export interface ApplicationDecisionResult {
  applicationId: string;
  /** `"approved"` | `"denied"` on success (server-reported). */
  status: string;
  reviewedBy: string;
  reviewedAt: string;
}

/** The reviewer's application decision verb. */
export type ApplicationDecision = 'approve' | 'deny';

/**
 * Load pending creator-access applications (admin-gated SERVER-SIDE; a non-admin
 * gets a clear "admin only" error so the panel locks its Requests view). The UI
 * admin gate (`isAdminGate`) is UX only — this is not the authorization boundary.
 */
export async function loadCreatorApplications(): Promise<CreatorApplication[]> {
  return invoke<CreatorApplication[]>('list_creator_applications');
}

/**
 * Approve or deny a pending application. `note` is optional (recorded for audit;
 * conventionally required by the UI for a deny). On success the row is dropped
 * from the local queue.
 */
export async function decideCreatorApplication(
  id: string,
  decision: ApplicationDecision,
  note?: string | null,
): Promise<ApplicationDecisionResult> {
  return invoke<ApplicationDecisionResult>('decide_creator_application', {
    id,
    decision,
    note: note?.trim() ? note.trim() : null,
  });
}

// ---------------------------------------------------------------------------
// US-016 — desktop Profile tab (claim handle, edit profile, upload avatar).
// ---------------------------------------------------------------------------

/** Successful handle claim (mirrors Rust `ClaimResult` 1:1, camelCase). */
export interface ClaimResult {
  /** The claimed handle (the creator entity slug). */
  handle: string;
  /** The created creator entity's internal uid — opaque to the UI. */
  uid: string;
  /** ISO-8601 claim timestamp. */
  createdAt: string;
}

/**
 * Classified handle-claim FAILURE (mirrors Rust `ClaimError`). `taken` is true
 * for the duplicate (409) case so the panel shows a focused "unavailable"
 * affordance; `code` carries the server's stable reason code
 * (`HANDLE_FORMAT_INVALID` | `HANDLE_RESERVED` | `HANDLE_CONFUSABLE` |
 * `HANDLE_ALREADY_CLAIMED` | …). The Rust command rejects the IPC promise with
 * this object as its payload.
 */
export interface ClaimError {
  message: string;
  code: string;
  taken: boolean;
}

/** One social link on a creator profile (mirrors the server `SocialLink`). */
export interface SocialLink {
  label: string;
  url: string;
}

/** The merged creator profile echoed after an update / nested in the preview. */
export interface CreatorProfile {
  handle: string;
  /** Display name (public preview only; the authed echo omits it). */
  displayName?: string;
  bio?: string | null;
  tipUrl?: string | null;
  socialLinks: SocialLink[];
  avatarUrl?: string | null;
}

/** The public profile preview — the redacted profile + approved listings. */
export interface PublicCreatorPreview {
  creator: CreatorProfile;
  listings: MarketplaceListing[];
}

/**
 * Type-guard: was an `invoke('claim_creator_handle')` rejection a structured
 * `ClaimError`? Tauri rejects with the serialized error value, so a typed error
 * arrives as a plain object — not an `Error` instance.
 */
export function isClaimError(value: unknown): value is ClaimError {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { message?: unknown }).message === 'string' &&
    typeof (value as { taken?: unknown }).taken === 'boolean'
  );
}

/**
 * Normalise ANY claim rejection into a `ClaimError`. A structured rejection
 * passes through; a bare string / Error is wrapped (taken=false). Used so the
 * panel always has a `taken` flag + message to render, even on a transport error.
 */
export function toClaimError(value: unknown): ClaimError {
  if (isClaimError(value)) return value;
  const message =
    value instanceof Error
      ? value.message
      : typeof value === 'string'
        ? value
        : 'Claim failed.';
  return { message, code: '', taken: false };
}

/**
 * Pure client-side handle FORMAT check, mirroring the server's
 * `validateHandleFormat` (3–30 chars, lowercase alnum + `-`/`_`, no leading/
 * trailing/consecutive separators). Returns the normalised (trimmed,
 * lowercased) handle on success, else a human reason. This is a FAST-FEEDBACK
 * hint only — the authoritative check is the server claim (which also screens
 * reserved/confusable + uniqueness). Pure so it's unit-tested without a DOM.
 */
export function checkHandleFormat(
  raw: string,
): { ok: true; handle: string } | { ok: false; reason: string } {
  const handle = raw.trim().toLowerCase();
  if (handle.length === 0) {
    return { ok: false, reason: 'Enter a handle.' };
  }
  if (handle.length < 3 || handle.length > 30) {
    return { ok: false, reason: 'Handle must be 3–30 characters.' };
  }
  if (!/^[a-z0-9_-]+$/.test(handle)) {
    return {
      ok: false,
      reason: 'Only lowercase letters, numbers, hyphens, and underscores.',
    };
  }
  if (/^[-_]|[-_]$/.test(handle)) {
    return { ok: false, reason: 'Cannot start or end with a hyphen or underscore.' };
  }
  if (/[-_]{2,}/.test(handle)) {
    return { ok: false, reason: 'No consecutive hyphens or underscores.' };
  }
  return { ok: true, handle };
}

/**
 * Pure client-side URL hint, mirroring the server's `validateProfileUrl`
 * allowlist (absolute http(s) only). FAST-FEEDBACK only — the server is the
 * authority. An empty string is treated as "no URL" (valid) so an optional
 * field isn't flagged while blank.
 */
export function checkHttpUrl(raw: string): { ok: true } | { ok: false; reason: string } {
  const url = raw.trim();
  if (url.length === 0) return { ok: true };
  let protocol: string;
  try {
    protocol = new URL(url).protocol;
  } catch {
    return { ok: false, reason: 'Must be an absolute http(s) URL.' };
  }
  if (protocol !== 'http:' && protocol !== 'https:') {
    return { ok: false, reason: 'Only http(s) links are allowed.' };
  }
  return { ok: true };
}

/**
 * Claim a creator handle. Resolves with the claimed handle; rejects with a
 * structured `ClaimError` (use `toClaimError` to normalise) whose `taken`/`code`
 * fields let the panel surface "unavailable" vs. the format/reserved reason.
 */
export async function claimCreatorHandle(handle: string): Promise<ClaimResult> {
  return invoke<ClaimResult>('claim_creator_handle', { handle: handle.trim() });
}

/**
 * Update the caller's OWN creator profile. Only the supplied fields are sent
 * (the server does a partial merge: absent = unchanged, empty string/array =
 * clear). Every URL is http(s)-validated server-side. Returns the merged profile.
 */
export async function updateCreatorProfile(patch: {
  bio?: string;
  socialLinks?: SocialLink[];
  tipUrl?: string;
}): Promise<CreatorProfile> {
  return invoke<CreatorProfile>('update_creator_profile', {
    bio: patch.bio ?? null,
    socialLinks: patch.socialLinks ?? null,
    tipUrl: patch.tipUrl ?? null,
  });
}

/**
 * Upload the caller's OWN avatar from a local file path (image-only, ≤2 MiB —
 * enforced server-side and pre-checked in Rust). Resolves with the presigned
 * avatar URL.
 */
export async function uploadCreatorAvatar(filePath: string): Promise<string> {
  return invoke<string>('upload_creator_avatar', { filePath });
}

/** Open the native image picker for an avatar. Resolves to the path or `null`. */
export async function pickAvatarFile(): Promise<string | null> {
  return invoke<string | null>('pick_avatar_file');
}

/** Fetch a creator's PUBLIC profile + approved listings for the preview. */
export async function getCreatorProfile(handle: string): Promise<PublicCreatorPreview> {
  return invoke<PublicCreatorPreview>('get_creator_profile', { handle: handle.trim() });
}
