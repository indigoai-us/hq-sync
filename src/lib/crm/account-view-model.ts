/**
 * Pure read-model for the HQ Sync Accounts view (US-010, hq-native-crm).
 *
 * This module owns ALL of the Accounts surface's logic — grouping clients by
 * funnel stage, surfacing "needs attention" accounts, and assembling a single
 * end-to-end account-detail view — as pure functions over the vault-synced
 * `crm-projection.json`. Keeping every decision here (and out of the Svelte
 * component) makes the surface trivially unit-testable in the repo's `node`
 * vitest environment and guarantees the rendering path touches NO network: the
 * model is a pure transform of already-on-disk JSON, exactly like the read-only
 * Board surface reads `board.json`.
 *
 * The projection is produced server-side by hq-pro (US-009,
 * `src/ontology/crm/projection.ts`) and synced down to the company vault as
 * `companies/{co}/crm-projection.json`. The desktop app reads it the SAME way
 * it reads `board.json` (local-first scan + vault-API fallback) — there are no
 * calls to Attio / Stripe / PandaDoc / Neon anywhere on this surface.
 *
 * Field-name tolerance: the hq-pro builder emits snake_case (`synced_at`,
 * `external_ids`) while the design handoff sketched camelCase (`syncedAt`).
 * `normalizeProjection` accepts BOTH so the UI is forward/back-compatible with
 * either producer revision. Every leg is optional — a missing source/timeline/
 * leg degrades to an em-dash chip rather than throwing (graceful degradation,
 * AC #4).
 */

// ── Projection wire shape (tolerant superset of the US-009 emit) ─────────────

/** Funnel stage of an account (drives grouping + the stage pill). */
export type AccountStage =
  | 'lead'
  | 'demo'
  | 'demo_done'
  | 'proposal'
  | 'signed'
  | 'active';

/** The external system backing a source card on an account. */
export type AccountSourceSystem = 'neon' | 'attio' | 'pandadoc' | 'stripe';

/** One source-of-record leg on an account (Inbound/Pipeline/Contract/Billing). */
export interface AccountSource {
  system: AccountSourceSystem;
  status: string;
  value: string;
  meta: string;
  ref: string;
}

/** A timeline entry projected onto an account (meetings & signals). */
export interface ProjectedTimelineEntry {
  date: string;
  type: string;
  text: string;
  sourceMeetingRef: string;
}

/** The four named source legs, any of which may be absent (missing leg). */
export type AccountSourceRole = 'inbound' | 'pipeline' | 'contract' | 'billing';

/** External-system join keys for an account. */
export interface ExternalIds {
  attio?: string;
  stripe?: string;
  pandadoc?: string;
  neon?: string;
  [key: string]: string | undefined;
}

/** A single client account in the projection. */
export interface ProjectedAccount {
  id: string;
  name: string;
  domain: string;
  stage: AccountStage;
  owner: string;
  lastActivity: string;
  external_ids: ExternalIds;
  sources: Partial<Record<AccountSourceRole, AccountSource>>;
  timeline: ProjectedTimelineEntry[];
}

/** The full per-company CRM projection read off the vault. */
export interface CrmProjection {
  schema_version: number;
  accounts: ProjectedAccount[];
  /** ISO 8601 stamp this projection was generated (snake from the producer). */
  synced_at: string;
}

// ── Normalization (tolerant of the producer's exact field casing) ────────────

const STAGES: readonly AccountStage[] = [
  'lead',
  'demo',
  'demo_done',
  'proposal',
  'signed',
  'active',
];

/** Funnel display order for stage grouping + the lifecycle rail. */
export const STAGE_ORDER: readonly AccountStage[] = STAGES;

/** Human label for a funnel stage (used by group headers + the stage pill). */
export const STAGE_LABELS: Record<AccountStage, string> = {
  lead: 'Lead',
  demo: 'Demo',
  demo_done: 'Demo done',
  proposal: 'Proposal',
  signed: 'Signed',
  active: 'Active',
};

function str(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function coerceStage(raw: unknown): AccountStage {
  const v = str(raw).trim().toLowerCase().replace(/-/g, '_');
  return (STAGES as readonly string[]).includes(v) ? (v as AccountStage) : 'lead';
}

function normalizeSource(raw: unknown): AccountSource | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const system = str(r.system).toLowerCase();
  const okSystems: readonly string[] = ['neon', 'attio', 'pandadoc', 'stripe'];
  if (!okSystems.includes(system)) return null;
  return {
    system: system as AccountSourceSystem,
    status: str(r.status),
    value: str(r.value),
    meta: str(r.meta),
    ref: str(r.ref),
  };
}

function normalizeTimelineEntry(raw: unknown): ProjectedTimelineEntry {
  const r = (raw ?? {}) as Record<string, unknown>;
  return {
    date: str(r.date),
    type: str(r.type),
    // Accept either `text` (UI shape) or `preview` (the US-008 timeline shape).
    text: str(r.text) || str(r.preview),
    // Accept either `sourceMeetingRef` (handoff) or `sourceRef` (US-008).
    sourceMeetingRef: str(r.sourceMeetingRef) || str(r.sourceRef),
  };
}

function normalizeExternalIds(raw: unknown): ExternalIds {
  if (!raw || typeof raw !== 'object') return {};
  const out: ExternalIds = {};
  for (const [k, v] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof v === 'string' && v.length > 0) out[k] = v;
  }
  return out;
}

function normalizeAccount(raw: unknown): ProjectedAccount | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const id = str(r.id);
  const name = str(r.name);
  // An account with neither id nor name is unusable — drop it rather than render
  // a blank row.
  if (!id && !name) return null;

  const sources: Partial<Record<AccountSourceRole, AccountSource>> = {};
  const rawSources = (r.sources ?? {}) as Record<string, unknown>;
  for (const role of ['inbound', 'pipeline', 'contract', 'billing'] as const) {
    const src = normalizeSource(rawSources[role]);
    if (src) sources[role] = src;
  }

  const timeline = Array.isArray(r.timeline)
    ? r.timeline.map(normalizeTimelineEntry)
    : [];

  return {
    id: id || name,
    name: name || id,
    domain: str(r.domain),
    stage: coerceStage(r.stage),
    owner: str(r.owner),
    lastActivity: str(r.lastActivity) || str(r.last_activity),
    external_ids: normalizeExternalIds(r.external_ids ?? r.externalIds),
    sources,
    timeline,
  };
}

/**
 * Normalize a raw, loosely-typed projection payload (from the local-first scan
 * or the vault API) into a well-formed {@link CrmProjection}. Tolerant of
 * snake/camel casing, missing collections, and a missing/garbage leg. A
 * null/garbage payload yields an EMPTY projection (no accounts) rather than
 * throwing — the surface renders its empty state.
 */
export function normalizeProjection(raw: unknown): CrmProjection {
  const r = (raw ?? {}) as Record<string, unknown>;
  const accounts = Array.isArray(r.accounts)
    ? r.accounts
        .map(normalizeAccount)
        .filter((a): a is ProjectedAccount => a !== null)
    : [];
  const schemaVersion =
    typeof r.schema_version === 'number'
      ? r.schema_version
      : typeof r.schemaVersion === 'number'
        ? r.schemaVersion
        : 0;
  return {
    schema_version: schemaVersion,
    accounts,
    synced_at: str(r.synced_at) || str(r.syncedAt),
  };
}

// ── Status-dot tone (the only color on the surface, per indigo-no-purple) ────

/** A status-dot tone from the V4 palette — maps to `--v4-ok/warn/error/idle`. */
export type DotTone = 'ok' | 'warn' | 'error' | 'idle';

/**
 * Map a free-form source/contract/billing status string onto a V4 status-dot
 * tone. Affirmative states (paid/signed/active/won) → green `ok`; in-flight
 * (sent/open/pending/draft) → amber `warn`; failed (overdue/past_due/failed) →
 * red `error`; unknown/empty → grey `idle`. Monochrome elsewhere — the dot is
 * the ONLY color, never purple (hard policy indigo-no-purple).
 */
export function statusTone(status: string | undefined): DotTone {
  const s = (status ?? '').trim().toLowerCase();
  if (!s) return 'idle';
  if (/(paid|signed|active|won|complete|completed|live|granted)/.test(s)) {
    return 'ok';
  }
  if (/(overdue|past_due|past due|failed|canceled|cancelled|lost|error|declined)/.test(s)) {
    return 'error';
  }
  if (/(sent|open|pending|draft|proposal|review|demo|processing|in_progress)/.test(s)) {
    return 'warn';
  }
  return 'idle';
}

// ── Stage grouping (the list view) ───────────────────────────────────────────

/** A stage group in the list view: a funnel stage + its accounts. */
export interface AccountStageGroup {
  stage: AccountStage;
  label: string;
  accounts: ProjectedAccount[];
}

/**
 * Group accounts by funnel stage in funnel order, dropping empty stages. Within
 * a stage, accounts keep the projection's (already deterministic) order; we sort
 * by name as a stable tie-break so the list never depends on scan order.
 */
export function groupAccountsByStage(
  accounts: ProjectedAccount[],
): AccountStageGroup[] {
  const byStage = new Map<AccountStage, ProjectedAccount[]>();
  for (const account of accounts) {
    const list = byStage.get(account.stage) ?? [];
    list.push(account);
    byStage.set(account.stage, list);
  }
  const groups: AccountStageGroup[] = [];
  for (const stage of STAGE_ORDER) {
    const list = byStage.get(stage);
    if (!list || list.length === 0) continue;
    list.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
    groups.push({ stage, label: STAGE_LABELS[stage], accounts: list });
  }
  return groups;
}

// ── Needs-attention surfacing (open action-items from signals) ───────────────

/** Activity types in an account timeline that represent an OPEN obligation. */
const OPEN_ACTION_TYPES = new Set(['action_item', 'commitment', 'risk']);

/** A "needs attention" row: an account with open action-items from signals. */
export interface NeedsAttentionItem {
  account: ProjectedAccount;
  /** Count of open action-items / commitments / risks on the timeline. */
  openCount: number;
  /** The most-recent open item's text (the headline reason). */
  reason: string;
  /** Citation to the source meeting for the headline reason. */
  sourceMeetingRef: string;
}

/**
 * Open action-items on an account = timeline entries whose type is an open
 * obligation (action_item / commitment / risk). The Phase-1 projection does not
 * yet carry a dedicated `needsAttention[]` collection (that is a Batch-3 derived
 * read-model, see design-handoff), so the surface derives it from the per-account
 * activity timeline (US-008 signals) — which is exactly what "open action-items
 * from signals" means in AC #2.
 */
export function openActionItems(
  account: ProjectedAccount,
): ProjectedTimelineEntry[] {
  return account.timeline.filter((entry) =>
    OPEN_ACTION_TYPES.has(entry.type.trim().toLowerCase()),
  );
}

/**
 * Surface the accounts that need attention — those carrying open action-items
 * from signals — most-pressing first (by open-item count, then most-recent
 * activity). Accounts with zero open items are omitted.
 */
export function needsAttention(
  accounts: ProjectedAccount[],
): NeedsAttentionItem[] {
  const items: NeedsAttentionItem[] = [];
  for (const account of accounts) {
    const open = openActionItems(account);
    if (open.length === 0) continue;
    // Headline = the most-recent open item (timeline is most-recent-first).
    const headline = open[0];
    items.push({
      account,
      openCount: open.length,
      reason: headline.text,
      sourceMeetingRef: headline.sourceMeetingRef,
    });
  }
  items.sort((a, b) => {
    if (b.openCount !== a.openCount) return b.openCount - a.openCount;
    return b.account.lastActivity.localeCompare(a.account.lastActivity);
  });
  return items;
}

// ── Account detail (the single end-to-end surface) ───────────────────────────

/** One source card on the detail view, with its role label + resolved tone. */
export interface DetailSourceCard {
  role: AccountSourceRole;
  /** Human role label: "Inbound · Neon", "Billing · Stripe", … */
  label: string;
  /** The source leg, or null when the leg is missing (renders an em-dash). */
  source: AccountSource | null;
  tone: DotTone;
}

/** One step of the lifecycle rail with its position relative to the current stage. */
export interface LifecycleStep {
  stage: AccountStage;
  label: string;
  state: 'past' | 'current' | 'future';
}

/** The fully-assembled account-detail read-model (the US-010 centerpiece). */
export interface AccountDetail {
  account: ProjectedAccount;
  /** Demo origin (the Inbound·Neon leg) — the "where did this client come from". */
  demoOrigin: AccountSource | null;
  /** Pipeline stage (the funnel stage as a labeled pill). */
  stageLabel: string;
  /** The lifecycle rail Demo → Demo done → … → Active with the current step. */
  lifecycle: LifecycleStep[];
  /** Contract status leg (PandaDoc). */
  contract: AccountSource | null;
  /** Latest invoice / billing leg (Stripe). */
  latestInvoice: AccountSource | null;
  /** The four source cards (Inbound / Pipeline / Contract / Billing). */
  sourceCards: DetailSourceCard[];
  /** Chronological (most-recent-first) meetings & signals activity. */
  timeline: ProjectedTimelineEntry[];
}

const SOURCE_CARD_LABELS: Record<AccountSourceRole, string> = {
  inbound: 'Inbound · Neon',
  pipeline: 'Pipeline · Attio',
  contract: 'Contract · PandaDoc',
  billing: 'Billing · Stripe',
};

/** The lifecycle rail stages, in design order (Demo → … → Active). */
const LIFECYCLE_STAGES: readonly AccountStage[] = [
  'demo',
  'demo_done',
  'proposal',
  'signed',
  'active',
];

function buildLifecycle(stage: AccountStage): LifecycleStep[] {
  // A "lead" sits before the rail entirely — every step is future.
  const currentIndex = LIFECYCLE_STAGES.indexOf(stage);
  return LIFECYCLE_STAGES.map((s, i) => {
    let state: LifecycleStep['state'];
    if (currentIndex === -1) state = 'future';
    else if (i < currentIndex) state = 'past';
    else if (i === currentIndex) state = 'current';
    else state = 'future';
    return { stage: s, label: STAGE_LABELS[s], state };
  });
}

/**
 * Assemble the single end-to-end account-detail surface from one projected
 * account: demo origin, pipeline stage, contract status, latest invoice, and the
 * meetings/signals activity timeline — all from the projection, no network. A
 * missing leg resolves to `null` (the card renders an em-dash chip).
 */
export function buildAccountDetail(account: ProjectedAccount): AccountDetail {
  const sourceCards: DetailSourceCard[] = (
    ['inbound', 'pipeline', 'contract', 'billing'] as const
  ).map((role) => {
    const source = account.sources[role] ?? null;
    return {
      role,
      label: SOURCE_CARD_LABELS[role],
      source,
      tone: statusTone(source?.status),
    };
  });

  return {
    account,
    demoOrigin: account.sources.inbound ?? null,
    stageLabel: STAGE_LABELS[account.stage],
    lifecycle: buildLifecycle(account.stage),
    contract: account.sources.contract ?? null,
    latestInvoice: account.sources.billing ?? null,
    sourceCards,
    timeline: account.timeline,
  };
}

/** Find one account in a projection by id (the detail-view selector). */
export function findAccount(
  projection: CrmProjection,
  accountId: string,
): ProjectedAccount | null {
  return projection.accounts.find((a) => a.id === accountId) ?? null;
}

// ── Small display helpers (shared by list + detail) ──────────────────────────

/** The canonical em-dash a missing leg / value renders as (graceful degrade). */
export const EM_DASH = '—';

/** Display a possibly-empty value, falling back to the em-dash. */
export function orDash(value: string | null | undefined): string {
  const v = (value ?? '').trim();
  return v.length > 0 ? v : EM_DASH;
}

/** Human relative timestamp ("just now", "5m ago") for activity/synced lines. */
export function relativeTime(
  iso: string | null | undefined,
  now: number = Date.now(),
): string {
  const trimmed = (iso ?? '').trim();
  if (!trimmed) return EM_DASH;
  const then = new Date(trimmed).getTime();
  if (Number.isNaN(then)) return EM_DASH;
  const secs = Math.max(0, Math.round((now - then) / 1000));
  if (secs < 60) return 'just now';
  const mins = Math.round(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.round(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.round(hrs / 24)}d ago`;
}
