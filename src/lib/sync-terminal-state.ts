// Pure decision core for the terminal syncState when a menubar-spawned sync
// run finishes (the `sync:all-complete` event). Extracted from `App.svelte` so
// the "clean-enough vs real failure" logic is unit-testable without a Svelte
// component harness — same idiom as `joinableMemberships` in `workspaces.ts`.

export type SyncTerminalState = 'idle' | 'error' | 'conflict';

/**
 * Resolve the terminal syncState from a completed run's aggregate outcome.
 *
 * Inputs:
 *  - `currentState`: syncState at the moment `sync:all-complete` fires.
 *  - `rollupErrorCount`: length of the all-complete `errors` array. The Rust
 *    side (`filter_benign_all_complete_errors`) has ALREADY stripped benign
 *    empty-company / not-yet-provisioned rows, so this counts only genuinely
 *    alertable failures.
 *  - `sawGenuineErrorThisRun`: whether a genuine `sync:error` fired mid-run.
 *    Benign empty-company errors are reclassified to `sync:complete` by the
 *    Rust side and never fire `sync:error`, so any observed `sync:error` is
 *    genuine. This also captures wrapper-side failures (e.g. a failed
 *    first-push) that never appear in the runner's aggregate rollup.
 *
 * Rules:
 *  - `conflict` is a distinct, separately-actionable state (resolve flow +
 *    banner) and takes priority — never overridden here.
 *  - a run with any genuine error latches `error` (still surfaces the banner),
 *    EVEN when the error was routed on the runner's stderr channel and so never
 *    fired a mid-stream `sync:error` — the rollup catches it.
 *  - otherwise the run is clean-enough → `idle`. Crucially this clears an
 *    `error` latched earlier in this run, which is how a brand-new empty-company
 *    member escapes the stuck "Sync initialized / Finish in Claude Code" banner
 *    after one clean sync.
 */
export function resolveAllCompleteState(
  currentState: string,
  rollupErrorCount: number,
  sawGenuineErrorThisRun: boolean,
): SyncTerminalState {
  if (currentState === 'conflict') return 'conflict';
  if (rollupErrorCount > 0 || sawGenuineErrorThisRun) return 'error';
  return 'idle';
}
