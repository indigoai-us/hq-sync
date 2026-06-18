<script lang="ts">
  /**
   * FilePreviewPane — preview the selected company file beside the file tree
   * (US-004).
   *
   * Self-fetching + presentational, like the sibling detail panels. On every
   * `path` change it resets state and calls the `get_company_file_content`
   * Tauri command (US-001), which returns the UTF-8 text on success and REJECTS
   * for binary OR oversized files (size cap = MAX_FILE_BYTES). So a resolved
   * promise = text to render; a rejected promise = the "can't preview"
   * placeholder.
   *
   * Rendering:
   *   * `.md` / `.markdown` (case-insensitive) → rendered to safe HTML by the
   *     dependency-free, CSP-safe `lib/markdown.ts` renderer (same as
   *     LibraryDetailPanel / ProjectDetailView) into an `<article
   *     class="markdown-body">`. The `.markdown-body :global(...)` typography
   *     mirrors LibraryDetailPanel.
   *   * other text → a monospaced `<pre>{text}</pre>` (Svelte auto-escapes the
   *     bound text), whitespace preserved, horizontal scroll allowed.
   *   * binary/oversized (fetch rejected) → a friendly placeholder.
   *
   * The two open actions — "Open in Claude Code" (reuses
   * OpenFileInClaudeCode.svelte's claude:// custom-command path) and "Reveal in
   * Finder" (the already-granted `shell:allow-open` via plugin-shell `open()`) —
   * render in the header REGARDLESS of preview success, so they stay available
   * in the can't-preview state.
   *
   * No purple anywhere (hard Indigo policy).
   */
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-shell';
  import { renderMarkdown } from '../lib/markdown';
  import OpenFileInClaudeCode from './OpenFileInClaudeCode.svelte';
  import '../v4/tokens.css';

  interface Props {
    /** HQ-folder-relative, forward-slash path of the selected file. */
    path: string;
    /** Absolute HQ root (`get_config().hqFolderPath`). Empty → open actions
     *  that need an absolute path suppress themselves. */
    hqFolderPath: string;
  }

  let { path, hqFolderPath }: Props = $props();

  let content = $state<string | null>(null);
  let loading = $state(false);
  let unsupported = $state(false);
  let revealError = $state<string | null>(null);

  const fileName = $derived(path.split('/').pop() ?? path);
  const isMarkdown = $derived(/\.(md|markdown)$/i.test(path));

  // Build the absolute path by joining hqFolderPath + the relative FileNode
  // path with `/`. Guard against a trailing slash on hqFolderPath. Empty root →
  // no absolute path (Reveal suppresses itself).
  const absolutePath = $derived(
    hqFolderPath ? `${hqFolderPath.replace(/\/+$/, '')}/${path}` : '',
  );

  const markdownHtml = $derived(
    isMarkdown && content !== null ? renderMarkdown(content) : '',
  );

  // Fetch content on every `path` change. Cancel-flag guards against an
  // out-of-order completion when the user clicks through files quickly.
  $effect(() => {
    const current = path;
    content = null;
    unsupported = false;
    revealError = null;

    if (!current) {
      loading = false;
      return;
    }

    loading = true;
    let cancelled = false;

    void invoke<string>('get_company_file_content', { path: current })
      .then((text) => {
        if (!cancelled) {
          content = text;
          unsupported = false;
        }
      })
      .catch((err) => {
        // Reject = binary OR oversized OR unreadable → can't-preview state.
        console.error('get_company_file_content failed:', err);
        if (!cancelled) {
          content = null;
          unsupported = true;
        }
      })
      .finally(() => {
        if (!cancelled) loading = false;
      });

    return () => {
      cancelled = true;
    };
  });

  async function revealInFinder(): Promise<void> {
    if (!absolutePath) return;
    revealError = null;
    try {
      await open(absolutePath);
    } catch (err) {
      console.error('Reveal in Finder failed:', err);
      revealError = 'Could not reveal file';
      setTimeout(() => (revealError = null), 4000);
    }
  }
</script>

<section
  class="preview-pane"
  data-testid="file-preview-pane"
  aria-label={`Preview of ${fileName}`}
>
  <header class="preview-header">
    <h3 class="preview-name" title={path}>{fileName}</h3>
    <div class="preview-actions">
      <OpenFileInClaudeCode file={path} folder={hqFolderPath} variant="inline" />
      {#if absolutePath}
        <button
          type="button"
          class="reveal-btn"
          class:error={!!revealError}
          data-testid="reveal-in-finder"
          onclick={revealInFinder}
          title={revealError ?? `Reveal ${fileName} in Finder`}
          aria-label={`Reveal ${fileName} in Finder`}
        >
          <svg
            width="12"
            height="12"
            viewBox="0 0 16 16"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true"
          >
            <path
              d="M2 4.5a1 1 0 0 1 1-1h3l1.2 1.4H13a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4.5z"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linejoin="round"
            />
          </svg>
          <span class="reveal-label">
            {revealError ? 'Failed' : 'Reveal in Finder'}
          </span>
        </button>
      {/if}
    </div>
  </header>

  <div class="preview-body">
    {#if loading}
      <div class="preview-skeleton" aria-label="Loading preview">
        {#each Array(6) as _, index (index)}
          <span style={`width: ${92 - index * 9}%`}></span>
        {/each}
      </div>
    {:else if unsupported}
      <div class="preview-unsupported" data-testid="file-preview-unsupported">
        <svg
          width="28"
          height="28"
          viewBox="0 0 24 24"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
          aria-hidden="true"
        >
          <path
            d="M6 3h8l4 4v14a0 0 0 0 1 0 0H6a0 0 0 0 1 0 0V3z"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linejoin="round"
          />
          <path
            d="M14 3v4h4"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linejoin="round"
          />
        </svg>
        <strong>Can&rsquo;t preview this file</strong>
        <span>It&rsquo;s binary or too large to show here. Use the actions above to open it.</span>
      </div>
    {:else if content !== null && isMarkdown}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      <article class="markdown-body" data-testid="file-preview-markdown">{@html markdownHtml}</article>
    {:else if content !== null}
      <pre class="mono" data-testid="file-preview-monospace">{content}</pre>
    {/if}
  </div>
</section>

<style>
  .preview-pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
  }

  .preview-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-width: 0;
    padding: 11px 13px;
    border-bottom: 1px solid var(--border);
  }

  .preview-name {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .preview-actions {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: var(--space-2);
  }

  .reveal-btn {
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
    font-size: var(--text-base);
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease,
      border-color 140ms ease;
  }

  .reveal-btn:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    color: var(--fg);
  }

  .reveal-btn:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .reveal-btn.error {
    color: var(--amber);
    opacity: 0.9;
  }

  .reveal-label {
    line-height: 1;
  }

  .preview-body {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    padding: 14px 13px;
    overflow: auto;
  }

  .preview-skeleton {
    display: grid;
    gap: 10px;
  }

  .preview-skeleton span {
    height: 16px;
    border-radius: 5px;
    background: linear-gradient(
      90deg,
      var(--v4-control-faint),
      var(--v4-hairline),
      var(--v4-control-faint)
    );
    background-size: 200% 100%;
    animation: preview-skeleton 1.2s ease-in-out infinite;
  }

  .preview-unsupported {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: 36px 16px;
    text-align: center;
    color: var(--muted);
  }

  .preview-unsupported svg {
    color: var(--muted-3);
  }

  .preview-unsupported strong {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
  }

  .preview-unsupported span {
    max-width: 320px;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .mono {
    margin: 0;
    color: var(--muted);
    font-family: var(--font-mono);
    font-size: var(--text-base);
    line-height: 1.55;
    white-space: pre;
    overflow-wrap: normal;
  }

  /* ---- markdown typography (mirrors LibraryDetailPanel .markdown-body) ----- */
  .markdown-body {
    color: var(--fg);
    font-size: var(--text-base);
    line-height: 1.6;
  }

  .markdown-body :global(h1),
  .markdown-body :global(h2),
  .markdown-body :global(h3),
  .markdown-body :global(h4),
  .markdown-body :global(h5),
  .markdown-body :global(h6) {
    margin: var(--space-5) 0 var(--space-2);
    color: var(--fg);
    font-weight: 600;
    line-height: 1.3;
  }

  .markdown-body :global(h1) {
    font-size: var(--text-base);
  }
  .markdown-body :global(h2) {
    padding-bottom: var(--space-1);
    border-bottom: 1px solid var(--border);
    font-size: var(--text-base);
  }
  .markdown-body :global(h3) {
    font-size: var(--text-base);
  }

  .markdown-body :global(p) {
    margin: var(--space-2) 0;
    color: var(--muted);
  }

  .markdown-body :global(ul),
  .markdown-body :global(ol) {
    margin: var(--space-2) 0;
    padding-left: var(--space-5);
    color: var(--muted);
  }

  .markdown-body :global(li) {
    margin: var(--space-1) 0;
  }

  .markdown-body :global(a) {
    color: var(--blue);
    text-decoration: none;
  }

  .markdown-body :global(a:hover) {
    text-decoration: underline;
  }

  .markdown-body :global(code) {
    padding: 1px var(--space-1);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: var(--text-base);
  }

  .markdown-body :global(pre) {
    margin: var(--space-3) 0;
    padding: var(--space-3);
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-subtle);
  }

  .markdown-body :global(pre code) {
    padding: 0;
    background: transparent;
  }

  .markdown-body :global(blockquote) {
    margin: var(--space-3) 0;
    padding: var(--space-1) var(--space-3);
    border-left: 3px solid var(--border-strong);
    color: var(--muted-3);
  }

  .markdown-body :global(hr) {
    margin: var(--space-4) 0;
    border: 0;
    border-top: 1px solid var(--border);
  }

  .markdown-body :global(strong) {
    color: var(--fg);
    font-weight: 600;
  }

  @keyframes preview-skeleton {
    from {
      background-position: 0 0;
    }
    to {
      background-position: -200% 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .preview-skeleton span {
      animation: none;
    }
  }
</style>
