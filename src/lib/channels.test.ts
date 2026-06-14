import { describe, it, expect } from 'vitest';
import {
  type Channel,
  channelDisplayName,
  companyNameFor,
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

describe('group DMs', () => {
  it('labels an unnamed group by member count, else a generic label', () => {
    expect(
      channelDisplayName(ch({ channelId: 'g1', name: '', scope: 'group', memberCount: 3 })),
    ).toBe('Group · 3');
    expect(channelDisplayName(ch({ channelId: 'g1', name: '', scope: 'group' }))).toBe('Group DM');
  });
  it('uses a "Group" scope chip', () => {
    expect(scopeChipLabel(ch({ channelId: 'g', name: '', scope: 'group' }))).toBe('Group');
  });
  it('buckets group DMs under a "Direct" header, first and separate from company/personal', () => {
    const groups = groupChannels([
      ch({ channelId: 'g1', name: '', scope: 'group', memberCount: 3 }),
      ch({ channelId: 'p1', name: 'diary', scope: 'personal' }),
      ch({ channelId: 'c1', name: 'eng', scope: 'company', companyUid: 'ent_1', companyName: 'Acme' }),
    ]);
    expect(groups[0].key).toBe('group');
    expect(groups[0].label).toBe('Direct');
    expect(groups[0].channels.map((c) => c.channelId)).toEqual(['g1']);
    // Group DMs never leak into the company buckets.
    const company = groups.find((g) => g.scope === 'company');
    expect(company?.channels.map((c) => c.channelId)).toEqual(['c1']);
  });
});

describe('scopeChipLabel', () => {
  it('returns Personal for personal channels', () => {
    expect(scopeChipLabel(ch({ channelId: 'c', name: 'x', scope: 'personal' }))).toBe('Personal');
  });
  it('prefers companyName, else generic — NEVER the raw UID', () => {
    expect(
      scopeChipLabel(ch({ channelId: 'c', name: 'x', scope: 'company', companyName: 'Acme', companyUid: 'ent_1' })),
    ).toBe('Acme');
    // A bare companyUid (no name) degrades to "Company", not the opaque UID —
    // this is the leak fix; the chip must never render `ent_1` / `cmp_…`.
    expect(scopeChipLabel(ch({ channelId: 'c', name: 'x', scope: 'company', companyUid: 'ent_1' }))).toBe('Company');
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

describe('company label resolution never leaks a raw UID (REGRESSION)', () => {
  // The unified rail rendered `cmp_01KQ2RYAHXHDPCTY9GPQPTH3DG` as a chip when the
  // server omitted companyName. A row must NEVER show the opaque cmp_ UID.
  const COMPANY_UID = 'cmp_01KQ2RYAHXHDPCTY9GPQPTH3DG';

  it('scopeChipLabel returns "Company", not the cmp_ UID, when no name is known', () => {
    const label = scopeChipLabel(
      ch({ channelId: 'c1', name: 'crew', scope: 'company', companyUid: COMPANY_UID }),
    );
    expect(label).toBe('Company');
    expect(label).not.toContain('cmp_');
  });

  it('companyNameFor resolves the server companyName when present', () => {
    expect(
      companyNameFor(
        ch({ channelId: 'c1', name: 'crew', scope: 'company', companyUid: COMPANY_UID, companyName: 'Indigo' }),
      ),
    ).toBe('Indigo');
  });

  it('companyNameFor resolves the name from the memberships list by UID', () => {
    expect(
      companyNameFor(
        ch({ channelId: 'c1', name: 'crew', scope: 'company', companyUid: COMPANY_UID }),
        [{ companyUid: COMPANY_UID, companyName: 'Indigo' }],
      ),
    ).toBe('Indigo');
  });

  it('companyNameFor falls back to "Company" — never the UID — when unresolved', () => {
    const name = companyNameFor(
      ch({ channelId: 'c1', name: 'crew', scope: 'company', companyUid: COMPANY_UID }),
    );
    expect(name).toBe('Company');
    expect(name).not.toContain('cmp_');
  });

  it('companyNameFor returns null for personal/group channels (no company chip)', () => {
    expect(companyNameFor(ch({ channelId: 'c1', name: 'notes', scope: 'personal' }))).toBeNull();
    expect(companyNameFor(ch({ channelId: 'c2', name: '', scope: 'group' }))).toBeNull();
  });

  it('groupChannels header label never falls back to the raw UID', () => {
    const groups = groupChannels([
      ch({ channelId: 'c1', name: 'crew', scope: 'company', companyUid: COMPANY_UID }),
    ]);
    const companyGroup = groups.find((g) => g.scope === 'company');
    expect(companyGroup?.label).toBe('Company');
    expect(companyGroup?.label).not.toContain('cmp_');
  });
});
