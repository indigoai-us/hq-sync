/**
 * Decide how much each sync event contributes to the popover's
 * "N transferred" counter (`syncFilesProgressed` in App.svelte).
 *
 * The counter must only ever count files that actually moved:
 *
 *  - `runner-progress` — the runner emits `sync:progress` once per file it
 *    transfers (and only for transfers, never for skips). Counts as 1.
 *
 *  - `personal-first-push-progress` — the in-process Rust walker
 *    (run_personal_first_push) emits this once per file it EXAMINES,
 *    including files it then skips because the journal hash matches.
 *    It is a liveness signal, not a transfer. Counts as 0 — counting it
 *    made a fully-in-sync HQ read "2,503 transferred" after a Sync click
 *    that moved nothing.
 *
 *  - `personal-first-push-complete` — carries the walker's actual upload
 *    count, the only place real personal-phase transfers are reported.
 *    Counts as `filesUploaded`.
 *
 * Pure function so the accounting is unit-testable without a Svelte
 * component harness — see transfer-count.test.ts for the regression test.
 */

export type TransferCountEvent =
  | { kind: 'runner-progress' }
  | { kind: 'personal-first-push-progress'; currentFile: string | null }
  | { kind: 'personal-first-push-complete'; filesUploaded: number };

export function transferCountDelta(event: TransferCountEvent): number {
  switch (event.kind) {
    case 'runner-progress':
      return 1;
    case 'personal-first-push-progress':
      return 0;
    case 'personal-first-push-complete':
      return event.filesUploaded;
  }
}
