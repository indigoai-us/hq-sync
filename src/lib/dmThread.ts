// Pure helpers for the DM detail thread (DmDetail.svelte).
//
// The detail window opens scoped to one peer and renders the two-way thread. A
// freshly-arrived inbound DM (broadcast as `dm:new-events`) should fold into the
// open thread live — but only when it belongs to THIS conversation and isn't
// already shown. That decision (peer filter + dedupe) is the part worth testing,
// so it lives here, free of the DOM. The component owns the listen() wiring and
// the field mapping into its rendered message shape.

/** Minimal shape needed to decide whether an inbound DM is already in view. */
export interface ThreadIdLike {
  eventId: string;
}

/** Minimal shape of an inbound DM event for the append decision. */
export interface InboundDmLike {
  eventId: string;
  fromPersonUid: string;
}

/**
 * True when a freshly-arrived inbound DM should be appended to the open thread:
 * it must be from the peer the window is scoped to (`peerUid`) and not already
 * present (by `eventId`). Returns false for DMs from another peer (this window
 * is a single conversation), for an unset peer (nothing open yet), and for
 * duplicates (the poll can re-surface an event, and the same id may also land in
 * a later fetch_dm_thread).
 */
export function shouldAppendInbound(
  messages: ThreadIdLike[],
  dm: InboundDmLike,
  peerUid: string | null | undefined,
): boolean {
  if (!peerUid || dm.fromPersonUid !== peerUid) return false;
  return !messages.some((m) => m.eventId === dm.eventId);
}
