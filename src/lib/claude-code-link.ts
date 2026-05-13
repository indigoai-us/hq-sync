/**
 * Claude Code deep-link builder.
 *
 * On macOS, the Claude Code desktop app registers the `claude://` URL scheme.
 * Opening `claude://open?cwd=<path>&prompt=<text>` launches a new session
 * inside `<path>` with `<text>` prefilled as the first user message — the
 * exact UX we want for "Fix in Claude Code" buttons in the HQ Sync popover.
 *
 * If Claude Code is not installed, the system handler falls through and the
 * `tauri-plugin-shell` `open()` call returns an error; callers should fall
 * back to clipboard-copy in that case so the prompt isn't lost.
 *
 * Keep the URL builder pure and side-effect-free — actual dispatch happens
 * in `OpenInClaudeCodeButton.svelte` so tests can compare URLs without
 * touching the shell plugin.
 */

export interface ClaudeCodeLinkInput {
  /** Absolute path the Claude Code session should `cwd` into. Typically the
   *  HQ folder root from `get_config`'s `hqFolderPath`. */
  cwd: string;
  /** Prefilled prompt text. Multi-line is fine — encoding handles newlines. */
  prompt: string;
}

/**
 * Build a `claude://open?cwd=…&prompt=…` URL. Both values are
 * percent-encoded with `encodeURIComponent` so spaces, newlines, and unicode
 * survive intact through macOS's URL dispatcher.
 */
export function buildClaudeCodeUrl({ cwd, prompt }: ClaudeCodeLinkInput): string {
  const cwdParam = encodeURIComponent(cwd);
  const promptParam = encodeURIComponent(prompt);
  return `claude://open?cwd=${cwdParam}&prompt=${promptParam}`;
}
