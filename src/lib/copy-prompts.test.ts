import { describe, expect, it } from 'vitest';
import { buildPrompt, type Issue, type IssueKind } from './copy-prompts';

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
});
