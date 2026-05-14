import { describe, expect, it } from 'vitest';
import {
  buildPrompt,
  parseLocalEnvFailure,
  type Issue,
  type IssueKind,
} from './copy-prompts';

const ALL_KINDS: IssueKind[] = [
  'sync-conflict',
  'sync-failed',
  'auth-expired',
  'app-update-available',
  'hq-cli-update-available',
  'hq-cli-update-failed',
  'cloud-unreachable',
  'manifest-error',
  'workspace-needs-connect',
  'workspace-broken',
  'repair-company',
  'local-env-failure',
];

describe('buildPrompt', () => {
  it.each(ALL_KINDS)('returns a non-empty prompt for kind "%s"', (kind) => {
    const out = buildPrompt({ kind });
    expect(out.length).toBeGreaterThan(50);
  });

  it('falls back to a generic prompt for an unknown kind', () => {
    const out = buildPrompt({ kind: 'made-up-kind' as IssueKind });
    expect(out).toContain('unknown issue kind');
    expect(out).toContain('made-up-kind');
  });

  it('embeds the conflict count in sync-conflict prompts', () => {
    const out = buildPrompt({ kind: 'sync-conflict', payload: { count: 7 } });
    expect(out).toContain('7 file conflicts');
  });

  it('singularises a single conflict', () => {
    const out = buildPrompt({ kind: 'sync-conflict', payload: { count: 1 } });
    expect(out).toContain('1 file conflict');
    expect(out).not.toContain('1 file conflicts');
  });

  it('mentions the offending company in sync-failed when provided', () => {
    const out = buildPrompt({
      kind: 'sync-failed',
      payload: { company: 'indigo', message: 'NET_FAIL: connection reset' },
    });
    expect(out).toContain('indigo');
    expect(out).toContain('NET_FAIL: connection reset');
  });

  it('points at the current log + per-slug journal paths in sync-failed', () => {
    // ADR-0001 Phase 5 moved the diagnostic log to ~/.hq/logs/hq-sync.log and
    // split the sync journal per-slug — guard against the prompt drifting back
    // to the legacy paths.
    const withCompany = buildPrompt({
      kind: 'sync-failed',
      payload: { company: 'indigo' },
    });
    expect(withCompany).toContain('~/.hq/logs/hq-sync.log');
    expect(withCompany).toContain('~/.hq/sync-journal.indigo.json');
    expect(withCompany).not.toContain('~/.hq/sync-debug.log');
    expect(withCompany).not.toMatch(/sync-journal\.log\b/);

    const noCompany = buildPrompt({ kind: 'sync-failed' });
    expect(noCompany).toContain('~/.hq/sync-journal.<slug>.json');
  });

  it('embeds the version in app-update-available', () => {
    const out = buildPrompt({
      kind: 'app-update-available',
      payload: { version: '0.1.99' },
    });
    expect(out).toContain('v0.1.99');
  });

  it('includes the local→latest delta in hq-cli-update-available', () => {
    const out = buildPrompt({
      kind: 'hq-cli-update-available',
      payload: { local: '0.5.0', latest: '0.7.2' },
    });
    expect(out).toContain('v0.5.0');
    expect(out).toContain('v0.7.2');
  });

  it('surfaces the parser error in manifest-error prompts', () => {
    const out = buildPrompt({
      kind: 'manifest-error',
      payload: { error: 'unexpected token at line 12' },
    });
    expect(out).toContain('unexpected token at line 12');
  });

  it('embeds slug and reason in workspace-broken', () => {
    const out = buildPrompt({
      kind: 'workspace-broken',
      payload: { slug: 'amass', reason: 'cloud_uid mismatch' },
    });
    expect(out).toContain('amass');
    expect(out).toContain('cloud_uid mismatch');
  });

  it('handles missing payload gracefully', () => {
    const issue: Issue = { kind: 'sync-conflict' };
    const out = buildPrompt(issue);
    // Falls back to generic copy without count
    expect(out).toContain('sync conflicts');
    expect(out).not.toContain('NaN');
  });

  it('refers to HQ skills the agent can invoke', () => {
    // The whole point of the registry is to give the receiving agent
    // concrete next actions — guard against the templates becoming generic
    // "please help me" prose.
    expect(buildPrompt({ kind: 'sync-conflict' })).toContain('/resolve-conflicts');
    expect(buildPrompt({ kind: 'auth-expired' })).toContain('/hq-login');
    expect(buildPrompt({ kind: 'sync-failed' })).toMatch(/\/diagnose|\/investigate/);
  });

  describe('local-env-failure prompts', () => {
    // The local-env-failure prompts are "attempt the fix" by design — the
    // popover button on `LocalEnv` errors wires to action, not just diagnose.
    // Guard the action-oriented language so a future refactor can't quietly
    // downgrade them to "please investigate" copy.

    it('npm-cache-permission instructs the agent to run chown', () => {
      const out = buildPrompt({
        kind: 'local-env-failure',
        payload: {
          slug: 'new-co',
          kind: 'npm-cache-permission',
          detail: 'npm error path /Users/foo/.npm/_cacache/x',
        },
      });
      expect(out).toContain('npm-cache-permission');
      expect(out).toContain('chown -R');
      expect(out).toContain('~/.npm');
      // Action-language guard.
      expect(out).toMatch(/attempt the fix/i);
      // The detail string should be threaded into the prompt so the agent
      // can see the offending path.
      expect(out).toContain('/Users/foo/.npm/_cacache/x');
    });

    it('disk-full instructs the agent to free space, not auto-delete', () => {
      const out = buildPrompt({
        kind: 'local-env-failure',
        payload: { slug: 'x', kind: 'disk-full', detail: 'ENOSPC' },
      });
      expect(out).toContain('disk-full');
      expect(out).toMatch(/df -h|free space|reclaim/i);
      // Critical safety guardrail: never auto-delete in disk-full prompts.
      expect(out).toMatch(/don.?t auto-delete|recommend.*don.?t|with my confirmation/i);
    });

    it('npm-registry-unreachable proposes a registry config check', () => {
      const out = buildPrompt({
        kind: 'local-env-failure',
        payload: { slug: 'x', kind: 'npm-registry-unreachable', detail: 'ENOTFOUND' },
      });
      expect(out).toContain('npm-registry-unreachable');
      expect(out).toMatch(/npm config get registry/);
    });

    it('npm-registry-timeout points at proxy + status page', () => {
      const out = buildPrompt({
        kind: 'local-env-failure',
        payload: { slug: 'x', kind: 'npm-registry-timeout', detail: 'ETIMEDOUT' },
      });
      expect(out).toContain('npm-registry-timeout');
      expect(out).toMatch(/status\.npmjs\.org|proxy/);
    });

    it('falls back gracefully when kind is unknown', () => {
      const out = buildPrompt({
        kind: 'local-env-failure',
        payload: { slug: 'x', kind: 'some-future-kind', detail: 'whatever' },
      });
      // Must still be useful — points at the diagnostic log + no `sudo`
      // without confirmation. Don't crash, don't silent-fall-through.
      expect(out).toMatch(/~\/.hq\/logs|hq-sync\.log/);
      expect(out).toMatch(/sudo.*confirm|don.?t.*sudo/i);
    });
  });
});

describe('parseLocalEnvFailure (IPC contract)', () => {
  // This regex is the wire format between Rust and the popover — if Rust's
  // `CliProvisionError::LocalEnv` Display impl is reworded, both sides need
  // to update. The Rust side has `local_env_display_contract_is_parseable`
  // covering the same boundary; this is the TS half.

  it('extracts kind + detail from the canonical Display string', () => {
    const parsed = parseLocalEnvFailure(
      'local environment failure (npm-cache-permission): npm error path /Users/foo/.npm/_cacache/x',
    );
    expect(parsed).toEqual({
      kind: 'npm-cache-permission',
      detail: 'npm error path /Users/foo/.npm/_cacache/x',
    });
  });

  it('handles every known kind', () => {
    const kinds = [
      'npm-cache-permission',
      'disk-full',
      'npm-registry-unreachable',
      'npm-registry-timeout',
    ];
    for (const k of kinds) {
      const parsed = parseLocalEnvFailure(`local environment failure (${k}): detail text`);
      expect(parsed).not.toBeNull();
      expect(parsed?.kind).toBe(k);
      expect(parsed?.detail).toBe('detail text');
    }
  });

  it('returns null for vault errors so they route to the generic branch', () => {
    expect(
      parseLocalEnvFailure(
        'vault/network error from `hq cloud provision`: exit 1 (vault) — see ~/.hq/logs/hq-sync.log',
      ),
    ).toBeNull();
  });

  it('returns null for unknown kinds (defensive against wire drift)', () => {
    // If Rust ships a new kind before TS knows about it, we must not render
    // a "Fix in Claude Code" button for it — better to fall through to the
    // existing generic-error retry treatment.
    expect(
      parseLocalEnvFailure('local environment failure (mystery-kind): something'),
    ).toBeNull();
  });

  it('returns null for empty / malformed input', () => {
    expect(parseLocalEnvFailure('')).toBeNull();
    expect(parseLocalEnvFailure('not a local env failure at all')).toBeNull();
    expect(parseLocalEnvFailure('local environment failure: no kind')).toBeNull();
  });
});
