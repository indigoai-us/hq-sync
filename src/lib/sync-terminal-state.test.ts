import { describe, it, expect } from 'vitest';
import { resolveAllCompleteState } from './sync-terminal-state';

describe('resolveAllCompleteState', () => {
  it('brand-new empty-company member: clean run resets a latched error to idle', () => {
    // The malaberg case. The prior run left syncState='error' (the stuck
    // "Sync initialized / Finish in Claude Code" banner). This run is clean:
    // the Rust side stripped the benign empty-company rows (rollup empty) and
    // no genuine sync:error fired. It must return to idle.
    expect(resolveAllCompleteState('error', 0, false)).toBe('idle');
  });

  it('clean run from syncing stays idle', () => {
    expect(resolveAllCompleteState('syncing', 0, false)).toBe('idle');
  });

  it('genuine rollup error latches error even from a syncing state', () => {
    // A genuinely failing company survived filtering — must still latch error,
    // including when the error was routed on stderr and never fired a
    // mid-stream sync:error.
    expect(resolveAllCompleteState('syncing', 1, false)).toBe('error');
  });

  it('wrapper-side genuine error (not in rollup) still latches error', () => {
    // e.g. a first-push failure fired sync:error but the runner rollup is empty.
    expect(resolveAllCompleteState('error', 0, true)).toBe('error');
  });

  it('conflict is preserved and takes priority over the error/idle decision', () => {
    expect(resolveAllCompleteState('conflict', 0, false)).toBe('conflict');
    expect(resolveAllCompleteState('conflict', 3, true)).toBe('conflict');
  });
});
