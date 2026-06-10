/**
 * Refreshers that must run every time the popover gains focus (i.e. is
 * opened). Centralised here — rather than inlined in the `onFocusChanged`
 * handler — so the set is a single typed contract that can't silently drift.
 *
 * Why this exists: `loadHqVersion()` was historically missing from the
 * open-refresh set. The footer reads hq-core's `hqVersion` from `core.yaml`
 * once at mount; if that read returned null (a transient — the HQ folder
 * resolving or `core.yaml` becoming readable a moment after launch), the
 * footer stayed "HQ version unknown" until a full app relaunch re-ran mount.
 * Re-reading on every popover open recovers the version without a relaunch.
 *
 * Each refresher is fire-and-forget; callbacks own their own error handling
 * (the App.svelte implementations swallow + log, so a transient failure never
 * blanks a row mid-session).
 */
export interface PopoverOpenRefreshers {
  /** Re-pull the workspace/company list (a company added between syncs). */
  loadWorkspaces: () => unknown;
  /** Re-pull hq-cli update state so a missed event surfaces within one open. */
  refreshHqCliUpdate: () => unknown;
  /** Re-read hq-core `hqVersion` so the footer recovers from a startup null. */
  loadHqVersion: () => unknown;
}

/**
 * Invoke every popover-open refresher. The {@link PopoverOpenRefreshers}
 * interface makes it a compile error to drop one — the runtime test in
 * `popover-refresh.test.ts` is the belt to this type's suspenders.
 */
export function refreshOnPopoverOpen(refreshers: PopoverOpenRefreshers): void {
  refreshers.loadWorkspaces();
  refreshers.refreshHqCliUpdate();
  refreshers.loadHqVersion();
}
