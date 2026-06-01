//! Pure helpers for resolving which company a desktop-alt recording is
//! attributed to, extracted so the rules are unit-testable without Tauri.
//!
//! Mirrors the inline logic the classic `App.svelte` carries for the popover
//! UI: a default-recording-company resolved from settings (validated against
//! the *active* memberships), a per-meeting override the user can set, and a
//! back-fill that seeds late-loading "detected" rows with the resolved default
//! without clobbering an explicit user choice.
//!
//! ## Why a separate module
//!
//! `src/lib/activeMeetings.ts` is the desktop-alt recording store; it owns the
//! Tauri `invoke` surface and Svelte writables. Keeping the *decisions* here —
//! pure functions over plain data — lets `recordingCompany.test.ts` assert the
//! attribution rules with plain fixtures, the same way `meetingDetection.ts` /
//! `meetingDetection.test.ts` split the detection decision from the IPC.

/**
 * One membership row the user can attribute a recording to. Structurally a
 * subset of the store's `CompanyMembership` (desktop-alt meetings model) and
 * the classic `MembershipRow`, so the active list can be passed straight in.
 */
export interface RecordingMembership {
  companyUid: string;
  companyName?: string | null;
  role?: string | null;
  status: string;
}

/**
 * The attribution-relevant slice of an `ActiveMeeting` row. `companyUserSet`
 * marks an *explicit* user choice (including an explicit "Personal" = `null`),
 * which must never be overwritten by a resolved default or a back-fill.
 */
export interface AttributableRow {
  companyUid: string | null;
  companyUserSet?: boolean;
}

/**
 * Validate a stored default-recording-company UID against the memberships the
 * user actually has. A stale default (company left / membership revoked) must
 * not silently attribute recordings to a company the user can no longer record
 * for — so an unmatched UID resolves to `null` (= Personal).
 */
export function resolveValidDefault(
  defaultUid: string | null,
  memberships: RecordingMembership[],
): string | null {
  return defaultUid && memberships.some((m) => m.companyUid === defaultUid)
    ? defaultUid
    : null;
}

/**
 * Decide the company a `start_recording` call should attribute to.
 *
 * An explicit per-meeting choice (`companyUserSet`) always wins — even when it
 * is `null` (the user deliberately chose Personal). Otherwise fall back to the
 * validated default, and last of all to whatever the row already carried.
 */
export function resolveStartCompany(
  row: AttributableRow | undefined,
  defaultUid: string | null,
  memberships: RecordingMembership[],
): string | null {
  return row?.companyUserSet
    ? (row.companyUid ?? null)
    : (resolveValidDefault(defaultUid, memberships) ?? row?.companyUid ?? null);
}

/**
 * Filter memberships to the ones the user can actually record for. Mirrors the
 * classic popover's load step (`App.svelte`: `list.filter(m => m.status ===
 * 'active')`) — invited / suspended rows must not appear in the picker nor be a
 * valid default.
 */
export function activeMemberships(
  memberships: RecordingMembership[],
): RecordingMembership[] {
  return memberships.filter((m) => m.status === 'active');
}

/**
 * Whether a "detected" row that loaded before the recording-company context
 * was ready should be back-filled with the resolved default. Only seeds a
 * genuine default onto a non-user-set row that doesn't already carry it — never
 * touches an explicit user choice and never overwrites with `null`.
 */
export function shouldBackfill(
  row: AttributableRow,
  validDefault: string | null,
): boolean {
  return validDefault !== null && !row.companyUserSet && row.companyUid !== validDefault;
}
