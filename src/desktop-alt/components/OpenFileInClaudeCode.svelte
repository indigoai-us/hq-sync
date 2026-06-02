<script lang="ts">
  /**
   * OpenFileInClaudeCode — desktop-alt drill-in affordance that opens a single
   * file in Claude Code (US-012).
   *
   * REUSE, do not reimplement: the Claude Code link is built by the existing
   * `buildClaudeCodeUrl` util (src/lib/claude-code-link.ts) and dispatched
   * through the same `open_claude_code_link` Tauri command that
   * `src/components/OpenInClaudeCodeButton.svelte` uses for the "Fix in Claude
   * Code" CTA. The command (src-tauri/src/commands/app.rs) rejects any non-
   * `claude://` URL and shells out to macOS `open`, so we never widen
   * `shell:allow-open` to arbitrary schemes.
   *
   * Where `OpenInClaudeCodeButton` is error-oriented (it takes an `Issue` and
   * renders a remediation prompt from `copy-prompts.ts`), this variant is
   * file-oriented: it builds a short prompt asking the agent to open the given
   * file, with `folder` set to the user's HQ root so the session starts with
   * full repo context.
   *
   * Limitation (noted per AC1): the desktop-alt story/activity data carries a
   * repo-relative or display path, not a verified absolute path. We therefore
   * set the session `folder` to the HQ root (`hqFolderPath`) and pass the file
   * path in the prompt — the agent resolves it inside the HQ tree. When
   * `folder` is empty (config not loaded yet) the affordance suppresses itself,
   * matching `OpenInClaudeCodeButton`'s contract.
   */
  import { invoke } from '@tauri-apps/api/core';
  import { buildClaudeCodeUrl } from '../../lib/claude-code-link';

  interface Props {
    /** The file path to open. Repo-/HQ-relative or display path — surfaced as-is
     *  from the story `files` list or an activity entry's `file`. */
    file: string;
    /** Absolute HQ root the Claude Code session should `cwd` into. Caller passes
     *  `config.hqFolderPath`. Empty → the affordance suppresses itself. */
    folder: string;
    /** Layout variant. `inline` shows the label; `compact` hides it until hover
     *  (visually-hidden) for dense rows. Mirrors `OpenInClaudeCodeButton`. */
    variant?: 'inline' | 'compact';
    /** Optional override label. Default: "Open in Claude Code". */
    label?: string;
  }

  let { file, folder, variant = 'inline', label = 'Open in Claude Code' }: Props =
    $props();

  let dispatched = $state(false);
  let dispatchError = $state<string | null>(null);
  let copiedFallback = $state(false);

  /** Short, file-scoped prompt handed to the new Claude Code session. */
  function buildOpenPrompt(path: string): string {
    return [
      `Open the file \`${path}\` from my HQ project and show me its contents.`,
      '',
      'Resolve the path inside my HQ folder (the session is already rooted there). If it is repo-relative, find the matching repo under `repos/`. Once open, give me a one-line summary of what the file does and wait for my next instruction.',
    ].join('\n');
  }

  async function dispatch() {
    if (!folder) return;
    const prompt = buildOpenPrompt(file);
    const url = buildClaudeCodeUrl({ folder, prompt });
    dispatchError = null;
    copiedFallback = false;
    try {
      // Same dispatch path as OpenInClaudeCodeButton — the dedicated Tauri
      // command, NOT plugin-shell's open(), so the URL scheme stays locked to
      // `claude://`.
      await invoke('open_claude_code_link', { url });
      dispatched = true;
      setTimeout(() => (dispatched = false), 1800);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error('OpenFileInClaudeCode dispatch failed:', e);
      // Fallback: copy the prompt so the user still has it in hand and the
      // tooltip flips to a clear hint — mirrors OpenInClaudeCodeButton.
      try {
        await navigator.clipboard.writeText(prompt);
        copiedFallback = true;
        dispatchError = 'Claude Code not installed — prompt copied instead';
        setTimeout(() => {
          copiedFallback = false;
          dispatchError = null;
        }, 4000);
      } catch {
        dispatchError = msg;
        setTimeout(() => (dispatchError = null), 4000);
      }
    }
  }
</script>

{#if folder}
  <button
    type="button"
    class="open-claude-btn"
    class:compact={variant === 'compact'}
    class:dispatched
    class:fallback={copiedFallback}
    class:error={!!dispatchError && !copiedFallback}
    data-testid="open-in-claude-code"
    onclick={dispatch}
    title={dispatchError ?? `Open ${file} in Claude Code`}
    aria-label={`${label}: ${file}`}
  >
    {#if dispatched}
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M3.5 8.5l3 3 6-6.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    {:else}
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M8 2.5l1.4 3.6 3.6 1.4-3.6 1.4L8 12.5 6.6 8.9 3 7.5l3.6-1.4L8 2.5z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" />
      </svg>
    {/if}
    <span class="open-claude-label">
      {#if dispatched}
        Opened
      {:else if copiedFallback}
        Prompt copied
      {:else if dispatchError}
        Failed
      {:else}
        {label}
      {/if}
    </span>
  </button>
{/if}

<style>
  .open-claude-btn {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    gap: var(--space-1);
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-2);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease,
      border-color 140ms ease;
  }

  .open-claude-btn:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    color: var(--fg);
  }

  .open-claude-btn:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .open-claude-btn.dispatched,
  .open-claude-btn.fallback {
    color: var(--fg);
    border-color: var(--border-strong);
  }

  .open-claude-btn.error {
    opacity: 0.85;
  }

  .open-claude-label {
    line-height: 1;
  }

  .open-claude-btn.compact .open-claude-label {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
</style>
