import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Conflict dead-end fix: when a sync is conflict-aborted, the runner reports
// conflicts only in aggregate (`sync:complete {conflicts, aborted}`) — it no
// longer emits per-file `sync:conflict` events, so the per-file ConflictModal
// can never populate. Previously the conflict state was a silent dead-end: the
// tray went red and the popover body showed NOTHING actionable. This wires an
// honest, actionable conflict banner (resolve-in-Claude-Code + Copy prompt, with
// the header Sync button as retry), driven by an aggregate count that is reset
// at every sync start so a resolved conflict doesn't linger.
//
// Source-contract assertions (mirroring the US-* story tests) so a dropped wire
// — the count accumulation, the reset, the prop pass-through, the banner branch
// — fails fast without a macOS Tauri build.

const read = (p: string) => readFileSync(resolve(process.cwd(), p), 'utf8');
const normalize = (s: string) => s.replace(/\s+/g, ' ');

const popover = read('src/components/Popover.svelte');
const app = read('src/App.svelte');

describe('conflict dead-end: actionable conflict banner', () => {
  it('App accumulates the aggregate conflict count on an aborted sync:complete', () => {
    const a = normalize(app);
    expect(a).toContain('syncConflictCount += event.payload.conflicts');
    // The single-vs-multi company hint: name the slug only when exactly one
    // company aborted, otherwise blank it.
    expect(a).toContain('syncConflictCompany = event.payload.company');
    expect(app).toContain("syncConflictCompany = ''");
  });

  it('App resets the conflict accounting at every sync start (manual + fanout)', () => {
    // Reset appears in handleSyncNow AND the sync:fanout-plan handler so a
    // resolved conflict never carries a stale banner into the next run.
    const resets = app.split('syncConflictCount = 0').length - 1;
    expect(resets).toBeGreaterThanOrEqual(2);
  });

  it('App passes the aggregate conflict count + company down to the popover', () => {
    expect(app).toContain('conflictCount={syncConflictCount}');
    expect(app).toContain('conflictCompany={syncConflictCompany}');
  });

  it('Popover renders an actionable conflict banner in the conflict state', () => {
    const p = normalize(popover);
    // A dedicated branch for the conflict state (not just auth/error).
    expect(p).toContain("{:else if syncState === 'conflict'}");
    // Plain, non-alarming framing — no raw paths, no "failed".
    expect(p).toContain('Sync paused');
    expect(p).not.toContain('Sync failed');
    // The resolve action routes through the existing sync-conflict prompt with
    // the aggregate count + company (the prompt builder runs /resolve-conflicts).
    expect(p).toContain("kind: 'sync-conflict'");
    expect(p).toContain('count: conflictCount');
    expect(p).toContain('company: conflictCompany');
    // Both an Open-in-Claude-Code affordance and a Copy-prompt fallback.
    expect(popover).toContain('Resolve in Claude Code');
    expect(popover).toContain('Copy prompt');
  });
});
