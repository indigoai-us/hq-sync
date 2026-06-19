import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Tauri API mocks ───────────────────────────────────────────────────────────
//
// `library-refresh.ts` wires two refresh signals:
//   - `getCurrentWindow().onFocusChanged(...)` (window focus regained), and
//   - `listen('sync:complete', ...)` (a sync finished).
// We capture both registered handlers so a test can fire a synthetic focus /
// sync event straight at the real consumer, and we hand back spy unlisten fns so
// the teardown path is observable.

type EventHandler = (event: { payload: unknown }) => void;
type FocusHandler = (event: { payload: boolean }) => void;

const eventHandlers = new Map<string, EventHandler>();
const unlistenEvent = vi.fn();
const unlistenFocus = vi.fn();
let focusHandler: FocusHandler | undefined;

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, handler: EventHandler) => {
    eventHandlers.set(name, handler);
    return Promise.resolve(unlistenEvent);
  }),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: (handler: FocusHandler) => {
      focusHandler = handler;
      return Promise.resolve(unlistenFocus);
    },
  }),
}));

import { subscribeLibraryRefresh } from './library-refresh';

function fireFocus(focused: boolean): void {
  if (!focusHandler) throw new Error('focus handler not registered');
  focusHandler({ payload: focused });
}

function fireSyncComplete(): void {
  const handler = eventHandlers.get('sync:complete');
  if (!handler) throw new Error('sync:complete listener not registered');
  handler({ payload: {} });
}

describe('subscribeLibraryRefresh', () => {
  beforeEach(() => {
    eventHandlers.clear();
    focusHandler = undefined;
    unlistenEvent.mockClear();
    unlistenFocus.mockClear();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('refreshes when the window regains focus', async () => {
    const onRefresh = vi.fn();
    await subscribeLibraryRefresh(onRefresh);

    fireFocus(true);

    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it('does NOT refresh when the window loses focus', async () => {
    const onRefresh = vi.fn();
    await subscribeLibraryRefresh(onRefresh);

    fireFocus(false);

    expect(onRefresh).not.toHaveBeenCalled();
  });

  it('refreshes when a sync completes', async () => {
    const onRefresh = vi.fn();
    await subscribeLibraryRefresh(onRefresh);

    fireSyncComplete();

    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it('refreshes once per signal (focus + sync are independent)', async () => {
    const onRefresh = vi.fn();
    await subscribeLibraryRefresh(onRefresh);

    fireFocus(true);
    fireSyncComplete();
    fireFocus(true);

    expect(onRefresh).toHaveBeenCalledTimes(3);
  });

  it('tears down both subscriptions on unlisten', async () => {
    const onRefresh = vi.fn();
    const unlisten = await subscribeLibraryRefresh(onRefresh);

    unlisten();

    expect(unlistenFocus).toHaveBeenCalledTimes(1);
    expect(unlistenEvent).toHaveBeenCalledTimes(1);
  });
});
