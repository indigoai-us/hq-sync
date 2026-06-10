import { describe, it, expect, vi } from 'vitest';
import { refreshOnPopoverOpen, type PopoverOpenRefreshers } from './popover-refresh';

function makeRefreshers(): PopoverOpenRefreshers {
  return {
    loadWorkspaces: vi.fn<() => unknown>(),
    refreshHqCliUpdate: vi.fn<() => unknown>(),
    loadHqVersion: vi.fn<() => unknown>(),
  };
}

describe('refreshOnPopoverOpen', () => {
  it('re-reads hq-core version on every popover open (regression: footer stuck "version unknown")', () => {
    const r = makeRefreshers();
    refreshOnPopoverOpen(r);
    // The bug: loadHqVersion was missing from the open-refresh set, so a
    // startup null left the footer stale until a full app relaunch.
    expect(r.loadHqVersion).toHaveBeenCalledTimes(1);
  });

  it('refreshes the full open-set (workspaces + cli-update + version)', () => {
    const r = makeRefreshers();
    refreshOnPopoverOpen(r);
    expect(r.loadWorkspaces).toHaveBeenCalledTimes(1);
    expect(r.refreshHqCliUpdate).toHaveBeenCalledTimes(1);
    expect(r.loadHqVersion).toHaveBeenCalledTimes(1);
  });
});
