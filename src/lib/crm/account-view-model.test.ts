import { describe, expect, it } from 'vitest';
import {
  buildAccountDetail,
  findAccount,
  groupAccountsByStage,
  needsAttention,
  normalizeProjection,
  openActionItems,
  orDash,
  relativeTime,
  statusTone,
  type ProjectedAccount,
} from './account-view-model';

// ── Fixtures ─────────────────────────────────────────────────────────────────

/** A fully-legged "new client" — the US-010 end-to-end proof slice. */
function fullAccountRaw() {
  return {
    id: 'ent_acme',
    name: 'Acme Robotics',
    domain: 'acme.com',
    stage: 'signed',
    owner: 'corey',
    lastActivity: '2026-06-14T18:00:00Z',
    external_ids: {
      neon: 'req_001',
      attio: 'rec_aa',
      pandadoc: 'doc_pp',
      stripe: 'cus_ss',
    },
    sources: {
      inbound: { system: 'neon', status: 'new', value: 'Demo request', meta: '2026-05-01', ref: 'req_001' },
      pipeline: { system: 'attio', status: 'demo', value: 'Demo', meta: 'in pipeline', ref: 'rec_aa' },
      contract: { system: 'pandadoc', status: 'signed', value: '$48,000', meta: 'annual', ref: 'doc_pp' },
      billing: { system: 'stripe', status: 'paid', value: '$4,000', meta: '2026-06-30', ref: 'cus_ss' },
    },
    timeline: [
      { date: '2026-06-14', type: 'decision', text: 'Picked the annual plan', sourceMeetingRef: 'mtg_3' },
      { date: '2026-06-10', type: 'action_item', text: 'Send the signed SOW', sourceMeetingRef: 'mtg_2' },
      { date: '2026-06-01', type: 'meeting', text: 'Kickoff call', sourceMeetingRef: 'mtg_1' },
    ],
  };
}

/** The fixture projection the US-010 e2e test renders the detail view from. */
function fixtureProjection() {
  return {
    schema_version: 1,
    synced_at: '2026-06-15T00:00:00Z',
    accounts: [
      fullAccountRaw(),
      {
        id: 'ent_lead',
        name: 'Beta Corp',
        domain: 'beta.io',
        stage: 'lead',
        owner: '',
        lastActivity: '',
        external_ids: { neon: 'req_002' },
        // Only an inbound leg — contract + billing missing (graceful degrade).
        sources: {
          inbound: { system: 'neon', status: 'new', value: 'Demo request', meta: '2026-06-12', ref: 'req_002' },
        },
        timeline: [
          { date: '2026-06-12', type: 'action_item', text: 'Reply to inbound demo request', sourceMeetingRef: 'sig_9' },
          { date: '2026-06-11', type: 'risk', text: 'Budget unconfirmed', sourceMeetingRef: 'sig_8' },
        ],
      },
    ],
  };
}

// ── normalizeProjection ──────────────────────────────────────────────────────

describe('normalizeProjection', () => {
  it('parses a well-formed projection and its accounts', () => {
    const p = normalizeProjection(fixtureProjection());
    expect(p.accounts).toHaveLength(2);
    expect(p.synced_at).toBe('2026-06-15T00:00:00Z');
    expect(p.schema_version).toBe(1);
  });

  it('tolerates camelCase syncedAt + externalIds from an alternate producer', () => {
    const p = normalizeProjection({
      schemaVersion: 2,
      syncedAt: '2026-06-16T00:00:00Z',
      accounts: [{ id: 'x', name: 'X', externalIds: { attio: 'rec_x' } }],
    });
    expect(p.synced_at).toBe('2026-06-16T00:00:00Z');
    expect(p.schema_version).toBe(2);
    expect(p.accounts[0].external_ids.attio).toBe('rec_x');
  });

  it('accepts a US-008 timeline entry shape (preview / sourceRef)', () => {
    const p = normalizeProjection({
      accounts: [
        { id: 'x', name: 'X', timeline: [{ date: '2026-01-01', type: 'meeting', preview: 'hi', sourceRef: 'mtg_x' }] },
      ],
    });
    expect(p.accounts[0].timeline[0].text).toBe('hi');
    expect(p.accounts[0].timeline[0].sourceMeetingRef).toBe('mtg_x');
  });

  it('returns an empty projection for null / garbage payloads (no throw)', () => {
    expect(normalizeProjection(null).accounts).toEqual([]);
    expect(normalizeProjection(undefined).accounts).toEqual([]);
    expect(normalizeProjection(42).accounts).toEqual([]);
    expect(normalizeProjection({}).accounts).toEqual([]);
  });

  it('coerces an unknown stage to lead and drops id-less + name-less accounts', () => {
    const p = normalizeProjection({
      accounts: [
        { id: 'a', name: 'A', stage: 'whatever' },
        { stage: 'demo' }, // no id, no name → dropped
      ],
    });
    expect(p.accounts).toHaveLength(1);
    expect(p.accounts[0].stage).toBe('lead');
  });

  it('drops a malformed source leg rather than rendering it', () => {
    const p = normalizeProjection({
      accounts: [
        { id: 'a', name: 'A', sources: { billing: { system: 'not-a-system', status: 'x' } } },
      ],
    });
    expect(p.accounts[0].sources.billing).toBeUndefined();
  });
});

// ── statusTone (the only color; never purple) ────────────────────────────────

describe('statusTone', () => {
  it('maps affirmative states to ok (green)', () => {
    for (const s of ['paid', 'signed', 'active', 'won', 'Complete']) {
      expect(statusTone(s)).toBe('ok');
    }
  });
  it('maps failed states to error (red)', () => {
    for (const s of ['overdue', 'past_due', 'canceled', 'lost', 'failed']) {
      expect(statusTone(s)).toBe('error');
    }
  });
  it('maps in-flight states to warn (amber)', () => {
    for (const s of ['sent', 'open', 'pending', 'draft', 'demo']) {
      expect(statusTone(s)).toBe('warn');
    }
  });
  it('maps unknown / empty to idle (grey)', () => {
    expect(statusTone('')).toBe('idle');
    expect(statusTone(undefined)).toBe('idle');
    expect(statusTone('mystery')).toBe('idle');
  });
});

// ── groupAccountsByStage (list view) ─────────────────────────────────────────

describe('groupAccountsByStage', () => {
  it('groups accounts by funnel stage in funnel order, dropping empty stages', () => {
    const p = normalizeProjection(fixtureProjection());
    const groups = groupAccountsByStage(p.accounts);
    // Only lead + signed are present in the fixture; lead comes first (funnel order).
    expect(groups.map((g) => g.stage)).toEqual(['lead', 'signed']);
    expect(groups[0].label).toBe('Lead');
    expect(groups[1].label).toBe('Signed');
    expect(groups[1].accounts[0].name).toBe('Acme Robotics');
  });

  it('sorts accounts within a stage by name', () => {
    const accounts = normalizeProjection({
      accounts: [
        { id: '1', name: 'Zeta', stage: 'demo' },
        { id: '2', name: 'Alpha', stage: 'demo' },
      ],
    }).accounts;
    const [demo] = groupAccountsByStage(accounts);
    expect(demo.accounts.map((a) => a.name)).toEqual(['Alpha', 'Zeta']);
  });
});

// ── needsAttention (open action-items from signals) ──────────────────────────

describe('needsAttention', () => {
  it('surfaces accounts with open action-items / commitments / risks', () => {
    const p = normalizeProjection(fixtureProjection());
    const items = needsAttention(p.accounts);
    // Beta Corp has 2 open items (action_item + risk); Acme has 1 (action_item).
    expect(items.map((i) => i.account.name)).toEqual(['Beta Corp', 'Acme Robotics']);
    expect(items[0].openCount).toBe(2);
    expect(items[0].reason).toBe('Reply to inbound demo request');
  });

  it('omits accounts with no open items', () => {
    const accounts = normalizeProjection({
      accounts: [
        { id: '1', name: 'NoOpen', stage: 'active', timeline: [{ type: 'meeting', text: 'call', date: '', sourceMeetingRef: '' }] },
      ],
    }).accounts;
    expect(needsAttention(accounts)).toEqual([]);
  });

  it('openActionItems counts only open obligation types', () => {
    const account: ProjectedAccount = normalizeProjection({
      accounts: [
        {
          id: '1',
          name: 'A',
          timeline: [
            { type: 'action_item', text: 'a', date: '', sourceMeetingRef: '' },
            { type: 'commitment', text: 'b', date: '', sourceMeetingRef: '' },
            { type: 'decision', text: 'c', date: '', sourceMeetingRef: '' },
          ],
        },
      ],
    }).accounts[0];
    expect(openActionItems(account)).toHaveLength(2);
  });
});

// ── buildAccountDetail (the single end-to-end surface) ───────────────────────

describe('buildAccountDetail', () => {
  it('assembles demo origin, pipeline stage, contract, latest invoice, and timeline in one surface', () => {
    const p = normalizeProjection(fixtureProjection());
    const acme = findAccount(p, 'ent_acme')!;
    const detail = buildAccountDetail(acme);

    // Demo origin (Inbound·Neon).
    expect(detail.demoOrigin?.system).toBe('neon');
    expect(detail.demoOrigin?.ref).toBe('req_001');
    // Pipeline stage.
    expect(detail.stageLabel).toBe('Signed');
    // Contract status (PandaDoc).
    expect(detail.contract?.system).toBe('pandadoc');
    expect(detail.contract?.status).toBe('signed');
    // Latest invoice + status (Stripe).
    expect(detail.latestInvoice?.system).toBe('stripe');
    expect(detail.latestInvoice?.status).toBe('paid');
    // Activity timeline from meetings & signals.
    expect(detail.timeline).toHaveLength(3);
    expect(detail.timeline[0].type).toBe('decision');

    // All four source cards present, each carrying a resolved tone.
    expect(detail.sourceCards.map((c) => c.role)).toEqual([
      'inbound',
      'pipeline',
      'contract',
      'billing',
    ]);
    expect(detail.sourceCards.find((c) => c.role === 'billing')?.tone).toBe('ok');
  });

  it('builds the lifecycle rail with the current stage ringed', () => {
    const acme = normalizeProjection(fixtureProjection()).accounts.find((a) => a.id === 'ent_acme')!;
    const detail = buildAccountDetail(acme);
    const current = detail.lifecycle.find((s) => s.state === 'current');
    expect(current?.stage).toBe('signed');
    // Stages before signed are past; active (after) is future.
    expect(detail.lifecycle.find((s) => s.stage === 'demo')?.state).toBe('past');
    expect(detail.lifecycle.find((s) => s.stage === 'active')?.state).toBe('future');
  });

  it('degrades gracefully when a leg (contract / billing) is missing', () => {
    const p = normalizeProjection(fixtureProjection());
    const beta = findAccount(p, 'ent_lead')!;
    const detail = buildAccountDetail(beta);

    // Inbound present, but contract + billing legs are absent → null (em-dash chip).
    expect(detail.demoOrigin?.system).toBe('neon');
    expect(detail.contract).toBeNull();
    expect(detail.latestInvoice).toBeNull();
    // The cards still render all four roles; the missing ones carry no source.
    expect(detail.sourceCards).toHaveLength(4);
    expect(detail.sourceCards.find((c) => c.role === 'contract')?.source).toBeNull();
    expect(detail.sourceCards.find((c) => c.role === 'contract')?.tone).toBe('idle');
  });

  it('puts a lead before the lifecycle rail (every step future)', () => {
    const beta = normalizeProjection(fixtureProjection()).accounts.find((a) => a.id === 'ent_lead')!;
    const detail = buildAccountDetail(beta);
    expect(detail.lifecycle.every((s) => s.state === 'future')).toBe(true);
  });
});

// ── small display helpers ────────────────────────────────────────────────────

describe('display helpers', () => {
  it('orDash falls back to an em-dash for empty / missing values', () => {
    expect(orDash('hello')).toBe('hello');
    expect(orDash('')).toBe('—');
    expect(orDash('   ')).toBe('—');
    expect(orDash(null)).toBe('—');
    expect(orDash(undefined)).toBe('—');
  });

  it('relativeTime renders a human age and em-dashes empties', () => {
    const now = Date.parse('2026-06-15T00:00:00Z');
    expect(relativeTime('2026-06-15T00:00:00Z', now)).toBe('just now');
    expect(relativeTime('2026-06-14T22:00:00Z', now)).toBe('2h ago');
    expect(relativeTime('', now)).toBe('—');
    expect(relativeTime('not-a-date', now)).toBe('—');
  });
});
