<script lang="ts">
  /**
   * StoryDetailPanel — right-side story detail slide-over (US-008).
   *
   * Ported from hq-desktop's story-detail-panel.tsx (slide-over layout, backdrop,
   * sections, AC checklist with progress, dependency chips, the agent-activity
   * section), restyled to the HQ Sync unified desktop token set (monochrome glass;
   * no hardcoded hex).
   *
   * Renders nothing when `story` is null. Slides in from the right behind a dark
   * translucent backdrop. Closes on Escape, backdrop click, and the X button.
   *
   * Agent activity degrades gracefully: this app has no orchestrator live-signal
   * wired in, so the section renders a calm "No active run" empty state rather
   * than faking activity. The seam for later wiring is the optional `activity`
   * prop (see AgentActivity below) — when a future story plumbs an orchestrator
   * feed (e.g. hq-desktop's useStoryActivity / get_story_activity command), pass
   * it in and the empty state is replaced with the live render.
   */
  import { invoke } from '@tauri-apps/api/core';
  import { labelColor, type Story } from '../lib/projects-model';
  import LabelChip from './LabelChip.svelte';
  import OpenFileInClaudeCode from './OpenFileInClaudeCode.svelte';

  /**
   * The US-004 Story type carries no `notes` / `files` / `model_hint` fields, but
   * hq-desktop's detail panel renders them when present. We defensively augment
   * the type so those sections appear only when upstream data carries them,
   * without forcing the canonical Story type to grow the fields.
   */
  type StoryWithExtras = Story & {
    notes?: string | null;
    files?: string[] | null;
    model_hint?: string | null;
  };

  /**
   * Agent-activity seam. This app has no orchestrator live-signal, so the prop is
   * optional and defaults to null → the calm "No active run" empty state. A later
   * story can fetch from the orchestrator and pass a populated object here.
   *
   * TODO(US-future): wire an orchestrator feed (mirrors hq-desktop's
   * use-story-activity hook + get_story_activity Tauri command) and pass it as
   * `activity` so the live phase pipeline / subagent cards render instead of the
   * empty state.
   */
  interface AgentActivity {
    /** True when there is an in-flight run for this story. */
    active: boolean;
    /** Optional human label for the running phase (e.g. "implementation"). */
    phase?: string | null;
  }

  interface Props {
    /** The story to display. When null, the panel renders nothing. */
    story: StoryWithExtras | null;
    /** Called when the panel should close (Escape / backdrop / X). */
    onclose: () => void;
    /** Called when a dependency chip is clicked, with the dep story id. */
    onselectDependency?: (storyId: string) => void;
    /**
     * Optional live agent-activity signal. Absent/null → graceful empty state.
     * This is the seam for later orchestrator wiring (see file header TODO).
     */
    activity?: AgentActivity | null;
  }

  let { story, onclose, onselectDependency, activity = null }: Props = $props();

  // HQ root for the Claude Code session (US-012). Loaded lazily via get_config —
  // the same command App.svelte uses; Tauri caches the read. Empty until loaded,
  // at which point each file's "Open in Claude Code" affordance suppresses
  // itself (see OpenFileInClaudeCode). Best-effort: a failure leaves it empty
  // and the per-file affordances simply don't render.
  let hqFolderPath = $state('');

  $effect(() => {
    let cancelled = false;
    void invoke<{ hqFolderPath?: string }>('get_config')
      .then((config) => {
        if (!cancelled) hqFolderPath = config?.hqFolderPath ?? '';
      })
      .catch((err) => {
        console.error('StoryDetailPanel get_config failed:', err);
      });
    return () => {
      cancelled = true;
    };
  });

  const acItems = $derived(story?.acceptanceCriteria ?? []);
  const acTotal = $derived(acItems.length);
  // AC progress: the Story type carries no per-AC done flags, only a story-level
  // `passes` (same model as StoryCard). Completed stories read full; everything
  // else reads 0/total.
  const acComplete = $derived(story?.passes ? acTotal : 0);
  const acPercent = $derived(acTotal > 0 ? (acComplete / acTotal) * 100 : 0);

  const deps = $derived(story?.dependsOn ?? []);
  const labels = $derived(story?.labels ?? []);
  const files = $derived(story?.files ?? []);
  const priorityLabel = $derived(
    typeof story?.priority === 'number' ? `P${story.priority}` : null,
  );

  // Activity is "live" only when an orchestrator signal is wired AND active. With
  // no signal (the default in this app), we always show the calm empty state.
  const activityLive = $derived(activity?.active === true);

  // Status pill derived from the story-level `passes` flag (same model as the
  // AC progress). Completed → "Complete"; an active run → "In progress"; else
  // "To do". Kept deliberately simple — the Story type carries no richer state.
  const statusLabel = $derived(
    story?.passes ? 'Complete' : activityLive ? 'In progress' : 'To do',
  );
  const statusTone = $derived(
    story?.passes ? 'complete' : activityLive ? 'active' : 'todo',
  );

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onclose();
    }
  }

  function selectDependency(depId: string): void {
    onselectDependency?.(depId);
  }
</script>

<svelte:window onkeydown={story ? handleKeydown : undefined} />

{#if story}
  <!-- Backdrop — dark translucent scrim; click closes the panel. -->
  <div
    class="detail-backdrop"
    data-testid="story-detail-backdrop"
    onclick={onclose}
    aria-hidden="true"
  ></div>

  <div
    class="detail-panel"
    role="dialog"
    aria-modal="true"
    aria-label={`Story ${story.id}: ${story.title}`}
    data-testid="story-detail-panel"
  >
    <header class="detail-header">
      <div class="header-text">
        <span class="story-id">{story.id}</span>
        <h2 class="story-title">{story.title}</h2>

        <!-- Status line: status pill + inline agent-activity + priority. -->
        <div class="status-line">
          <span class="state-badge tone-{statusTone}">{statusLabel}</span>
          {#if priorityLabel}
            <span class="priority-badge" data-priority={priorityLabel}>{priorityLabel}</span>
          {/if}
          {#if activityLive}
            <span class="activity-inline is-active" data-testid="agent-activity-live">
              <span class="activity-dot"></span>
              <span>running{activity?.phase ? ` ${activity.phase}` : ''}</span>
            </span>
          {:else}
            <span class="activity-inline" data-testid="agent-activity-empty">
              <span aria-hidden="true">·</span>
              <span>no active run</span>
            </span>
          {/if}
        </div>
      </div>
      <button
        type="button"
        class="close-button"
        data-testid="story-detail-close"
        aria-label="Close story details"
        onclick={onclose}
      >
        <span aria-hidden="true">×</span>
      </button>
    </header>

    <div class="detail-body">
      {#if story.description}
        <section class="detail-section">
          <h3 class="section-title">Description</h3>
          <p class="section-body">{story.description}</p>
        </section>
      {/if}

      <!-- Inspector metadata — only rows whose data exists are rendered. -->
      {#if priorityLabel || deps.length > 0 || labels.length > 0}
        <section class="detail-section" aria-label="Details">
          <dl class="meta-grid">
            {#if priorityLabel}
              <div class="meta-row">
                <dt class="meta-key">Priority</dt>
                <dd class="meta-val">{priorityLabel}</dd>
              </div>
            {/if}
            {#if deps.length > 0}
              <div class="meta-row">
                <dt class="meta-key">Dependencies</dt>
                <dd class="meta-val">{deps.length}</dd>
              </div>
            {/if}
            {#if labels.length > 0}
              <div class="meta-row">
                <dt class="meta-key">Labels</dt>
                <dd class="meta-val">{labels.length}</dd>
              </div>
            {/if}
          </dl>
        </section>
      {/if}

      {#if acTotal > 0}
        <section class="detail-section">
          <h3 class="section-title">Acceptance Criteria</h3>
          <div class="ac-progress">
            <div
              class="progress-track"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={acTotal}
              aria-valuenow={acComplete}
              aria-label="Acceptance criteria complete"
            >
              <div
                class="progress-fill"
                style={`--progress-scale: ${Math.max(0, Math.min(1, acPercent / 100))}`}
              ></div>
            </div>
            <span class="ac-count" data-testid="ac-progress-count"
              >{acComplete}/{acTotal} criteria</span
            >
          </div>
          <ul class="ac-list" data-testid="ac-checklist">
            {#each acItems as criterion, index (index)}
              <li class="ac-item" class:is-done={story.passes}>
                <span class="ac-mark" aria-hidden="true">
                  {#if story.passes}
                    <svg viewBox="0 0 16 16" width="14" height="14" fill="none">
                      <circle cx="8" cy="8" r="7" fill="currentColor" opacity="0.16" />
                      <path
                        d="M4.5 8.2 7 10.5l4.5-5"
                        stroke="currentColor"
                        stroke-width="1.6"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </svg>
                  {:else}
                    <svg viewBox="0 0 16 16" width="14" height="14" fill="none">
                      <circle
                        cx="8"
                        cy="8"
                        r="6.5"
                        stroke="currentColor"
                        stroke-width="1.4"
                      />
                    </svg>
                  {/if}
                </span>
                <span class="ac-text">{criterion}</span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if deps.length > 0}
        <section class="detail-section">
          <h3 class="section-title">Dependencies</h3>
          <div class="chip-row" data-testid="dependency-chips">
            {#each deps as depId (depId)}
              <button
                type="button"
                class="dep-chip"
                data-testid="dependency-chip"
                onclick={() => selectDependency(depId)}
                title={`Open ${depId}`}
              >
                {depId}
              </button>
            {/each}
          </div>
        </section>
      {/if}

      {#if labels.length > 0}
        <section class="detail-section">
          <h3 class="section-title">Labels</h3>
          <div class="chip-row">
            {#each labels as label (label)}
              <LabelChip {label} />
            {/each}
          </div>
        </section>
      {/if}

      {#if story.notes}
        <section class="detail-section">
          <h3 class="section-title">Notes</h3>
          <p class="section-body">{story.notes}</p>
        </section>
      {/if}

      {#if files.length > 0}
        <section class="detail-section">
          <h3 class="section-title">Files</h3>
          <ul class="file-list" data-testid="story-files">
            {#each files as file (file)}
              <li class="file-item">
                <span class="file-path">{file}</span>
                <OpenFileInClaudeCode {file} folder={hqFolderPath} variant="compact" />
              </li>
            {/each}
          </ul>
        </section>
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
    max-width: 92vw;
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

  .story-id {
    color: var(--muted-3);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 500;
    letter-spacing: 0.02em;
  }

  .story-title {
    margin: var(--space-2) 0 0;
    color: var(--fg);
    font-family: var(--font-display);
    font-size: 19px;
    font-weight: 600;
    letter-spacing: -0.01em;
    line-height: 1.2;
  }

  /* Status line — pill + inline activity + priority on one row. */
  .status-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .state-badge,
  .priority-badge {
    display: inline-flex;
    align-items: center;
    padding: 2px 9px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-raise);
    color: var(--muted-2);
    font-size: var(--text-sm);
    font-weight: 500;
    line-height: 16px;
  }

  .state-badge.tone-complete {
    color: var(--emerald);
  }

  .state-badge.tone-active {
    color: var(--amber);
  }

  .state-badge.tone-todo {
    color: var(--muted-2);
  }

  /* Inline agent-activity — quiet text folded into the status line. */
  .activity-inline {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--muted);
    font-size: var(--text-sm);
  }

  .activity-inline.is-active {
    color: var(--emerald);
  }

  .priority-badge {
    font-variant-numeric: tabular-nums;
  }

  /* Color-coded priority (hq-desktop parity): P1 red · P2 amber · P3 blue. */
  .priority-badge[data-priority='P1'] {
    border-color: transparent;
    background: rgba(248, 113, 113, 0.15);
    color: var(--red);
  }
  .priority-badge[data-priority='P2'] {
    border-color: transparent;
    background: rgba(245, 158, 11, 0.15);
    color: var(--amber);
  }
  .priority-badge[data-priority='P3'] {
    border-color: transparent;
    background: rgba(96, 165, 250, 0.15);
    color: var(--blue);
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

  .detail-section {
    min-width: 0;
  }

  .section-title {
    margin: 0 0 var(--space-2);
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .section-body {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-sm);
    line-height: 1.5;
  }

  /* Inspector metadata grid — label → value rows. */
  .meta-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-raise);
  }

  .meta-row {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
  }

  .meta-key {
    flex-shrink: 0;
    width: 96px;
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .meta-val {
    margin: 0;
    color: var(--fg-data);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }

  /* Agent-activity dot — only the live inline render uses it now. */
  .activity-dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--emerald);
    animation: pulse 1.6s ease-in-out infinite;
  }

  /* AC progress — mirrors the StoryCard progress-bar visual language. */
  .ac-progress {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }

  .progress-track {
    flex: 1;
    height: 5px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-active);
  }

  .progress-fill {
    width: 100%;
    height: 100%;
    border-radius: inherit;
    background: var(--emerald);
    transform: scaleX(var(--progress-scale, 0));
    transform-origin: left center;
    transition: transform 180ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  .ac-count {
    flex-shrink: 0;
    color: var(--muted-3);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }

  .ac-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ac-item {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .ac-mark {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    height: 21px;
    color: var(--muted-3);
  }

  .ac-item.is-done .ac-mark {
    color: var(--emerald);
  }

  .ac-text {
    color: var(--muted-2);
    font-size: var(--text-sm);
    line-height: 1.5;
  }

  .ac-item.is-done .ac-text {
    color: var(--muted);
  }

  .chip-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .dep-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-raise);
    color: var(--fg-data);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease,
      border-color 140ms ease;
  }

  .dep-chip:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    color: var(--fg);
  }

  .dep-chip:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .file-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .file-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: 2px 4px;
    border-radius: var(--radius-sm);
    transition: background 140ms ease;
  }

  .file-item:hover {
    background: var(--row-hover);
  }

  .file-path {
    flex: 1 1 auto;
    min-width: 0;
    color: var(--muted);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.45;
    word-break: break-all;
  }

  /* Compact affordance reveals on row hover / keyboard focus — matches the
     drill-in language used by the board + deployments rows. */
  .file-item :global(.open-claude-btn.compact) {
    opacity: 0;
    transition: opacity 140ms ease;
  }

  .file-item:hover :global(.open-claude-btn.compact),
  .file-item :global(.open-claude-btn.compact:focus-visible) {
    opacity: 1;
  }

  @media (prefers-reduced-motion: reduce) {
    .file-item :global(.open-claude-btn.compact) {
      transition: none;
    }
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

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .detail-backdrop,
    .detail-panel {
      animation: none;
    }

    .progress-fill {
      transition: none;
    }

    .activity-dot {
      animation: none;
    }
  }
</style>
