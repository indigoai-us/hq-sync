/**
 * Keep a Library surface fresh.
 *
 * The desktop Library reads workers/skills from the local HQ folder once, when a
 * page mounts (`LibraryPage` / `CompanyLibraryPanel`). Without a refresh signal a
 * worker created in another tool — e.g. `/newworker` in Claude Code, or hand-
 * authoring a `worker.yaml` — does not appear until the page is remounted
 * (navigate to another section and back, or reopen the desktop window). That
 * "I made a worker but it isn't in the library" gap is what this module closes.
 *
 * We re-fetch on two signals:
 *   - the desktop window regaining focus — the primary trigger: the user creates
 *     a worker elsewhere, then switches back to the app, and
 *   - `sync:complete` — a worker authored by a teammate that arrives via cloud
 *     sync.
 *
 * This mirrors `DesktopApp`'s existing `onFocusChanged -> refreshRealState`
 * convention (the Library surfaces were simply never wired into it). The work is
 * a cheap local-filesystem re-read, so refreshing on focus is safe to do eagerly.
 *
 * Returns a single unlisten that tears down both subscriptions.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

export async function subscribeLibraryRefresh(
  onRefresh: () => void,
): Promise<UnlistenFn> {
  const unlistenFocus = await getCurrentWindow().onFocusChanged(
    ({ payload: focused }) => {
      // Only a gained focus is a refresh signal; losing focus is a no-op.
      if (focused) onRefresh();
    },
  );

  const unlistenSync = await listen('sync:complete', () => {
    onRefresh();
  });

  return () => {
    unlistenFocus();
    unlistenSync();
  };
}
