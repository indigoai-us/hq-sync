<script lang="ts">
  /**
   * LibraryDetailPanel — right-side slide-over for a library item (worker or
   * skill). Structurally mirrors StoryDetailPanel (backdrop, Escape/backdrop/X
   * close, .detail-panel layout). On open it lazily loads the full detail:
   *
   *   * worker → loadWorkerDetail(path): name, type/team chips, description, a
   *     Skills section (name + optional description), and Instructions rendered
   *     as markdown.
   *   * skill  → loadSkillDetail(path): name, description, Allowed Tools chips,
   *     and the SKILL.md body rendered as markdown.
   *
   * Markdown is rendered by the dependency-free, CSP-safe lib/markdown.ts helper
   * (same as ProjectDetailView) — no `marked`, no DOM sanitizer.
   */
  import {
    loadSkillDetail,
    loadWorkerDetail,
    type LibraryItem,
    type SkillDetail,
    type WorkerDetail,
  } from '../lib/library';
  import { renderMarkdown } from '../lib/markdown';
  import LabelChip from './LabelChip.svelte';

  interface Props {
    /** The item to display. When null, the panel renders nothing. */
    item: LibraryItem | null;
    /** Called when the panel should close (Escape / backdrop / X). */
    onclose: () => void;
  }

  let { item, onclose }: Props = $props();

  let workerDetail = $state<WorkerDetail | null>(null);
  let skillDetail = $state<SkillDetail | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  const title = $derived(
    item === null ? '' : item.kind === 'worker' ? item.worker.name : item.skill.name,
  );
  const kindLabel = $derived(item?.kind === 'worker' ? 'Worker' : 'Skill');

  // Load detail whenever the open item changes. Cancel-flag guards against an
  // out-of-order completion when the user clicks through items quickly.
  $effect(() => {
    const current = item;
    workerDetail = null;
    skillDetail = null;
    error = null;

    if (!current) {
      loading = false;
      return;
    }

    loading = true;
    let cancelled = false;

    void (async () => {
      try {
        if (current.kind === 'worker') {
          const detail = await loadWorkerDetail(current.worker.path);
          if (!cancelled) workerDetail = detail;
        } else {
          const detail = await loadSkillDetail(current.skill.path);
          if (!cancelled) skillDetail = detail;
        }
      } catch (err) {
        console.error('LibraryDetailPanel load failed:', err);
        if (!cancelled) error = 'Could not load details.';
      } finally {
        if (!cancelled) loading = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  const workerInstructionsHtml = $derived(
    workerDetail && workerDetail.instructions.trim() !== ''
      ? renderMarkdown(workerDetail.instructions)
      : '',
  );
  const skillBodyHtml = $derived(
    skillDetail && skillDetail.body.trim() !== '' ? renderMarkdown(skillDetail.body) : '',
  );

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onclose();
    }
  }
</script>

<svelte:window onkeydown={item ? handleKeydown : undefined} />

{#if item}
  <div
    class="detail-backdrop"
    data-testid="library-detail-backdrop"
    onclick={onclose}
    aria-hidden="true"
  ></div>

  <div
    class="detail-panel"
    role="dialog"
    aria-modal="true"
    aria-label={`${kindLabel}: ${title}`}
    data-testid="library-detail-panel"
  >
    <header class="detail-header">
      <div class="header-text">
        <span class="kind-tag">{kindLabel}</span>
        <h2 class="detail-title">{title}</h2>
        <div class="badges">
          {#if item.kind === 'worker'}
            {#if item.worker.type}
              <LabelChip label={item.worker.type} />
            {/if}
            {#if item.worker.team}
              <LabelChip label={item.worker.team} />
            {/if}
            <span class="scope-badge">
              {item.worker.scope === 'company' ? (item.worker.company ?? 'company') : 'shared'}
            </span>
          {:else}
            <span class="scope-badge">
              {item.skill.scope === 'company' ? (item.skill.company ?? 'company') : item.skill.scope}
            </span>
          {/if}
        </div>
      </div>
      <button
        type="button"
        class="close-button"
        data-testid="library-detail-close"
        aria-label="Close details"
        onclick={onclose}
      >
        <span aria-hidden="true">×</span>
      </button>
    </header>

    <div class="detail-body">
      {#if error}
        <div class="detail-error" role="alert">{error}</div>
      {/if}

      {#if item.kind === 'worker'}
        {#if item.worker.description}
          <section class="detail-section">
            <h3 class="section-title">Description</h3>
            <p class="section-body">{item.worker.description}</p>
          </section>
        {/if}

        {#if loading}
          <p class="muted-note">Loading…</p>
        {:else if workerDetail}
          {#if workerDetail.skills.length > 0}
            <section class="detail-section">
              <h3 class="section-title">Skills</h3>
              <ul class="skill-list">
                {#each workerDetail.skills as skill (skill.name)}
                  <li class="skill-item">
                    <span class="skill-name">{skill.name}</span>
                    {#if skill.description}
                      <span class="skill-desc">{skill.description}</span>
                    {/if}
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          {#if workerInstructionsHtml}
            <section class="detail-section">
              <h3 class="section-title">Instructions</h3>
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              <article class="markdown-body" data-testid="worker-instructions">{@html workerInstructionsHtml}</article>
            </section>
          {/if}
        {/if}
      {:else}
        {#if item.skill.description}
          <section class="detail-section">
            <h3 class="section-title">Description</h3>
            <p class="section-body">{item.skill.description}</p>
          </section>
        {/if}

        {#if item.skill.allowedTools.length > 0}
          <section class="detail-section">
            <h3 class="section-title">Allowed Tools</h3>
            <div class="chip-row">
              {#each item.skill.allowedTools as tool (tool)}
                <span class="tool-chip">{tool}</span>
              {/each}
            </div>
          </section>
        {/if}

        {#if loading}
          <p class="muted-note">Loading…</p>
        {:else if skillBodyHtml}
          <section class="detail-section">
            <h3 class="section-title">Details</h3>
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            <article class="markdown-body" data-testid="skill-body">{@html skillBodyHtml}</article>
          </section>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .detail-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(0, 0, 0, 0.45);
    animation: backdrop-fade 160ms ease;
  }

  .detail-panel {
    position: fixed;
    inset-block: 0;
    inset-inline-end: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    width: 520px;
    max-width: 94vw;
    border-left: 1px solid var(--border);
    background: var(--bg);
    box-shadow: -8px 0 32px rgba(0, 0, 0, 0.45);
    animation: panel-slide-in 200ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  .detail-header {
    display: flex;
    flex-shrink: 0;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-5) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  .header-text {
    min-width: 0;
  }

  .kind-tag {
    color: var(--muted);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .detail-title {
    margin: var(--space-1) 0 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 680;
    line-height: 22px;
    overflow-wrap: anywhere;
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-top: var(--space-2);
  }

  .scope-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-3);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 16px;
    text-transform: lowercase;
  }

  .close-button {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .close-button:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  .close-button:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .detail-body {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: var(--space-5);
    min-height: 0;
    padding: var(--space-5);
    overflow-y: auto;
  }

  .detail-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-base);
  }

  .detail-section {
    min-width: 0;
  }

  .section-title {
    margin: 0 0 var(--space-2);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .section-body {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
    line-height: 19px;
    overflow-wrap: anywhere;
  }

  .muted-note {
    margin: 0;
    color: var(--muted-3);
    font-size: var(--text-base);
  }

  .skill-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .skill-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
  }

  .skill-name {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 650;
  }

  .skill-desc {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  .chip-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .tool-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-2);
    font-family:
      ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-base);
    font-weight: 600;
  }

  /* ---- markdown typography (mirrors ProjectDetailView .markdown-body) ----- */
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
    font-weight: 680;
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
    font-family:
      ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: 0.92em;
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
    font-weight: 680;
  }

  @keyframes backdrop-fade {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes panel-slide-in {
    from {
      transform: translateX(16px);
      opacity: 0;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .detail-backdrop,
    .detail-panel {
      animation: none;
    }
  }
</style>
