/**
 * Map a sync-progress `company` field to a human-readable popover label.
 *
 * The cross-process sync-progress snapshot written by hq-cloud carries the raw
 * vault UID (`prs_…` for the personal vault, `cmp_…` for a company), or null —
 * not a friendly name. Without mapping, the popover reads "Syncing prs_01abc…"
 * instead of "Personal". Company UIDs are resolved against the known company
 * list (from the fanout-plan event); an already-friendly slug passes through.
 */
export interface CompanyRef {
  uid: string;
  slug: string;
  name?: string;
}

export function friendlyCompanyLabel(
  company: string | null | undefined,
  companies: CompanyRef[],
): string {
  if (!company || company.startsWith("prs_")) return "Personal";
  const match = companies.find((c) => c.uid === company);
  if (match) return match.name ?? match.slug;
  // A company UID we don't have a name for (e.g. an external CLI sync that
  // never emitted a fanout-plan). Better a clean generic than a raw UID.
  if (company.startsWith("cmp_")) return "a company";
  // Already a friendly slug.
  return company;
}
