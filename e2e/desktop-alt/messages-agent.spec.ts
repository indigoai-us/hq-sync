import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('desktop-alt Messages agent handoff', () => {
  const shell = readRepoFile('src/components/messaging/MessagesShell.svelte');

  it('routes the Your agent conversation to Claude Code instead of a fake DM', () => {
    expect(shell).toContain("import { buildClaudeCodeUrl } from '../../lib/claude-code-link'");
    expect(shell).toContain('function sendAgentPrompt(text: string)');
    expect(shell).toContain('buildClaudeCodeUrl({ folder: hqFolderPath, prompt })');
    expect(shell).toContain("invoke('open_claude_code_link', { url })");
    expect(shell).toContain("personUid: 'agent:self'");
    expect(shell).not.toContain("personUid: selfPersonUid ?? 'agent:self'");
  });

  it('keeps agent handoff out of DM-only features', () => {
    expect(shell).toContain("peer.source === 'agent'");
    expect(shell).toContain("selected.source === 'agent'");
    expect(shell).toContain("selected.source === 'agent' ? {} : (dmReactions?.map ?? {})");
    expect(shell).toContain(
      "selected.source === 'agent' ? undefined : dmReactions?.toggle",
    );
  });
});
