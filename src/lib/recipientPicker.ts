// Pure helpers for the New Message recipient picker (US-010).
//
// The picker autocompletes a recipient from three sources, in priority order:
//   (a) known contacts        — list_contacts
//   (b) per-company members    — list_company_members for each of the caller's
//                                companies, grouped "From {companyName}"
//   (c) a free-text email row  — "Send to {email}" when the typed string is a
//                                valid email not already present in (a) or (b)
//
// Keeping the matching/grouping/dedupe logic here (not inside the .svelte
// component) makes it unit-testable without a DOM. The component owns the
// invoke() calls, keyboard handling, and rendering.

/** Connection state of a recipient relative to the caller. */
export type ConnectionState = 'active' | 'pending' | 'none' | 'blocked';

/** A person the caller can address, as returned by list_contacts /
 * list_company_members. Mirrors the Rust `Contact` wire shape (camelCase). The
 * server now tags each contact with a `connectionState`; older payloads that
 * omit it are treated as 'none' (not connected → request flow). */
export interface ContactLike {
  personUid: string;
  email: string;
  displayName?: string;
  companyUid?: string | null;
  source?: string | null;
  connectionState?: ConnectionState;
}

/** The recipient the picker emits once one is chosen. */
export interface SelectedRecipient {
  personUid?: string;
  email: string;
  displayName?: string;
  connectionState: ConnectionState;
}

/** A group of suggestion rows under a heading. `key` is stable for keying. */
export interface SuggestionGroup {
  key: string;
  /** Heading shown above the group, e.g. "Contacts" or "From Acme Inc." */
  label: string;
  rows: SuggestionRow[];
}

/** One selectable row in the dropdown. `freeText` rows are the "Send to
 * {email}" affordance for an unknown but valid email. */
export interface SuggestionRow {
  recipient: SelectedRecipient;
  /** Primary label (display name or email). */
  primary: string;
  /** Secondary label (email when distinct from primary), or null. */
  secondary: string | null;
  /** True for the synthetic "Send to {email}" free-text row. */
  freeText: boolean;
}

/**
 * RFC-pragmatic email check — good enough to decide whether to offer a
 * "Send to {email}" row. Intentionally simple: one @, non-empty local part, a
 * dotted domain, no spaces.
 */
export function isValidEmail(s: string): boolean {
  const v = s.trim();
  if (!v || /\s/.test(v)) return false;
  return /^[^@]+@[^@]+\.[^@]+$/.test(v);
}

/** Normalize a contact's connectionState, defaulting absent → 'none'. */
function stateOf(c: ContactLike): ConnectionState {
  return c.connectionState ?? 'none';
}

function labelOf(c: ContactLike): string {
  return c.displayName?.trim() || c.email?.trim() || c.personUid;
}

/** True when the contact matches the query on display name or email
 * (case-insensitive substring). An empty query matches everything. */
export function matchesQuery(c: ContactLike, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const name = (c.displayName ?? '').toLowerCase();
  const email = (c.email ?? '').toLowerCase();
  return name.includes(q) || email.includes(q);
}

function toRow(c: ContactLike): SuggestionRow {
  const name = c.displayName?.trim();
  const email = c.email?.trim();
  return {
    recipient: {
      personUid: c.personUid,
      email: email ?? '',
      displayName: name || undefined,
      connectionState: stateOf(c),
    },
    primary: labelOf(c),
    secondary: name && email ? email : null,
    freeText: false,
  };
}

/** A company the caller belongs to — used to label per-company member groups. */
export interface CompanyInfo {
  companyUid: string;
  companyName: string | null;
}

export interface BuildSuggestionsInput {
  query: string;
  /** Known contacts from list_contacts. */
  contacts: ContactLike[];
  /** Members keyed by companyUid, from list_company_members per company. */
  membersByCompany: Record<string, ContactLike[]>;
  /** The caller's companies, for group headings (ordered). */
  companies: CompanyInfo[];
  /** Cap on rows per group (keeps the dropdown bounded). */
  limitPerGroup?: number;
}

/**
 * Build the grouped suggestion list for the current query.
 *
 * Priority + dedupe rules:
 *   1. Contacts group first (source (a)).
 *   2. One group per company (source (b)), in `companies` order, each labeled
 *      "From {companyName}". A person already shown in Contacts is NOT repeated
 *      in a company group (dedupe by personUid, then by lowercased email).
 *   3. A free-text "Send to {email}" row (source (c)) ONLY when the query is a
 *      valid email that does not already appear in any earlier group.
 * Empty groups are omitted.
 */
export function buildSuggestions(input: BuildSuggestionsInput): SuggestionGroup[] {
  const { query, contacts, membersByCompany, companies, limitPerGroup = 6 } = input;
  const groups: SuggestionGroup[] = [];

  // Track who's already been surfaced so later groups don't repeat them.
  const seenUids = new Set<string>();
  const seenEmails = new Set<string>();

  const claim = (c: ContactLike): boolean => {
    const uid = c.personUid?.trim();
    const email = c.email?.trim().toLowerCase();
    if (uid && seenUids.has(uid)) return false;
    if (email && seenEmails.has(email)) return false;
    if (uid) seenUids.add(uid);
    if (email) seenEmails.add(email);
    return true;
  };

  // (a) Contacts.
  const contactRows = contacts
    .filter((c) => matchesQuery(c, query))
    .filter(claim)
    .slice(0, limitPerGroup)
    .map(toRow);
  if (contactRows.length > 0) {
    groups.push({ key: 'contacts', label: 'Contacts', rows: contactRows });
  }

  // (b) Per-company members, grouped + labeled.
  for (const co of companies) {
    const members = membersByCompany[co.companyUid] ?? [];
    const rows = members
      .filter((c) => matchesQuery(c, query))
      .filter(claim)
      .slice(0, limitPerGroup)
      .map(toRow);
    if (rows.length > 0) {
      groups.push({
        key: `company:${co.companyUid}`,
        label: `From ${co.companyName?.trim() || 'your team'}`,
        rows,
      });
    }
  }

  // (c) Free-text email row — only if the query is a valid email not already
  // listed above.
  const q = query.trim();
  if (isValidEmail(q) && !seenEmails.has(q.toLowerCase())) {
    groups.push({
      key: 'freetext',
      label: '',
      rows: [
        {
          recipient: {
            email: q,
            connectionState: 'none',
            displayName: undefined,
          },
          primary: `Send to ${q}`,
          secondary: null,
          freeText: true,
        },
      ],
    });
  }

  return groups;
}

/** Flatten groups to an ordered row list — used for keyboard navigation. */
export function flattenRows(groups: SuggestionGroup[]): SuggestionRow[] {
  return groups.flatMap((g) => g.rows);
}
