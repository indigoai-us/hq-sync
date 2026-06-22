import { describe, expect, it } from 'vitest';
import {
  UNATTRIBUTED,
  attributionCompanyName,
  attributionLabel,
  buildSetCompanyArgs,
  companyOptions,
  isRecorded,
  isUnattributed,
  parseSetCompanyResult,
  selectRecorded,
  selectUnattributed,
  setCompanySuccessMessage,
  setCompanyErrorMessage,
  sortByStartDesc,
  type CompanyMembershipLike,
  type ScheduledBotLike,
} from './meetingAttribution';

const memberships: CompanyMembershipLike[] = [
  { companyUid: 'cmp_b', companyName: 'Beta', status: 'active' },
  { companyUid: 'cmp_a', companyName: 'Alpha', status: 'active' },
  { companyUid: 'cmp_c', companyName: null, status: 'active' },
];

describe('isUnattributed', () => {
  it('treats missing, null, empty, and unknown company ids as unattributed', () => {
    expect(isUnattributed({ botId: 'bot_1' })).toBe(true);
    expect(isUnattributed({ botId: 'bot_1', companyId: null })).toBe(true);
    expect(isUnattributed({ botId: 'bot_1', companyId: '' })).toBe(true);
    expect(isUnattributed({ botId: 'bot_1', companyId: '   ' })).toBe(true);
    expect(isUnattributed({ botId: 'bot_1', companyId: ' UnKnOwN ' })).toBe(true);
  });

  it('treats real company ids as attributed', () => {
    expect(isUnattributed({ botId: 'bot_1', companyId: 'cmp_a' })).toBe(false);
  });
});

describe('attributionCompanyName', () => {
  it('returns the membership company name for an attributed bot', () => {
    expect(
      attributionCompanyName({ botId: 'bot_1', companyId: 'cmp_a' }, memberships),
    ).toBe('Alpha');
  });

  it('falls back to company uid when the membership has no name', () => {
    expect(
      attributionCompanyName({ botId: 'bot_1', companyId: 'cmp_c' }, memberships),
    ).toBe('cmp_c');
  });

  it('returns null when unattributed or when the company uid is unknown', () => {
    expect(
      attributionCompanyName(
        { botId: 'bot_1', companyId: UNATTRIBUTED },
        memberships,
      ),
    ).toBeNull();
    expect(
      attributionCompanyName({ botId: 'bot_1', companyId: 'cmp_missing' }, memberships),
    ).toBeNull();
  });
});

describe('attributionLabel', () => {
  it('returns the company name when one can be resolved', () => {
    expect(attributionLabel({ botId: 'bot_1', companyId: 'cmp_b' }, memberships)).toBe('Beta');
  });

  it('returns Unassigned when no company can be resolved', () => {
    expect(attributionLabel({ botId: 'bot_1', companyId: null }, memberships)).toBe('Unassigned');
    expect(attributionLabel({ botId: 'bot_1', companyId: 'cmp_missing' }, memberships)).toBe('Unassigned');
  });
});

describe('companyOptions', () => {
  it('includes active and statusless memberships, filters inactive ones, dedupes by uid, and sorts by label', () => {
    const options = companyOptions([
      { companyUid: 'cmp_b', companyName: 'beta', status: 'active' },
      { companyUid: 'cmp_inactive', companyName: 'Inactive', status: 'invited' },
      { companyUid: 'cmp_c', companyName: null },
      { companyUid: 'cmp_a', companyName: 'Alpha', status: 'ACTIVE' },
      { companyUid: 'cmp_b', companyName: 'Duplicate Beta', status: 'active' },
      { companyUid: 'cmp_suspended', companyName: 'Suspended', status: 'suspended' },
    ]);
    expect(options).toEqual([
      { companyUid: 'cmp_a', label: 'Alpha' },
      { companyUid: 'cmp_b', label: 'beta' },
      { companyUid: 'cmp_c', label: 'cmp_c' },
    ]);
  });

  it('ignores empty company ids', () => {
    expect(companyOptions([{ companyUid: '', companyName: 'Nope', status: 'active' }])).toEqual([]);
  });
});

describe('selectUnattributed', () => {
  it('selects unattributed bots and excludes cancelled meetings', () => {
    const bots: ScheduledBotLike[] = [
      { botId: 'bot_1', companyId: null, status: 'scheduled' },
      { botId: 'bot_2', companyId: ' unknown ', status: 'recording' },
      { botId: 'bot_3', companyId: '', status: 'cancelled' },
      { botId: 'bot_4', companyId: 'cmp_a', status: 'scheduled' },
      { botId: 'bot_5', companyId: undefined },
    ];
    expect(selectUnattributed(bots).map((b) => b.botId)).toEqual([
      'bot_1',
      'bot_2',
      'bot_5',
    ]);
  });
});

describe('isRecorded', () => {
  it('matches completed status', () => {
    expect(isRecorded({ botId: 'bot_1', status: ' completed ' })).toBe(true);
  });

  it('matches landed sources', () => {
    expect(isRecorded({ botId: 'bot_1', status: 'processing', sourceLanded: true })).toBe(true);
  });

  it('ignores bots that are neither completed nor landed', () => {
    expect(isRecorded({ botId: 'bot_1', status: 'processing', sourceLanded: false })).toBe(false);
  });
});

describe('selectRecorded', () => {
  it('selects completed and source-landed bots', () => {
    const bots: ScheduledBotLike[] = [
      { botId: 'bot_completed', status: 'completed' },
      { botId: 'bot_landed', status: 'processing', sourceLanded: true },
      { botId: 'bot_pending', status: 'processing' },
    ];
    expect(selectRecorded(bots).map((b) => b.botId)).toEqual([
      'bot_completed',
      'bot_landed',
    ]);
  });
});

describe('sortByStartDesc', () => {
  it('returns a new array sorted by scheduled start or createdAt descending with undated last', () => {
    const bots: ScheduledBotLike[] = [
      { botId: 'undated' },
      { botId: 'old', scheduledStartTime: '2026-05-01T10:00:00Z' },
      { botId: 'created', createdAt: '2026-05-03T10:00:00Z' },
      {
        botId: 'scheduled_wins',
        scheduledStartTime: '2026-05-04T10:00:00Z',
        createdAt: '2026-05-10T10:00:00Z',
      },
      { botId: 'invalid', scheduledStartTime: 'not-a-date' },
    ];

    const sorted = sortByStartDesc(bots);

    expect(sorted).not.toBe(bots);
    expect(bots.map((b) => b.botId)).toEqual([
      'undated',
      'old',
      'created',
      'scheduled_wins',
      'invalid',
    ]);
    expect(sorted.map((b) => b.botId)).toEqual([
      'scheduled_wins',
      'created',
      'old',
      'undated',
      'invalid',
    ]);
  });
});

describe('buildSetCompanyArgs', () => {
  it('builds camelCase Tauri args and defaults applyToSeries to true', () => {
    expect(buildSetCompanyArgs('bot_1', 'cmp_a')).toEqual({
      meetingId: 'bot_1',
      companyId: 'cmp_a',
      applyToSeries: true,
    });
  });

  it('preserves an explicit applyToSeries false', () => {
    expect(buildSetCompanyArgs('bot_1', UNATTRIBUTED, false)).toEqual({
      meetingId: 'bot_1',
      companyId: UNATTRIBUTED,
      applyToSeries: false,
    });
  });
});

describe('parseSetCompanyResult', () => {
  it('parses success results', () => {
    expect(
      parseSetCompanyResult({
        ok: true,
        meetingId: 'bot_1',
        companyId: 'cmp_a',
        seriesKey: null,
        appliedToSeries: true,
        refiled: false,
        occurrencesUpdated: 4,
        refiledCount: 2,
        refileWarning: 'partial refile',
      }),
    ).toEqual({
      ok: true,
      meetingId: 'bot_1',
      companyId: 'cmp_a',
      seriesKey: null,
      appliedToSeries: true,
      refiled: false,
      occurrencesUpdated: 4,
      refiledCount: 2,
      refileWarning: 'partial refile',
    });
  });

  it('parses error results and falls back for malformed raw values', () => {
    expect(
      parseSetCompanyResult({
        ok: false,
        code: 'meeting-not-found',
        error: 'Missing',
      }),
    ).toEqual({ ok: false, code: 'meeting-not-found', error: 'Missing' });
    expect(parseSetCompanyResult(null)).toEqual({ ok: false });
    expect(parseSetCompanyResult({ ok: 'nope' })).toEqual({ ok: false });
  });
});

describe('setCompanySuccessMessage', () => {
  it('describes a single company update', () => {
    expect(setCompanySuccessMessage({ ok: true, meetingId: 'bot_1', companyId: 'cmp_a' })).toBe('Company updated.');
  });

  it('describes unassigned updates', () => {
    expect(setCompanySuccessMessage({ ok: true, meetingId: 'bot_1', companyId: UNATTRIBUTED })).toBe('Marked unassigned.');
  });

  it('describes multi-occurrence series updates', () => {
    expect(
      setCompanySuccessMessage({
        ok: true,
        meetingId: 'bot_1',
        companyId: 'cmp_a',
        occurrencesUpdated: 3,
      }),
    ).toBe('Updated 3 meetings in this series.');
  });

  it('appends singular and plural refile counts', () => {
    expect(
      setCompanySuccessMessage({
        ok: true,
        meetingId: 'bot_1',
        companyId: 'cmp_a',
        refiledCount: 1,
      }),
    ).toBe('Company updated. Refiled 1 transcript.');
    expect(
      setCompanySuccessMessage({
        ok: true,
        meetingId: 'bot_1',
        companyId: 'cmp_a',
        refiledCount: 2,
      }),
    ).toBe('Company updated. Refiled 2 transcripts.');
  });

  it('combines series and refile copy', () => {
    expect(
      setCompanySuccessMessage({
        ok: true,
        meetingId: 'bot_1',
        companyId: 'cmp_a',
        occurrencesUpdated: 2,
        refiledCount: 1,
      }),
    ).toBe('Updated 2 meetings in this series. Refiled 1 transcript.');
  });
});

describe('setCompanyErrorMessage', () => {
  it('prefers a server-provided human error', () => {
    expect(setCompanyErrorMessage({ ok: false, error: 'Use another company.' })).toBe('Use another company.');
  });

  it('maps known codes', () => {
    expect(setCompanyErrorMessage({ ok: false, code: 'company-access-denied' })).toBe("You don't have access to that company.");
    expect(setCompanyErrorMessage({ ok: false, code: 'meeting-not-found' })).toBe('That meeting no longer exists.');
    expect(setCompanyErrorMessage({ ok: false, code: 'invalid-company' })).toBe('Pick a valid company.');
    expect(setCompanyErrorMessage({ ok: false, code: 'missing-company' })).toBe('Pick a valid company.');
  });

  it('falls back for unknown codes or blank error text', () => {
    expect(setCompanyErrorMessage({ ok: false, code: 'other' })).toBe("Couldn't update the meeting's company.");
    expect(setCompanyErrorMessage({ ok: false, error: '   ' })).toBe("Couldn't update the meeting's company.");
  });
});
