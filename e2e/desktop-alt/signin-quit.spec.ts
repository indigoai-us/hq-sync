import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('sign-in prompt quit escape', () => {
  const source = readRepoFile('src/components/SignInPrompt.svelte');

  it('wires the quit control to the app quit command', () => {
    expect(source).toContain('async function handleQuit()');
    expect(source).toContain("await invoke('quit_app')");
    expect(source).toContain("console.error('Failed to quit:', e)");
    expect(source).toContain('onclick={handleQuit}');
  });

  it('renders a user-visible quit button', () => {
    expect(source).toMatch(/<button\b[^>]*onclick=\{handleQuit\}[^>]*>\s*Quit\b/i);
  });

  it('keeps the quit button usable while waiting for the browser', () => {
    const quitButton = source.match(/<button\b[^>]*onclick=\{handleQuit\}[^>]*>[\s\S]*?<\/button>/);

    expect(source).toContain('Waiting for browser…');
    expect(quitButton?.[0]).toBeDefined();
    expect(quitButton?.[0]).not.toMatch(/disabled\s*=\s*\{\s*loadingProvider\b/);
  });
});
