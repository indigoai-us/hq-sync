// Pure helpers for the pending DM connection-requests UI (US-011).
//
// The Requests segment (MessagesShell) and the DmRequestCard render incoming
// connection requests and react to the dm:request-new / dm:request-update
// events. Keeping the naming, dedupe, and list-mutation logic here (not inside
// the .svelte components) makes it unit-testable without a DOM — the components
// own the invoke() calls and rendering. Mirrors the lib/recipientPicker.ts split.

/** One pending incoming connection request. Mirrors the Rust `DmRequest` wire
 * shape (camelCase) returned by `list_dm_requests`
 * (GET /v1/notify/connections/requests). */
export interface DmRequest {
  pairKey: string;
  fromPersonUid: string;
  fromEmail: string;
  fromDisplayName: string;
  message?: string | null;
  sharedCompany?: string | null;
  createdAt: string;
}

/** The recipient-side action taken on a request. */
export type RequestAction = 'accept' | 'decline' | 'block';

/** Best display label for a request: display name → email → personUid. */
export function requestDisplayName(req: DmRequest): string {
  return (
    req.fromDisplayName?.trim() ||
    req.fromEmail?.trim() ||
    req.fromPersonUid
  );
}

/** Two-letter avatar initials from the display label. */
export function requestInitials(req: DmRequest): string {
  const name = requestDisplayName(req);
  const parts = name.split(/\s+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  return name.slice(0, 2).toUpperCase();
}

/** Prepend a brand-new request (from dm:request-new), deduped by pairKey so a
 * re-emit never double-adds. Returns a new array (callers reassign $state). */
export function addRequest(list: DmRequest[], req: DmRequest): DmRequest[] {
  if (list.some((r) => r.pairKey === req.pairKey)) return list;
  return [req, ...list];
}

/** Remove a resolved/pruned request by pairKey (from dm:request-update or after
 * a successful respond_dm_request). Returns a new array. */
export function removeRequest(list: DmRequest[], pairKey: string): DmRequest[] {
  return list.filter((r) => r.pairKey !== pairKey);
}

/** The banner title for an incoming request — DISTINCT copy from a normal DM
 * ("{name} wants to connect"). */
export function requestBannerTitle(req: DmRequest): string {
  return `${requestDisplayName(req)} wants to connect`;
}

/** The banner body — the held first message if present, else a fallback prompt
 * to open Messages. */
export function requestBannerBody(req: DmRequest): string {
  return req.message?.trim()
    ? req.message.trim()
    : 'Open Messages to accept, decline, or block this request.';
}
