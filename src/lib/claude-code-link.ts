/**
 * Claude Code deep-link builder.
 *
 * The Claude Code desktop app registers `claude://code/new` as a deep link
 * for opening a new session, taking:
 *
 *   * `q`      — prefilled user prompt (the "question" / first message).
 *   * `folder` — absolute path to the working directory for the session.
 *
 * This is the same shape `Popover::fixHqCliUpdateInHq` already uses for the
 * "Fix this in HQ" CTA on the hq-cli auto-update banner; the keys + path
 * (`claude://code/new`) must match — Claude Code does NOT recognise the
 * `claude://open?cwd=...&prompt=...` shape (verified against current Claude
 * Code docs + the existing call site).
 *
 * Dispatch goes through the `open_claude_code_link` Tauri command (defined
 * in `src-tauri/src/commands/app.rs`), not the generic shell `open()`
 * plugin — the dedicated command keeps the surface tight (rejects non-
 * `claude://` URLs) so we don't have to widen `shell:allow-open` to the
 * world. See `OpenInClaudeCodeButton.svelte` for the call site.
 *
 * Kept pure and side-effect-free so the URL shape can be unit-tested in
 * isolation. A failing test here is the early-warning that the wire
 * contract has drifted from what Claude Code accepts.
 */

export interface ClaudeCodeLinkInput {
  /** Absolute path the Claude Code session should `cwd` into. Typically the
   *  HQ folder root from `get_config`'s `hqFolderPath`. Maps to the
   *  `folder` URL parameter. */
  folder: string;
  /** Prefilled prompt text. Multi-line is fine — `URLSearchParams`
   *  handles encoding. Maps to the `q` URL parameter. */
  prompt: string;
}

/**
 * Build a `claude://code/new?q=…&folder=…` URL. Both values are encoded by
 * `URLSearchParams` (which is what the existing `fixHqCliUpdateInHq` call
 * site uses — keep the two in sync).
 *
 * `folder` is omitted from the query when empty so the URL still parses
 * cleanly if the caller hasn't loaded `hqFolderPath` yet. The button
 * suppresses itself in that case, but defending the URL builder is cheap.
 */
export function buildClaudeCodeUrl({ folder, prompt }: ClaudeCodeLinkInput): string {
  const params = new URLSearchParams({ q: prompt });
  if (folder) params.set('folder', folder);
  return `claude://code/new?${params.toString()}`;
}
