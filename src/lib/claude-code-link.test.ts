import { describe, expect, it } from 'vitest';
import { buildClaudeCodeUrl } from './claude-code-link';

// These tests are deliberately strict on shape — the URL is parsed by the
// Claude Code desktop app, which only recognises the `claude://code/new`
// path with `q` + `folder` parameters. The same shape is used by
// `Popover::fixHqCliUpdateInHq` for the hq-cli auto-update "Fix this in HQ"
// banner; if this contract drifts, the popover button will report success
// (the Tauri `open` succeeds) while Claude Code drops the prompt silently.

describe('buildClaudeCodeUrl', () => {
  it('targets claude://code/new (NOT claude://open)', () => {
    const url = buildClaudeCodeUrl({ folder: '/Users/x/HQ', prompt: 'hi' });
    expect(url.startsWith('claude://code/new?')).toBe(true);
    expect(url).not.toContain('claude://open');
  });

  it('puts the prompt under the `q` parameter (NOT `prompt`)', () => {
    const url = buildClaudeCodeUrl({ folder: '/p', prompt: 'remediate my npm cache' });
    const parsed = new URL(url);
    expect(parsed.searchParams.get('q')).toBe('remediate my npm cache');
    expect(parsed.searchParams.get('prompt')).toBeNull();
  });

  it('puts the folder under the `folder` parameter (NOT `cwd`)', () => {
    const url = buildClaudeCodeUrl({ folder: '/Users/foo/HQ', prompt: 'p' });
    const parsed = new URL(url);
    expect(parsed.searchParams.get('folder')).toBe('/Users/foo/HQ');
    expect(parsed.searchParams.get('cwd')).toBeNull();
  });

  it('omits the folder parameter when empty so the URL still parses', () => {
    const url = buildClaudeCodeUrl({ folder: '', prompt: 'p' });
    const parsed = new URL(url);
    expect(parsed.searchParams.get('q')).toBe('p');
    expect(parsed.searchParams.has('folder')).toBe(false);
  });

  it('correctly encodes multi-line prompts with special characters', () => {
    const prompt = "Run `sudo chown -R $(id -u):$(id -g) ~/.npm`\nThen retry.";
    const url = buildClaudeCodeUrl({ folder: '/p', prompt });
    const parsed = new URL(url);
    // URLSearchParams round-trips ASCII control + special chars correctly;
    // assert exact preservation through the encode/decode boundary.
    expect(parsed.searchParams.get('q')).toBe(prompt);
  });

  it('produces URLs the open_claude_code_link Tauri command accepts', () => {
    // The Rust command (src-tauri/src/commands/app.rs::open_claude_code_link)
    // requires the URL to start with `claude://`. Lock that.
    const url = buildClaudeCodeUrl({ folder: '/p', prompt: 'p' });
    expect(url.startsWith('claude://')).toBe(true);
  });
});
