import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * Agent-native Messages — Catch-up digest (SPEC §5 "catch-up digest instead of
 * unread walls"). The v4/CatchUp component was built but unmounted; this wires
 * it into the Messages rail using ONLY real signals already loaded:
 *
 *   - channels carrying a real `unread` count, and
 *   - DMs whose last message came IN (previewDirection/lastMessageDirection
 *     === 'in') — the conversations where the ball is in your court.
 *
 * There is no per-DM unread flag server-side, so the digest never claims a DM
 * is "unread" — those rows are framed honestly as "waiting". No fabricated data.
 */

describe('desktop-alt Messages catch-up digest', () => {
  const shell = readRepoFile('src/components/messaging/MessagesShell.svelte');
  const catchUp = readRepoFile('src/components/messaging/v4/CatchUp.svelte');

  it('mounts the CatchUp digest in the rail', () => {
    expect(shell).toContain("import CatchUp, { type CatchUpItem } from './v4/CatchUp.svelte'");
    expect(shell).toContain('<CatchUp');
    // Only on the All segment, only when there are real items, dismissible.
    expect(shell).toContain("segment === 'all' && catchUpItems.length > 0 && !catchUpDismissed");
    expect(shell).toContain('ondismiss={() => (catchUpDismissed = true)}');
  });

  it('builds items from REAL unread channels + inbound DMs (no fabricated unread)', () => {
    // Channels: genuine unread count.
    expect(shell).toContain('(ch.unread ?? 0) > 0');
    expect(shell).toContain('${ch.unread} unread');
    // DMs: the last message came IN — never labelled "unread".
    expect(shell).toContain("((c.previewDirection ?? c.lastMessageDirection) ?? '') === 'in'");
    expect(shell).toContain("id: `dm:${c.personUid}`");
    expect(shell).toContain("id: `ch:${ch.channelId}`");
    // It's a ranked digest (top slice), not the whole list.
    expect(shell).toContain('CATCH_UP_LIMIT');
    expect(shell).toContain('rank: index + 1');
  });

  it('routes an opened digest item back to its real conversation', () => {
    expect(shell).toContain('function handleCatchUpOpen');
    expect(shell).toContain("item.id.startsWith('ch:')");
    expect(shell).toContain('selectChannel(channel)');
    expect(shell).toContain("item.id.startsWith('dm:')");
    expect(shell).toContain('void selectContact(contact)');
  });

  it('frames the digest honestly (waiting, not unread) and is dismissible + token-safe', () => {
    expect(catchUp).toContain('waiting');
    expect(catchUp).not.toContain('} unread</span>');
    expect(catchUp).toContain('ondismiss');
    expect(catchUp).toContain('catch-up-hide');
    // CatchUp renders inside MessagesShell's cascade (desktop-alt.css + popover.css),
    // which does NOT resolve --v4-* tokens — so the component must not use them.
    const style = catchUp.split('<style>')[1] ?? '';
    expect(style).not.toMatch(/var\(--v4-/);
    // No purple accent.
    expect(style).not.toMatch(/var\(--accent/);
  });
});
