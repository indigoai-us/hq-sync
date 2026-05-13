<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { buildPrompt, type Issue } from '../lib/copy-prompts';
  import { buildClaudeCodeUrl } from '../lib/claude-code-link';

  interface Props {
    /** Issue descriptor — the same shape `CopyPromptButton` accepts. The
     *  prompt template per kind lives in `lib/copy-prompts.ts`. */
    issue: Issue;
    /** Absolute path the Claude Code session should open as its working
     *  folder. Caller passes `config.hqFolderPath` so the agent starts
     *  inside the user's HQ root with full repo context. Empty = the
     *  caller is responsible for suppressing the button. */
    folder: string;
    /** Layout variant. `inline` matches the row-meta error line. `compact`
     *  hides the label until hover for dense surfaces. Mirrors the variants
     *  on `CopyPromptButton` so the two buttons can sit side-by-side without
     *  visual drift. */
    variant?: 'inline' | 'compact';
    /** Optional override label. Default: "Fix in Claude Code". */
    label?: string;
  }

  let {
    issue,
    folder,
    variant = 'inline',
    label = 'Fix in Claude Code',
  }: Props = $props();

  let dispatched = $state(false);
  let dispatchError = $state<string | null>(null);
  let copiedFallback = $state(false);

  async function dispatch() {
    const prompt = buildPrompt(issue);
    const url = buildClaudeCodeUrl({ folder, prompt });
    dispatchError = null;
    copiedFallback = false;
    try {
      // Dispatch via the `open_claude_code_link` Tauri command — same path
      // `Popover::fixHqCliUpdateInHq` uses for the hq-cli "Fix this in HQ"
      // CTA. The dedicated command (src-tauri/src/commands/app.rs) rejects
      // any URL that isn't `claude://*` and shells out to macOS `open`,
      // which routes to the Claude Code desktop app if installed.
      //
      // We deliberately do NOT use `@tauri-apps/plugin-shell::open()` here
      // — that would require widening `shell:allow-open` to handle
      // arbitrary schemes, which is a much larger attack surface than the
      // single-purpose Rust command.
      await invoke('open_claude_code_link', { url });
      dispatched = true;
      setTimeout(() => (dispatched = false), 1800);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error('OpenInClaudeCodeButton dispatch failed:', e);
      // Fallback: copy the prompt to the clipboard so the user still has
      // the templated remediation in hand. Tooltip flips to a clear "not
      // installed" hint so the user knows the next step rather than just
      // seeing a generic error.
      try {
        await navigator.clipboard.writeText(prompt);
        copiedFallback = true;
        dispatchError = 'Claude Code not installed — prompt copied instead';
        setTimeout(() => {
          copiedFallback = false;
          dispatchError = null;
        }, 4000);
      } catch (copyErr) {
        dispatchError = msg;
        setTimeout(() => (dispatchError = null), 4000);
      }
    }
  }
</script>

<button
  type="button"
  class="open-claude-btn"
  class:compact={variant === 'compact'}
  class:dispatched
  class:fallback={copiedFallback}
  class:error={!!dispatchError && !copiedFallback}
  onclick={dispatch}
  title={dispatchError ?? `Open Claude Code in your HQ folder with this error preloaded as a prompt`}
  aria-label={`${label} — open this error in Claude Code with a prefilled fix prompt`}
>
  {#if dispatched}
    <!-- check -->
    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <path d="M3.5 8.5l3 3 6-6.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  {:else}
    <!-- sparkle / send icon — distinct from CopyPromptButton's clipboard glyph -->
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

<style>
  .open-claude-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.3125rem;
    padding: 0.25rem 0.5rem;
    font-family: inherit;
    font-size: 0.6875rem;
    font-weight: 500;
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.52));
    background: var(--popover-surface, rgba(255, 255, 255, 0.06));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    border-radius: 6px;
    cursor: pointer;
    transition: background-color 0.12s ease, color 0.12s ease, border-color 0.12s ease;
    flex-shrink: 0;
    white-space: nowrap;
  }

  .open-claude-btn:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .open-claude-btn:active {
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
  }

  .open-claude-btn.dispatched,
  .open-claude-btn.fallback {
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  /* Failure stays grey — same design rule as CopyPromptButton: no severity
     colour, the tooltip carries the error text. */
  .open-claude-btn.error {
    opacity: 0.85;
  }

  .open-claude-label {
    line-height: 1;
  }

  .open-claude-btn.compact {
    padding: 0.1875rem 0.3125rem;
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
