import { describe, it, expect } from 'vitest';
import {
  type Channel,
  channelDisplayName,
  scopeChipLabel,
  isInvitedNotJoined,
  canPost,
  groupChannels,
  totalChannelUnread,
  upsertChannel,
  bumpChannelUnread,
  clearChannelUnread,
} from './channels';

function ch(partial: Partial<Channel> & { channelId: string; name: string }): Channel {
  return {
    scope: 'personal',
    ...partial,
  };
}

describe('channelDisplayName', () => {
  it('strips leading # and trims', () => {
    expect(channelDisplayName(ch({ channelId: 'c1', name: '#general' }))).toBe('general');
    expect(channelDisplayName(ch({ channelId: 'c1', name: '  ##team  ' }))).toBe('team');
  });
  it('falls back to channelId when name is empty', () => {
    expect(channelDisplayName(ch({ channelId: 'c1', name: '   ' }))).toBe('c1');
  });
});

describe('scopeChipLabel', () => {
  it('returns Personal for personal channels', () => {
    expect(scopeChipLabel(ch({ channelId: 'c', name: 'x', scope: 'personal' }))).toBe('Personal');
  });
  it('prefers companyName, then companyUid, then generic', () => {
    expect(
      scopeChipLabel(ch({ channelId: 'c', name: 'x', scope: 'company', companyName: 'Acme', companyUid: 'ent_1' })),
    ).toBe('Acme');
    expect(scopeChipLabel(ch({ channelId: 'c', name: 'x', scope: 'company', companyUid: 'ent_1' }))).toBe('ent_1');
    expect(scopeChipLabel(ch({ channelId: 'c', name: 'x', scope: 'company' }))).toBe('Company');
  });
});

describe('membership helpers', () => {
  it('isInvitedNotJoined only true for invited', () => {
    expect(isInvitedNotJoined(ch({ channelId: 'c', name: 'x', membership: 'invited' }))).toBe(true);
    expect(isInvitedNotJoined(ch({ channelId: 'c', name: 'x', membership: 'joined' }))).toBe(false);
    // Absent membership defaults to joined.
    expect(isInvitedNotJoined(ch({ channelId: 'c', name: 'x' }))).toBe(false);
  });
  it('canPost requires joined membership', () => {
    expect(canPost(ch({ channelId: 'c', name: 'x', membership: 'joined' }))).toBe(true);
    expect(canPost(ch({ channelId: 'c', name: 'x', membership: 'invited' }))).toBe(false);
    expect(canPost(ch({ channelId: 'c', name: 'x' }))).toBe(true);
  });
});

describe('groupChannels', () => {
  it('puts Personal first, then companies in declared order', () => {
    const channels: Channel[] = [
      ch({ channelId: 'c-acme', name: '#acme-eng', scope: 'company', companyUid: 'ent_acme', companyName: 'Acme' }),
      ch({ channelId: 'p1', name: '#diary', scope: 'personal' }),
      ch({ channelId: 'c-beta', name: '#beta', scope: 'company', companyUid: 'ent_beta', companyName: 'Beta' }),
    ];
    const groups = groupChannels(channels, [
      { companyUid: 'ent_beta', companyName: 'Beta' },
      { companyUid: 'ent_acme', companyName: 'Acme' },
    ]);
    expect(groups.map((g) => g.label)).toEqual(['Personal', 'Beta', 'Acme']);
    expect(groups[0].scope).toBe('personal');
    expect(groups[1].scope).toBe('company');
    expect(groups[1].companyUid).toBe('ent_beta');
  });

  it('omits the Personal group when there are no personal channels', () => {
    const groups = groupChannels(
      [ch({ channelId: 'c', name: '#x', scope: 'company', companyUid: 'ent_1', companyName: 'One' })],
      [],
    );
    expect(groups.map((g) => g.label)).toEqual(['One']);
  });

  it('sorts channels within a group by display name (case-insensitive)', () => {
    const groups = groupChannels(
      [
        ch({ channelId: 'b', name: '#Zeta', scope: 'personal' }),
        ch({ channelId: 'a', name: '#alpha', scope: 'personal' }),
      ],
      [],
    );
    expect(groups[0].channels.map((c) => c.channelId)).toEqual(['a', 'b']);
  });

  it('appends companies not in the lookup list, sorted by label', () => {
    const channels: Channel[] = [
      ch({ channelId: 'c1', name: '#z', scope: 'company', companyUid: 'ent_z', companyName: 'Zeta Co' }),
      ch({ channelId: 'c2', name: '#a', scope: 'company', companyUid: 'ent_a', companyName: 'Alpha Co' }),
    ];
    const groups = groupChannels(channels, []);
    expect(groups.map((g) => g.label)).toEqual(['Alpha Co', 'Zeta Co']);
  });
});

describe('unread + upsert helpers', () => {
  it('totalChannelUnread sums unread counts', () => {
    expect(
      totalChannelUnread([
        ch({ channelId: 'a', name: 'a', unread: 2 }),
        ch({ channelId: 'b', name: 'b', unread: 3 }),
        ch({ channelId: 'c', name: 'c' }),
      ]),
    ).toBe(5);
  });

  it('upsertChannel replaces by id or appends', () => {
    const list = [ch({ channelId: 'a', name: 'a' })];
    const replaced = upsertChannel(list, ch({ channelId: 'a', name: 'a2' }));
    expect(replaced).toHaveLength(1);
    expect(replaced[0].name).toBe('a2');
    const appended = upsertChannel(list, ch({ channelId: 'b', name: 'b' }));
    expect(appended.map((c) => c.channelId)).toEqual(['a', 'b']);
  });

  it('bumpChannelUnread adds a delta clamped at 0', () => {
    const list = [ch({ channelId: 'a', name: 'a', unread: 1 })];
    expect(bumpChannelUnread(list, 'a', 2)[0].unread).toBe(3);
    expect(bumpChannelUnread(list, 'a', -5)[0].unread).toBe(0);
    // Unknown id leaves the list unchanged.
    expect(bumpChannelUnread(list, 'zzz', 1)[0].unread).toBe(1);
  });

  it('clearChannelUnread zeroes one channel', () => {
    const list = [
      ch({ channelId: 'a', name: 'a', unread: 4 }),
      ch({ channelId: 'b', name: 'b', unread: 2 }),
    ];
    const cleared = clearChannelUnread(list, 'a');
    expect(cleared[0].unread).toBe(0);
    expect(cleared[1].unread).toBe(2);
  });
});
