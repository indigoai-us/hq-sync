/**
 * Decide which caption to render under the live-progress bar during a sync.
 *
 * The denominator for "N of M transferred" MUST be the strict transfer count
 * (sum of push + pull + conflict files from per-company `sync:plan` events),
 * NOT the Rust pre-walk total — the pre-walk counts every syncable file in
 * the local tree, so using it makes the caption read "0 of 47,000 transferred"
 * at the start of a sync when only ~50 files actually need to move.
 *
 * Ordered preference:
 *   1. Plan total > 0 and we haven't overshot it -> "N of M transferred".
 *   2. We've recorded transfers -> "N transferred" (no denominator).
 *   3. Pre-walk computed 0 syncable files and fanout has started -> "Up to date".
 *   4. Fanout in flight but no plan / progress yet -> "Workspace N of M [· K files]".
 *   5. Personal first-push phase only -> "K of L files".
 *   6. Nothing to show.
 */

export type LiveProgressCaption =
  | { kind: 'transferred-of'; progressed: number; planTotal: number }
  | { kind: 'transferred'; progressed: number }
  | { kind: 'up-to-date' }
  | { kind: 'fanout'; current: number; total: number; progressed: number }
  | { kind: 'personal'; done: number; total: number }
  | { kind: 'none' };

export interface LiveProgressInputs {
  /** Sum of per-file `sync:progress` events received so far. */
  syncFilesProgressed: number;
  /** Sum of (filesToDownload + filesToUpload + filesToConflict) across all
   *  `sync:plan` events received so far. Strict transfer count. */
  syncPlanTotalFiles: number;
  /** Rust pre-walk total — count of every syncable file in the local tree.
   *  NOT a transfer count. Used only to detect the "nothing to sync" case
   *  (pre-walk returned 0 + fanout started). */
  syncTotalFiles: number;
  /** Number of companies in the current fanout-plan. */
  fanoutTotal: number;
  /** Number of companies whose `sync:complete` has fired. */
  fanoutDoneCount: number;
  /** Personal first-push progress (in-process Rust phase). */
  personalFilesDone: number;
  personalFilesTotal: number | null;
}

export function liveProgressCaption(i: LiveProgressInputs): LiveProgressCaption {
  if (i.syncPlanTotalFiles > 0 && i.syncFilesProgressed <= i.syncPlanTotalFiles) {
    return {
      kind: 'transferred-of',
      progressed: i.syncFilesProgressed,
      planTotal: i.syncPlanTotalFiles,
    };
  }
  if (i.syncFilesProgressed > 0) {
    return { kind: 'transferred', progressed: i.syncFilesProgressed };
  }
  if (i.syncTotalFiles === 0 && i.fanoutTotal > 0) {
    return { kind: 'up-to-date' };
  }
  if (i.fanoutTotal > 0) {
    return {
      kind: 'fanout',
      current: Math.min(i.fanoutDoneCount + 1, i.fanoutTotal),
      total: i.fanoutTotal,
      progressed: i.syncFilesProgressed,
    };
  }
  if (i.personalFilesTotal != null && i.personalFilesTotal > 0) {
    return {
      kind: 'personal',
      done: i.personalFilesDone,
      total: i.personalFilesTotal,
    };
  }
  return { kind: 'none' };
}
