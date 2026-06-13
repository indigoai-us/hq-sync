<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { buildClaudeCodeUrl } from '../../lib/claude-code-link';
  import { setStoryPasses } from '../lib/projects-store.svelte';
  import {
    projectDisplayName,
    type Project,
    type Story,
  } from '../lib/projects-model';
  import './tokens.css';

  interface Props {
    story: Story | null;
    project: Project | null;
    prdPath: string;
    onclose: () => void;
    onselectDependency?: (storyId: string) => void;
    onStoryPassesChange?: (storyId: string, passes: boolean) => void;
  }

  let {
    story,
    project,
    prdPath,
    onclose,
    onselectDependency,
    onStoryPassesChange,
  }: Props = $props();

  let passesOverride = $state<boolean | null>(null);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let footerBusy = $state<'prd' | 'run' | null>(null);
  let footerMessage = $state<string | null>(null);

  $effect(() => {
    void story?.id;
    void story?.passes;
    passesOverride = null;
    error = null;
    footerBusy = null;
    footerMessage = null;
    saving = false;
  });

  const currentPasses = $derived(passesOverride ?? story?.passes ?? false);
  const acItems = $derived(story?.acceptanceCriteria ?? []);
  const acComplete = $derived(currentPasses ? acItems.length : 0);
  const progress = $derived(acItems.length > 0 ? (acComplete / acItems.length) * 100 : 0);
  const priority = $derived(typeof story?.priority === 'number' ? `P${story.priority}` : 'P-');
  const hierarchy = $derived(
    project && story ? `${projectDisplayName(project)} / ${story.id}` : '',
  );

  async function setPasses(next: boolean) {
    if (!story || saving || next === currentPasses) return;
    const previous = currentPasses;
    passesOverride = next;
    saving = true;
    error = null;
    const result = await setStoryPasses(prdPath, story.id, previous, next);
    saving = false;
    if (result.ok) {
      passesOverride = result.passes;
      onStoryPassesChange?.(story.id, result.passes);
    } else {
      passesOverride = previous;
      error = result.error;
    }
  }

  function toggleCriterion() {
    void setPasses(!currentPasses);
  }

  async function openPrd() {
    if (!prdPath || footerBusy) return;
    footerBusy = 'prd';
    footerMessage = null;
    try {
      await invoke('open_in_editor', { path: prdPath });
    } catch (err) {
      console.error('open_in_editor failed:', err);
      footerMessage = 'Could not open the PRD file.';
    } finally {
      footerBusy = null;
    }
  }

  async function runStory() {
    if (!story || footerBusy) return;
    footerBusy = 'run';
    footerMessage = null;
    const projectName = project ? projectDisplayName(project) : 'the current project';
    const prompt = [
      `/run-project ${story.id}`,
      '',
      `Run story ${story.id}: ${story.title}`,
      `Project: ${projectName}`,
      prdPath ? `PRD: ${prdPath}` : null,
      story.description ? `Description: ${story.description}` : null,
      acItems.length > 0
        ? ['Acceptance criteria:', ...acItems.map((item) => `- ${item}`)].join('\n')
        : null,
      '',
      'Execute this story through the normal HQ project workflow, update the PRD/story state when done, and run the relevant checks before reporting back.',
    ]
      .filter((line): line is string => Boolean(line))
      .join('\n');

    try {
      const config: { hqFolderPath?: string } = await invoke<{ hqFolderPath?: string }>(
        'get_config',
      ).catch(() => ({}));
      const url = buildClaudeCodeUrl({ folder: config.hqFolderPath ?? '', prompt });
      await invoke('open_claude_code_link', { url });
      footerMessage = 'Story opened in Claude Code.';
    } catch (err) {
      console.error('open_claude_code_link failed:', err);
      try {
        await navigator.clipboard.writeText(prompt);
        footerMessage = 'Story prompt copied.';
      } catch {
        footerMessage = 'Could not open Claude Code.';
      }
    } finally {
      footerBusy = null;
    }
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onclose();
    }
  }
</script>

<svelte:window onkeydown={story ? handleKeydown : undefined} />

{#if story}
  <div class="story-backdrop" aria-hidden="true" onclick={onclose}></div>
  <aside class="story-panel" aria-label={`Story ${story.id}`} data-testid="v4-story-panel">
    <header class="panel-header">
      <div class="header-copy">
        <span class="hierarchy">{hierarchy}</span>
        <h2>{story.title}</h2>
        <div class="meta-row">
          <span class="priority">{priority}</span>
          {#each story.labels as label (label)}
            <span class="label-chip">{label}</span>
          {/each}
        </div>
      </div>
      <button type="button" class="icon-button" aria-label="Close story" onclick={onclose}>×</button>
    </header>

    <div class="status-control" aria-label="Story status">
      <button type="button" class:active={!currentPasses} disabled={saving} onclick={() => setPasses(false)}>
        To do
      </button>
      <button type="button" class:active={currentPasses} disabled={saving} onclick={() => setPasses(true)}>
        Done
      </button>
    </div>

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <div class="panel-body">
      <section class="section">
        <h3>Hierarchy</h3>
        <p>{project ? projectDisplayName(project) : 'Project'} -> {story.id}</p>
      </section>

      {#if story.description}
        <section class="section">
          <h3>Description</h3>
          <p>{story.description}</p>
        </section>
      {/if}

      <section class="section">
        <div class="section-title-row">
          <h3>Acceptance criteria</h3>
          <span>{acComplete}/{acItems.length}</span>
        </div>
        <div class="progress-track" aria-hidden="true">
          <span style={`width: ${progress}%`}></span>
        </div>
        <ul class="ac-list">
          {#each acItems as item, index (index)}
            <li>
              <button
                type="button"
                class:checked={currentPasses}
                disabled={saving}
                aria-pressed={currentPasses}
                onclick={toggleCriterion}
              >
                <span aria-hidden="true">{currentPasses ? '✓' : ''}</span>
              </button>
              <p>{item}</p>
            </li>
          {/each}
        </ul>
      </section>

      {#if story.dependsOn.length > 0}
        <section class="section">
          <h3>Depends on</h3>
          <div class="chip-row">
            {#each story.dependsOn as dep (dep)}
              <button type="button" class="dep-chip" onclick={() => onselectDependency?.(dep)}>
                {dep}
              </button>
            {/each}
          </div>
        </section>
      {/if}

      {#if story.files && story.files.length > 0}
        <section class="section">
          <h3>Files</h3>
          <ul class="file-list">
            {#each story.files as file (file)}
              <li>{file}</li>
            {/each}
          </ul>
        </section>
      {/if}
    </div>

    <footer class="panel-footer">
      <div class="footer-status" role="status">{footerMessage ?? ''}</div>
      <button type="button" onclick={() => void openPrd()} disabled={footerBusy !== null || !prdPath}>
        {footerBusy === 'prd' ? 'Opening…' : 'Open PRD'}
      </button>
      <button type="button" class="primary" onclick={() => void runStory()} disabled={footerBusy !== null}>
        {footerBusy === 'run' ? 'Opening…' : 'Run story'}
      </button>
    </footer>
  </aside>
{/if}

<style>
  .story-backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
    background: rgba(0, 0, 0, 0.36);
  }

  .story-panel {
    position: fixed;
    inset-block: 0;
    inset-inline-end: 0;
    z-index: 100;
    display: flex;
    width: min(420px, 100vw);
    flex-direction: column;
    border-left: 1px solid var(--v4-hairline);
    background: var(--v4-raised);
    color: var(--v4-text-1);
    box-shadow: -20px 0 48px rgba(0, 0, 0, 0.32);
  }

  .panel-header,
  .panel-footer,
  .meta-row,
  .section-title-row,
  .chip-row {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .panel-header {
    justify-content: space-between;
    gap: 14px;
    padding: 18px;
    border-bottom: 1px solid var(--v4-rowline);
  }

  .header-copy {
    min-width: 0;
  }

  .hierarchy,
  .section h3,
  .section-title-row span {
    overflow: hidden;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  h2 {
    margin: 6px 0 10px;
    overflow-wrap: anywhere;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.3;
  }

  .meta-row {
    gap: 6px;
    flex-wrap: wrap;
  }

  .priority,
  .label-chip,
  .dep-chip {
    display: inline-flex;
    align-items: center;
    min-height: 22px;
    padding: 0 8px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-inset);
    color: var(--v4-text-2);
    font-size: var(--text-base);
  }

  .icon-button {
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-3);
    font: inherit;
    font-size: var(--text-base);
  }

  .icon-button:hover {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  .status-control {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    margin: 14px 18px 0;
    padding: 3px;
    border: 1px solid var(--v4-hairline);
    border-radius: 7px;
    background: var(--v4-inset);
  }

  .status-control button,
  .panel-footer button {
    height: 28px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--v4-text-3);
    font: inherit;
    font-size: var(--text-base);
  }

  .status-control button.active {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  .error {
    margin: 10px 18px 0;
    color: var(--v4-error);
    font-size: var(--text-base);
  }

  .panel-body {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: 18px;
    min-height: 0;
    padding: 18px;
    overflow-y: auto;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }

  .section h3,
  .section p {
    margin: 0;
  }

  .section p {
    color: var(--v4-text-2);
    font-size: var(--text-base);
    line-height: 1.45;
  }

  .section-title-row {
    justify-content: space-between;
    gap: 10px;
  }

  .progress-track {
    height: 4px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--v4-rowline);
  }

  .progress-track span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--v4-ok);
  }

  .ac-list,
  .file-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ac-list li {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    gap: 8px;
    align-items: start;
  }

  .ac-list button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: 1px solid var(--v4-hairline);
    border-radius: 5px;
    background: transparent;
    color: var(--v4-ok);
    font-size: var(--text-base);
  }

  .ac-list button.checked {
    border-color: color-mix(in srgb, var(--v4-ok) 45%, transparent);
    background: color-mix(in srgb, var(--v4-ok) 12%, transparent);
  }

  .chip-row {
    flex-wrap: wrap;
    gap: 6px;
  }

  .dep-chip {
    cursor: pointer;
  }

  .file-list li {
    overflow: hidden;
    color: var(--v4-text-2);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: var(--text-base);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .panel-footer {
    justify-content: flex-end;
    gap: 8px;
    padding: 14px 18px;
    border-top: 1px solid var(--v4-rowline);
  }

  .footer-status {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .panel-footer button {
    padding: 0 11px;
    border: 1px solid var(--v4-hairline);
    color: var(--v4-text-2);
  }

  .panel-footer button.primary {
    border-color: transparent;
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  .panel-footer button:disabled {
    opacity: 0.52;
  }

  @media (max-width: 520px) {
    .story-panel {
      width: 100vw;
      border-left: 0;
    }

    .panel-header {
      align-items: flex-start;
      padding: 16px;
    }

    .status-control {
      margin-inline: 16px;
    }

    .panel-body {
      padding: 16px;
    }

    .panel-footer {
      flex-wrap: wrap;
      justify-content: flex-start;
      padding: 12px 16px;
    }

    .footer-status {
      flex-basis: 100%;
      white-space: normal;
    }
  }
</style>
