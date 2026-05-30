/**
 * Classify a `meetings_invite_bot` failure as the benign "already scheduled"
 * case.
 *
 * A bot-invite call can fail with HTTP 409 when a bot is already scheduled —
 * or in the middle of being scheduled — for the same meeting. This happens
 * when a separate hq-sync instance, the auto-schedule cron, or a double-submit
 * got there first. It is benign: the bot exists, so the UI should treat it as
 * success (clear the input, show a friendly toast, refresh the list) rather
 * than surfacing a scary failure.
 *
 * The server signals this with HTTP 409 and one of two codes — see hq-pro
 * `bot.controller.ts` `handleInvite`:
 *   - `bot-already-scheduled`  — a pre-existing sibling bot / Recall dedup
 *   - `bot-already-scheduling` — the atomic dedup-lock race (two Lambdas in
 *                                the same millisecond)
 * Both share the `bot-already-schedu` prefix. The error reaches the frontend
 * as a flattened Tauri command-error string (`bot/invite HTTP 409: {…}`), so
 * we match the shared code prefix and fall back to the bare `409` status.
 */
export function isAlreadyScheduledError(err: unknown): boolean {
  const msg = String(err ?? '');
  return msg.includes('409') || msg.includes('bot-already-schedu');
}
