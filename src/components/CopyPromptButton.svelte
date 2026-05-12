<script lang="ts">
  import { buildPrompt, type Issue } from '../lib/copy-prompts';

  interface Props {
    issue: Issue;
    /** Layout variant. "inline" sits inside a banner / row at small weight.
     *  "compact" is icon-only with the label hidden until hover (for dense
     *  surfaces like the SyncStats conflict row). */
    variant?: 'inline' | 'compact';
    /** Optional override label. Default: "Copy prompt". */
    label?: string;
  }

  let { issue, variant = 'inline', label = 'Copy prompt' }: Props = $props();

  let copied = $state(false);
  let copyError = $state<string | null>(null);

  async function copy() {
    const text = buildPrompt(issue);
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      copyError = null;
      setTimeout(() => (copied = false), 1500);
    } catch (e) {
      copyError = e instanceof Error ? e.message : String(e);
      console.error('CopyPromptButton clipboard write failed:', e);
      setTimeout(() => (copyError = null), 2500);
    }
  }
</script>

<button
  type="button"
  class="copy-prompt-btn"
  class:compact={variant === 'compact'}
  class:copied
  class:error={!!copyError}
  onclick={copy}
  title={copyError ?? `Copy a prompt for an HQ agent (Codex or Claude) to resolve this`}
  aria-label={`${label} for an HQ agent`}
>
  {#if copied}
    <!-- check -->
    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <path d="M3.5 8.5l3 3 6-6.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  {:else}
    <!-- clipboard -->
    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <rect x="4" y="3" width="8" height="11" rx="1.2" stroke="currentColor" stroke-width="1.4" />
      <path d="M6 3V2.2a.7.7 0 0 1 .7-.7h2.6a.7.7 0 0 1 .7.7V3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  {/if}
  <span class="copy-prompt-label">
    {copied ? 'Copied' : copyError ? 'Copy failed' : label}
  </span>
</button>

<style>
  .copy-prompt-btn {
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

  .copy-prompt-btn:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .copy-prompt-btn:active {
    background: var(--popover-surface-strong, rgba(255, 255, 255, 0.16));
  }

  .copy-prompt-btn.copied {
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    border-color: var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  /* Failure state is still grey — per design rule, no severity colour. The
     label flips to "Copy failed" and the tooltip carries the error text. */
  .copy-prompt-btn.error {
    opacity: 0.85;
  }

  .copy-prompt-label {
    line-height: 1;
  }

  /* Compact variant: icon-only by default, label appears via tooltip. Used
     in dense rows (SyncStats conflict line). */
  .copy-prompt-btn.compact {
    padding: 0.1875rem 0.3125rem;
  }

  .copy-prompt-btn.compact .copy-prompt-label {
    /* Visually hidden but readable to a screen reader. */
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
