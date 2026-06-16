/**
 * Story-contract test for the hq-native-crm US-010 Accounts view.
 *
 * The hq-sync vitest suite runs in the `node` environment (no jsdom/happy-dom),
 * so the repo's convention for component contracts is to read the source at
 * module level and assert the wiring with normalized-string `.toContain` checks
 * (see policy hq-sync-story-tests-read-source-at-module-level). This file pins
 * the load-bearing contracts of the read-only Accounts surface:
 *
 *   • the `accounts` CompanyTab is registered on the V4 route union + sidebar;
 *   • CompanyPage renders AccountView for the accounts tab;
 *   • AccountView reads the vault-synced projection via the local-first +
 *     vault-API loader and makes NO network call to Attio / Stripe / PandaDoc /
 *     Neon (the US-010 e2e contract: one surface, zero external network);
 *   • the detail surface renders demo origin + pipeline stage + contract status
 *     + latest invoice + the meetings/signals timeline + the ontology footer.
 *
 * The behavioral guarantees (grouping, needs-attention, detail assembly,
 * graceful degradation from a FIXTURE projection) are proven against real data
 * in `account-view-model.test.ts`; this file proves the component is wired to
 * that model and to the no-network read path.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  buildAccountDetail,
  findAccount,
  normalizeProjection,
} from './account-view-model';

const accountView = readFileSync(
  resolve(process.cwd(), 'src/lib/crm/AccountView.svelte'),
  'utf8',
);
const crmLoader = readFileSync(
  resolve(process.cwd(), 'src/lib/crm/crm-projection.ts'),
  'utf8',
);
const routeSource = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/route.ts'),
  'utf8',
);
const companyPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompanyPage.svelte'),
  'utf8',
);
const rustMain = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/main.rs'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

/**
 * Strip comments + the <style> block so policy/no-network assertions inspect the
 * EXECUTABLE source only. The component documents the policies it follows
 * ("indigo-no-purple", "no network to Attio/Stripe/PandaDoc") in prose, and the
 * footer literally reads "no network to Attio/Stripe/PandaDoc" — neither is a
 * forbidden CALL. We assert there is no Attio/Stripe network primitive in the
 * code, not that the words never appear in documentation.
 */
function codeOnly(source: string): string {
  return source
    .replace(/<style[\s\S]*?<\/style>/g, '') // drop CSS
    .replace(/<!--[\s\S]*?-->/g, '') // HTML comments
    .replace(/\/\*[\s\S]*?\*\//g, '') // block comments
    .replace(/(^|[^:])\/\/.*$/gm, '$1'); // line comments
}

const NORM_VIEW = normalize(accountView);
const NORM_LOADER = normalize(crmLoader);
const NORM_ROUTE = normalize(routeSource);
const NORM_PAGE = normalize(companyPage);
const NORM_MAIN = normalize(rustMain);
const VIEW_CODE = codeOnly(accountView).toLowerCase();
const LOADER_CODE = codeOnly(crmLoader).toLowerCase();

// ── route + tab registration ─────────────────────────────────────────────────

describe('US-010: accounts CompanyTab registration', () => {
  it("adds 'accounts' to the CompanyTab route union", () => {
    expect(NORM_ROUTE).toContain("| 'accounts'");
  });

  it("registers an Accounts row in COMPANY_SECTIONS", () => {
    expect(NORM_ROUTE).toContain("{ id: 'accounts', label: 'Accounts' }");
  });

  it('CompanyPage renders AccountView for the accounts tab', () => {
    expect(NORM_PAGE).toContain("import AccountView from '../../lib/crm/AccountView.svelte'");
    expect(NORM_PAGE).toContain("tab === 'accounts'");
    expect(NORM_PAGE).toContain('<AccountView slug={company.slug} {cloudBacked} />');
  });
});

// ── no external network (the US-010 e2e contract) ────────────────────────────

describe('US-010: read-only, no external network', () => {
  it('AccountView reads the projection only through the local-first + vault loader', () => {
    expect(NORM_VIEW).toContain("import { loadCrmProjection } from './crm-projection'");
    expect(NORM_VIEW).toContain('loadCrmProjection(activeSlug)');
  });

  it('AccountView makes NO call to Attio / Stripe / PandaDoc / Neon (no network primitive in code)', () => {
    // The executable source carries no network primitive and no external URL —
    // every read goes through loadCrmProjection. The ONLY place the external
    // systems are named in code is the rendered footer literal that AFFIRMS
    // there is no network (the design-spec footer text), so we assert there are
    // no call primitives rather than that the words never appear.
    expect(VIEW_CODE).not.toContain('fetch(');
    expect(VIEW_CODE).not.toContain('xmlhttprequest');
    expect(VIEW_CODE).not.toContain('.send(');
    expect(VIEW_CODE).not.toMatch(/https?:\/\//);
    // The only external-system mention is the no-network footer claim.
    expect(NORM_VIEW).toContain('no network to Attio/Stripe/PandaDoc');
  });

  it('the loader reads local-first then falls back to the vault API (board.json pattern)', () => {
    expect(NORM_LOADER).toContain("invoke<unknown>('get_company_crm_projection', { companySlug: slug, })");
    expect(NORM_LOADER).toContain("invoke<unknown>('get_company_crm_projection_vault', { slug, })");
    // The loader normalizes through the pure model; no external system endpoints
    // in the executable source.
    expect(LOADER_CODE).not.toContain('attio');
    expect(LOADER_CODE).not.toContain('stripe');
    expect(LOADER_CODE).not.toMatch(/https?:\/\//);
  });

  it('both Rust read commands are registered in main.rs', () => {
    expect(NORM_MAIN).toContain('commands::projects_local::get_company_crm_projection');
    expect(NORM_MAIN).toContain('commands::desktop_alt::get_company_crm_projection_vault');
  });
});

// ── single end-to-end surface (list + detail) ────────────────────────────────

describe('US-010: list + detail rendered from one surface', () => {
  it('renders the list view (stage groups, needs-attention, status chips)', () => {
    expect(NORM_VIEW).toContain('groupAccountsByStage');
    expect(NORM_VIEW).toContain('needsAttention');
    expect(NORM_VIEW).toContain('data-testid="needs-attention"');
    expect(NORM_VIEW).toContain('data-testid="stage-group"');
    expect(NORM_VIEW).toContain('data-testid="contract-chip"');
    expect(NORM_VIEW).toContain('data-testid="billing-chip"');
  });

  it('renders the detail view legs + the ontology footer', () => {
    expect(NORM_VIEW).toContain('buildAccountDetail');
    // The detail object exposes demo origin / stage / contract / invoice; the
    // template renders them via the assembled sourceCards + stage + timeline.
    expect(NORM_VIEW).toContain('detail.stageLabel');
    expect(NORM_VIEW).toContain('detail.sourceCards');
    expect(NORM_VIEW).toContain('detail.lifecycle');
    expect(NORM_VIEW).toContain('detail.timeline');
    expect(NORM_VIEW).toContain('data-testid="source-card"');
    expect(NORM_VIEW).toContain('source of truth: ontology');
    expect(NORM_VIEW).toContain('READ ONLY');
  });

  it('renders empty + loading states (graceful degradation)', () => {
    expect(NORM_VIEW).toContain('data-testid="account-view-loading"');
    expect(NORM_VIEW).toContain('data-testid="account-view-empty"');
    expect(NORM_VIEW).toContain('data-testid="source-missing"');
  });

  it('uses only the V4 status-dot palette — no purple anywhere (indigo-no-purple)', () => {
    // No purple/violet hue or deck-indigo hex in the executable + CSS source.
    // (The word "indigo" appears only in policy-name prose, stripped here.)
    const noComments = accountView
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/(^|[^:])\/\/.*$/gm, '$1')
      .toLowerCase();
    expect(noComments).not.toContain('purple');
    expect(noComments).not.toContain('violet');
    expect(noComments).not.toContain('#818cf8'); // the deck indigo, forbidden in-app
    expect(noComments).not.toMatch(/#a78bfa|#6366f1|#7c3aed/);
    // Status colors come from the V4 tokens (the only color on the surface).
    expect(NORM_VIEW).toContain('var(--v4-ok)');
  });

  it('never puts data-tauri-drag-region on a container (indigo-tauri-drag-regions)', () => {
    expect(accountView).not.toContain('data-tauri-drag-region');
  });
});

// ── the e2e fixture renders end-to-end through the model the view consumes ────

describe('US-010 e2e: a synced new-client projection renders one end-to-end surface', () => {
  // The exact fixture the story's e2e test describes: a new client whose
  // projection carries demo origin + pipeline + contract + latest invoice +
  // meeting timeline. The view renders this via buildAccountDetail (asserted
  // here against the same pure model the component calls) with NO external
  // network — the projection is already-joined JSON.
  const fixture = {
    schema_version: 1,
    synced_at: '2026-06-15T00:00:00Z',
    accounts: [
      {
        id: 'ent_newclient',
        name: 'Newco FDE',
        domain: 'newco.ai',
        stage: 'signed',
        owner: 'corey',
        lastActivity: '2026-06-14T12:00:00Z',
        external_ids: { neon: 'req_77', attio: 'rec_77', pandadoc: 'doc_77', stripe: 'cus_77' },
        sources: {
          inbound: { system: 'neon', status: 'new', value: 'Demo request', meta: '2026-05-20', ref: 'req_77' },
          pipeline: { system: 'attio', status: 'demo_done', value: 'Demo done', meta: 'pipeline', ref: 'rec_77' },
          contract: { system: 'pandadoc', status: 'signed', value: '$60,000', meta: 'annual', ref: 'doc_77' },
          billing: { system: 'stripe', status: 'paid', value: '$5,000', meta: '2026-06-30', ref: 'cus_77' },
        },
        timeline: [
          { date: '2026-06-14', type: 'meeting', text: 'Onboarding kickoff', sourceMeetingRef: 'mtg_77' },
        ],
      },
    ],
  };

  it('demo origin, pipeline stage, contract status, latest invoice, and the meeting timeline all render in one surface', () => {
    const projection = normalizeProjection(fixture);
    const account = findAccount(projection, 'ent_newclient');
    expect(account).not.toBeNull();
    const detail = buildAccountDetail(account!);

    expect(detail.demoOrigin?.system).toBe('neon'); // demo origin
    expect(detail.stageLabel).toBe('Signed'); // pipeline stage
    expect(detail.contract?.status).toBe('signed'); // contract status
    expect(detail.latestInvoice?.status).toBe('paid'); // latest invoice + status
    expect(detail.timeline[0].text).toBe('Onboarding kickoff'); // meeting timeline
    // One surface = all four legs co-present on the same detail object.
    expect(detail.sourceCards.filter((c) => c.source !== null)).toHaveLength(4);
  });
});
