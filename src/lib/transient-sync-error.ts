/**
 * Classifies a `sync:error` message as a transport-level transient failure — the
 * underlying Rust `reqwest` HTTP request never completed (DNS / TCP connect / TLS
 * / the request was sent but dropped before ANY response arrived), as opposed to
 * a server logic error (an HTTP status the server actually returned) or a real
 * client bug. `reqwest` renders these transport failures with the stable markers
 * "transport error" and "error sending request for url (…)".
 *
 * For a BACKGROUND sync step this is recoverable noise: e.g. the personal
 * first-push runs before the runner spawns and the runner re-walks the same tree
 * on the next pass, so a network blip there is retried automatically. Such a blip
 * must not be captured as an error-level Sentry event (HQ-SYNC-WEB-18:
 * `[sync] personal first-push failed: list person entities: transport error:
 * error sending request for url (https://hqapi.getindigo.ai/entity/by-type/person)`,
 * 1 event / 0 users — the request never reached a response).
 *
 * Kept deliberately NARROW — it matches ONLY the reqwest transport-failure
 * markers. A genuine failure the server actually answered (an HTTP 4xx/5xx, a
 * parse error, a logic bug) carries a different message and still surfaces. This
 * is the menubar analogue of hq-console's `isTransientNetworkError`.
 */
export function isTransientSyncTransportError(message: string): boolean {
  const m = message.toLowerCase();
  return (
    m.includes('transport error') || m.includes('error sending request')
  );
}
