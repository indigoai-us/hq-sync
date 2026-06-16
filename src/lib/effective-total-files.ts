/**
 * Decide the denominator the popover uses for sync progress — the "of M"
 * in "N of M" and the divisor for the progress-bar fill.
 *
 * There are two candidate sources, and they mean different things:
 *
 *  - `syncPlanTotalFiles` — the STRICT TRANSFER COUNT: the sum of
 *    `filesToDownload + filesToUpload + filesToConflict` across every
 *    per-company `sync:plan` event. This is "files actually being synced
 *    right now". It is 0 when the tree is already in sync.
 *
 *  - `syncTotalFiles` — the Rust pre-walk total: every syncable file in the
 *    local tree (tens of thousands once company knowledge + personal vault
 *    are counted). NOT a transfer count.
 *
 * The bug this function fixes: the old inline rule was
 *   `syncPlanTotalFiles > 0 ? syncPlanTotalFiles : syncTotalFiles`
 * which fell back to the full pre-walk whenever the transfer count was 0 —
 * i.e. exactly when nothing needed to sync. That made an up-to-date HQ show
 * "Syncing … of 13,660 files" (the whole vault) instead of "Up to date".
 *
 * The fix keys off whether the runner's plan has actually arrived. Modern
 * runners (hq-cloud@5.5.0+) always emit `sync:plan`; once we've seen one,
 * the transfer count is authoritative — including when it is 0. We only fall
 * back to the pre-walk for older runners that never emit a plan at all.
 *
 * Pure function so the accounting is unit-testable without a Svelte harness —
 * see effective-total-files.test.ts.
 */

export interface EffectiveTotalFilesInputs {
  /** True once at least one `sync:plan` event has been processed this run. */
  planReceived: boolean;
  /** Strict transfer count accumulated from `sync:plan` events. */
  syncPlanTotalFiles: number;
  /** Rust pre-walk total — every syncable file in the local tree. */
  syncTotalFiles: number;
}

export function effectiveTotalFiles(i: EffectiveTotalFilesInputs): number {
  // Once the plan is in, the transfer count is the truth — even at 0, which
  // is the "nothing to sync" signal the up-to-date caption depends on.
  // Pre-plan (or legacy runners with no plan), fall back to the pre-walk so
  // the bar still has a denominator instead of dividing by zero.
  return i.planReceived ? i.syncPlanTotalFiles : i.syncTotalFiles;
}
