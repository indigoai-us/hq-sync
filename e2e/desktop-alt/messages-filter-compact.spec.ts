import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * Regression — the Messages conversation filter is a compact horizontal row of
 * quiet text tabs, NOT a fat vertical stack, and carries NO purple active dot.
 *
 * Context: the filter was a vertical column of four 30px full-width buttons
 * (~130px of wasted height) whose active item rendered a 4px `var(--accent)`
 * (Indigo #6366f1) dot — a violation of SPEC.md §2 "No purple anywhere" / §6
 * "Blue is allowed only on Messages surfaces". The redesign collapses it to one
 * horizontal line of quiet text tabs (All · People · Channels, Requests demoted
 * to the row end), reclaiming ~100px so the conversation list starts much higher.
 * This spec locks the compact, purple-free shape in.
 */
describe('Messages filter — compact horizontal quiet tabs', () => {
  const shell = readRepoFile('src/components/messaging/MessagesShell.svelte');

  it('uses the compact .seg tab class, not the old fat .segment rows', () => {
    expect(shell).toContain('class="seg"');
    // the old full-width 30px row buttons are gone
    expect(shell).not.toContain('class="segment"');
  });

  it('removed the purple active dot — no var(--accent) ::before in the filter', () => {
    expect(shell).not.toContain('.segment.active::before');
    // the new active cue is a colorless underline, never an accent/purple fill
    expect(shell).toContain('box-shadow: inset 0 -1.5px 0 currentColor');
  });

  it('lays the filter out horizontally, not as a vertical column', () => {
    const start = shell.indexOf('.segments {');
    expect(start).toBeGreaterThan(-1);
    const rule = shell.slice(start, start + 200);
    expect(rule).toContain('align-items: center');
    expect(rule).not.toContain('flex-direction: column');
  });

  it('preserves all four filter behaviors (no segment dropped)', () => {
    for (const seg of ['all', 'people', 'channels', 'requests']) {
      expect(shell).toContain(`segment = '${seg}'`);
    }
    // Requests keeps its neutral count chip (no stoplight color)
    expect(shell).toContain('class="filter-count"');
  });

  it('uses no purple/indigo --accent anywhere on the Messages surface', () => {
    // SPEC §2/§6: blue (--blue / --v4-unread) is the ONLY sanctioned accent on
    // Messages surfaces; the indigo --accent / --accent-soft tokens are banned
    // (hard no-purple policy). The active-conversation cue + compose-button
    // focus use --blue; the "Your agent" bolt avatar is a neutral surface.
    expect(shell).not.toContain('var(--accent)');
    expect(shell).not.toContain('var(--accent-soft)');
  });

  it('keeps the staged v4 agent-native messaging components purple-free', () => {
    // AgentThread / CatchUp / SystemEventCard / UnfurlCard are staged for the
    // agent-native Messages build. They render inside MessagesShell's cascade,
    // so the same hard no-purple policy applies even before they all mount —
    // AgentThread's avatar previously leaked var(--accent-soft) (indigo).
    for (const name of ['AgentThread', 'CatchUp', 'SystemEventCard', 'UnfurlCard']) {
      const component = readRepoFile(`src/components/messaging/v4/${name}.svelte`);
      expect(component, `${name} must not use --accent`).not.toContain('var(--accent)');
      expect(component, `${name} must not use --accent-soft`).not.toContain('var(--accent-soft)');
    }
  });
});
