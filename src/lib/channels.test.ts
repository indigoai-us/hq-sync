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
import { mergeConversations } from '../components/messaging/contact-order';

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
  it('names an unnamed group DM by its participants (REGRESSION)', () => {
    expect(
      channelDisplayName(
        ch({
          channelId: 'g1',
          name: '',
          scope: 'group',
          memberCount: 3,
          members: [
            { personUid: 'p_s', displayName: 'Stefan' },
            { personUid: 'p_h', displayName: 'Hassaan' },
          ],
        }),
      ),
    ).toBe('Stefan, Hassaan');
  });
  it('truncates a long participant roster with +N', () => {
    expect(
      channelDisplayName(
        ch({
          channelId: 'g1',
          name: '',
          scope: 'group',
          members: [
            { personUid: 'a', displayName: 'Ann' },
            { personUid: 'b', displayName: 'Bo' },
            { personUid: 'c', displayName: 'Cy' },
            { personUid: 'd', displayName: 'Dee' },
            { personUid: 'e', displayName: 'Eli' },
          ],
        }),
      ),
    ).toBe('Ann, Bo, Cy +2');
  });
  it('falls back to the member-count label when participant names are blank', () => {
    expect(
      channelDisplayName(
        ch({
          channelId: 'g1',
          name: '',
          scope: 'group',
          memberCount: 3,
          members: [{ personUid: 'x', displayName: '   ' }],
        }),
      ),
    ).toBe('Group · 3');
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

describe('US-001 REPRO (intentionally RED): externally-created group DM does not surface live', () => {
  // Reproduction for hq-sync-live-new-channel/US-001.
  //
  // ROOT CAUSE (frontend, unified rail sort): when a brand-new group DM created
  // via `hq dm` by the SIGNED-IN user arrives on a channel poll, the
  // `channel:updated` handler in MessagesShell.svelte DOES upsert it into the
  // `channels` array (upsertChannel works) and the unified rail's
  // `mergeConversations(contacts, channels)` DOES include it (no filter drops
  // it). But the rail row inherits `time` from:
  //
  //     time: stamp || (unread > 0 ? now : 0)            // contact-order.ts
  //
  // A channel the caller CREATED/OWNS arrives with unread === 0 (the caller sent
  // the only message, so it is not unread to them) AND — for a just-created
  // channel — no server `lastActivityAt` / `lastMessageAt` yet (both optional on
  // the wire, "older servers omit them"). So `stamp === 0` and `unread === 0`
  // ⇒ `time === 0`: the freshly-arrived conversation sorts to the BOTTOM of a
  // newest-first rail, buried under every existing thread. To the user the new
  // group "never appeared" until a manual Sync re-ran loadChannels(). It is
  // mis-sorted, not dropped.
  //
  // This test encodes the DESIRED behavior (a brand-new conversation the caller
  // just created surfaces at the TOP of the rail). It FAILS against pre-fix code
  // and is the RED reproduction signal for US-001. US-002 fixes the sort seam;
  // US-003 keeps this guard green.
  type RailChannel = Channel; // mergeConversations only needs ChannelRecencyFields
  const NOW = Date.parse('2026-06-15T12:00:00Z');

  function makeContact(personUid: string, lastIso: string) {
    return { personUid, email: `${personUid}@x.com`, displayName: personUid, lastMessageAt: lastIso };
  }

  it('a just-upserted unnamed group DM (unread 0, no server stamps) surfaces at the TOP of the unified rail', () => {
    // An existing, recently-active DM already in the rail.
    const contacts = [makeContact('prs_alice', '2026-06-15T11:59:00Z')];

    // The rail's current channel list (one older, read channel).
    let channels: RailChannel[] = [
      ch({
        channelId: 'chn_existing',
        name: 'existing-channel',
        scope: 'company',
        unread: 0,
        lastActivityAt: '2026-06-15T11:50:00Z',
      }),
    ];

    // The brand-new group DM the signed-in user created via `hq dm`, delivered
    // through the channel:updated event: unnamed, participant-keyed, scope group,
    // unread 0 (caller owns/created it), and NO server activity stamps yet.
    const justCreated = ch({
      channelId: 'chn_01KV6C02ARDJME1W2ZC9JAX4FX',
      name: '',
      scope: 'group',
      memberCount: 5,
      unread: 0,
      lastActivityAt: null,
      lastMessageAt: null,
    });

    // Exactly what MessagesShell's channel:updated handler does.
    channels = upsertChannel(channels, justCreated);

    const rail = mergeConversations(contacts, channels, { now: NOW });

    // Sanity: the upsert + merge did include the new group (it is not dropped).
    const newRow = rail.find((r) => r.channel?.channelId === justCreated.channelId);
    expect(newRow, 'the freshly-upserted group DM should appear in the unified rail').toBeDefined();

    // The actual bug: a brand-new conversation the caller just created must
    // surface at (or near) the TOP — not be buried at the bottom by time:0.
    // This assertion is RED against the current `time: stamp || (unread>0?now:0)`
    // sort, where the new group sinks below every existing thread.
    expect(
      rail[0].channel?.channelId,
      'a brand-new group DM should be the most-recent (top) conversation, not buried at the bottom',
    ).toBe(justCreated.channelId);
  });
});

describe("REGRESSION US-003: externally-created group DM surfaces live at top of rail", () => {
  // Permanent regression guard for hq-sync-live-new-channel.
  //
  // The bug: a brand-new scope:'group' channel created OUTSIDE the app (e.g. via
  // `hq dm`, including one the signed-in user created/owns) arrives on a channel
  // poll through the `channel:updated` → upsertChannel path with unread === 0 and
  // NO server `lastActivityAt` / `lastMessageAt` yet. The pre-fix rail sort was
  // `time: stamp || (unread > 0 ? now : 0)`, so such a channel resolved to
  // `time: 0` and sank to the BOTTOM of the newest-first rail — to the user it
  // "never appeared" until a manual Sync re-ran loadChannels().
  //
  // The fix (US-002): upsertChannel stamps a client-only `arrivedAt` on first
  // insert, and mergeConversations falls back `stamp || arrivedAt || (unread>0?now:0)`,
  // so a freshly-arrived timeless group DM surfaces as recent (top) instead of
  // sinking. This block locks ALL of acceptance-criterion-1 down: the new group
  // is (a) upserted into the rail, (b) grouped under "Direct", (c) carries a
  // non-empty participant-derived title, and (d) — the specific guard for this
  // bug — surfaces at a visible (top) position of the newest-first rail.
  //
  // This test FAILS against the pre-fix sort (`time: stamp || (unread>0?now:0)`)
  // and PASSES after US-002. Do NOT weaken these assertions to make it pass — a
  // failure here means live-surfacing of new channels has regressed.
  const NOW = Date.parse("2026-06-15T12:00:00Z");

  function makeContact(personUid: string, lastIso: string) {
    return { personUid, email: `${personUid}@x.com`, displayName: personUid, lastMessageAt: lastIso };
  }

  // The brand-new, unnamed, participant-keyed group DM delivered via the
  // channel:updated event: scope group, unread 0 (caller created/owns it), and
  // NO server activity stamps yet (both optional on the wire).
  const NEW_GROUP_ID = "chn_01KV6C02ARDJME1W2ZC9JAX4FX";
  function freshGroupDm(): Channel {
    return ch({
      channelId: NEW_GROUP_ID,
      name: "",
      scope: "group",
      memberCount: 5,
      unread: 0,
      lastActivityAt: null,
      lastMessageAt: null,
    });
  }

  it("surfaces at the TOP of the newest-first rail, not buried at the bottom", () => {
    // A recently-active DM and an older, read company channel already in the rail.
    const contacts = [makeContact("prs_alice", "2026-06-15T11:59:00Z")];
    let channels: Channel[] = [
      ch({
        channelId: "chn_existing",
        name: "existing-channel",
        scope: "company",
        unread: 0,
        lastActivityAt: "2026-06-15T11:50:00Z",
      }),
    ];

    // Exactly what MessagesShell's channel:updated handler does — the single
    // insert path stamps arrivedAt on first entry.
    channels = upsertChannel(channels, freshGroupDm(), NOW);

    const rail = mergeConversations(contacts, channels, { now: NOW });

    // (a) It is upserted into the rail — not dropped or filtered.
    const newRow = rail.find((r) => r.channel?.channelId === NEW_GROUP_ID);
    expect(newRow, "the freshly-upserted group DM must appear in the unified rail").toBeDefined();

    // (d) The bug guard: it must surface at the TOP of the newest-first rail,
    // ahead of the existing DM and the older channel.
    expect(
      rail[0].channel?.channelId,
      "a brand-new externally-created group DM must be the most-recent (top) conversation",
    ).toBe(NEW_GROUP_ID);
  });

  it("is grouped under the 'Direct' header with a non-empty participant-derived title", () => {
    const channels = upsertChannel([], freshGroupDm(), NOW);

    // (b) It buckets under the "Direct" group, never into a company bucket.
    const groups = groupChannels(channels);
    const direct = groups.find((g) => g.scope === "group");
    expect(direct, "an externally-created group DM must bucket under a 'Direct' header").toBeDefined();
    expect(direct?.label).toBe("Direct");
    expect(direct?.channels.map((c) => c.channelId)).toContain(NEW_GROUP_ID);

    // (c) It renders a non-empty, participant-derived title — not a blank row.
    const title = channelDisplayName(freshGroupDm());
    expect(title.trim().length, "an unnamed group DM must render a non-empty title").toBeGreaterThan(0);
    expect(title).toBe("Group · 5"); // member-count fallback for an unnamed 5-person group
  });

  it("does not re-float an already-known group on re-poll (no flicker / no reorder)", () => {
    // First arrival stamps arrivedAt = NOW.
    let channels = upsertChannel([], freshGroupDm(), NOW);

    // A later DM arrives and is genuinely newer than the group's arrival.
    const contacts = [makeContact("prs_bob", "2026-06-15T12:05:00Z")];
    const LATER = NOW + 10 * 60_000;

    // Re-poll delivers the SAME group again (still timeless) — upsertChannel must
    // PRESERVE the original arrivedAt so it keeps its place instead of jumping
    // back to the top on every poll.
    channels = upsertChannel(channels, freshGroupDm(), LATER);
    const group = channels.find((c) => c.channelId === NEW_GROUP_ID);
    expect(group?.arrivedAt, "re-poll must preserve the original arrival stamp").toBe(NOW);

    const rail = mergeConversations(contacts, channels, { now: LATER });
    // The newer DM now sits above the (older-arrival) group — order is stable and
    // recency-correct, the group did not re-float to the top.
    expect(rail[0].contact?.personUid).toBe("prs_bob");
    expect(rail[1].channel?.channelId).toBe(NEW_GROUP_ID);
  });
});
