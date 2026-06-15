<script lang="ts">
  import { tick } from 'svelte';

  export interface CommandPaletteItem {
    id: string;
    label: string;
    detail: string;
    shortcut?: string;
    action: () => void | Promise<void>;
  }

  interface Props {
    commands: CommandPaletteItem[];
    onclose: () => void;
  }

  interface CommandPaletteSection {
    id: 'actions' | 'navigate';
    label: 'ACTIONS' | 'NAVIGATE';
    items: CommandPaletteItem[];
  }

  let { commands, onclose }: Props = $props();
  let query = $state('');
  let highlightedIndex = $state(0);
  let inputEl: HTMLInputElement | null = $state(null);

  function fuzzyMatch(value: string, needle: string): boolean {
    const haystack = value.toLowerCase();
    const queryText = needle.trim().toLowerCase();
    if (!queryText) return true;

    let searchFrom = 0;
    for (const char of queryText) {
      const foundAt = haystack.indexOf(char, searchFrom);
      if (foundAt === -1) return false;
      searchFrom = foundAt + 1;
    }
    return true;
  }

  const filteredCommands = $derived(
    commands.filter((command) =>
      fuzzyMatch(`${command.label} ${command.detail} ${command.shortcut ?? ''}`, query),
    ),
  );

  function sectionId(command: CommandPaletteItem): CommandPaletteSection['id'] {
    return command.id.startsWith('command-go-') ? 'navigate' : 'actions';
  }

  const commandSections = $derived.by((): CommandPaletteSection[] => {
    const sections: CommandPaletteSection[] = [
      { id: 'actions', label: 'ACTIONS', items: [] },
      { id: 'navigate', label: 'NAVIGATE', items: [] },
    ];
    for (const command of filteredCommands) {
      const target = sections.find((section) => section.id === sectionId(command));
      target?.items.push(command);
    }
    return sections.filter((section) => section.items.length > 0);
  });

  $effect(() => {
    if (highlightedIndex >= filteredCommands.length) {
      highlightedIndex = Math.max(0, filteredCommands.length - 1);
    }
  });

  $effect(() => {
    query;
    highlightedIndex = 0;
  });

  $effect(() => {
    void tick().then(() => inputEl?.focus());
  });

  async function execute(command: CommandPaletteItem | undefined) {
    if (!command) return;
    // Always close the palette, even if the action throws — otherwise a single
    // failing command left the palette stuck open and modal over the whole app.
    try {
      await command.action();
    } catch (err) {
      console.error('command-palette: action failed', err);
    } finally {
      onclose();
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      onclose();
      return;
    }

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      highlightedIndex =
        filteredCommands.length === 0 ? 0 : (highlightedIndex + 1) % filteredCommands.length;
      return;
    }

    if (event.key === 'ArrowUp') {
      event.preventDefault();
      highlightedIndex =
        filteredCommands.length === 0
          ? 0
          : (highlightedIndex - 1 + filteredCommands.length) % filteredCommands.length;
      return;
    }

    if (event.key === 'Enter') {
      event.preventDefault();
      void execute(filteredCommands[highlightedIndex]);
    }
  }
</script>

<div class="command-backdrop" role="presentation" onclick={onclose}>
  <div
    class="command-palette"
    role="dialog"
    aria-modal="true"
    aria-labelledby="command-palette-title"
    tabindex="-1"
    onkeydown={handleKeydown}
    onclick={(event) => event.stopPropagation()}
  >
    <div class="command-input-row">
      <span class="command-glyph" aria-hidden="true">⌘K</span>
      <input
        bind:this={inputEl}
        bind:value={query}
        type="text"
        autocomplete="off"
        spellcheck="false"
        aria-label="Filter commands"
        aria-controls="command-palette-list"
        aria-activedescendant={filteredCommands[highlightedIndex]?.id}
        placeholder="Search commands"
      />
    </div>

    <h2 id="command-palette-title">Command palette</h2>

    <div id="command-palette-list" class="command-list" role="listbox" aria-label="Commands">
      {#if commandSections.length > 0}
        {#each commandSections as section (section.id)}
          <div class="command-section" role="presentation">
            <div class="command-section-title">{section.label}</div>
            {#each section.items as command (command.id)}
              {@const index = filteredCommands.indexOf(command)}
              <button
                id={command.id}
                class:highlighted={index === highlightedIndex}
                type="button"
                role="option"
                aria-selected={index === highlightedIndex}
                onfocus={() => {
                  highlightedIndex = index;
                }}
                onmouseenter={() => {
                  highlightedIndex = index;
                }}
                onclick={() => void execute(command)}
              >
                <span class="command-copy">
                  <strong>{command.label}</strong>
                  <span>{command.detail}</span>
                </span>
                {#if command.shortcut}
                  <kbd>{command.shortcut}</kbd>
                {/if}
              </button>
            {/each}
          </div>
        {/each}
      {:else}
        <div class="command-empty" role="status">No commands found</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .command-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 72px 20px 20px;
    background: rgba(0, 0, 0, 0.5);
  }

  .command-palette {
    width: min(560px, 100%);
    overflow: hidden;
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 22px 60px rgba(0, 0, 0, 0.55);
    color: var(--fg);
    transform-origin: top center;
  }

  .command-palette h2 {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .command-input-row {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 48px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-subtle);
  }

  .command-glyph,
  .command-palette kbd {
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--row-active);
    color: var(--muted-3);
    font-family: var(--font-mono);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .command-glyph {
    flex: 0 0 auto;
    padding: 0 6px;
  }

  .command-palette input {
    width: 100%;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
  }

  .command-palette input::placeholder {
    color: var(--muted-3);
  }

  .command-list {
    max-height: min(360px, calc(100vh - 160px));
    overflow-y: auto;
    padding: 6px;
    scrollbar-color: var(--scrollbar-thumb-hover) transparent;
  }

  .command-section + .command-section {
    margin-top: 6px;
    padding-top: 6px;
    border-top: 1px solid var(--border);
  }

  .command-section-title {
    padding: 5px 8px 4px;
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 600;
    line-height: 14px;
    text-transform: uppercase;
  }

  .command-list button,
  .command-empty {
    width: 100%;
    min-height: 46px;
    border-radius: 6px;
  }

  .command-list button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 7px 8px;
    border: 0;
    background: transparent;
    color: var(--muted-2);
    font: inherit;
    text-align: left;
    cursor: default;
    transition:
      background-color 120ms ease,
      color 120ms ease,
      outline-color 120ms ease,
      transform 120ms ease;
  }

  .command-list button.highlighted,
  .command-list button:focus-visible {
    background: rgba(96, 165, 250, 0.14);
    color: var(--fg);
    outline: 1px solid rgba(96, 165, 250, 0.5);
  }

  .command-list button.highlighted {
    transform: translateX(2px);
  }

  .command-copy {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 2px;
  }

  .command-copy strong,
  .command-copy span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .command-copy strong {
    color: currentColor;
    font-size: var(--text-base);
    font-weight: 600;
  }

  .command-copy span {
    color: var(--muted);
    font-size: var(--text-base);
  }

  .command-palette kbd {
    flex: 0 0 auto;
    min-width: 22px;
    padding: 0 5px;
    text-align: center;
    transition:
      border-color 120ms ease,
      background-color 120ms ease,
      color 120ms ease;
  }

  .command-list button.highlighted kbd {
    border-color: rgba(96, 165, 250, 0.4);
    background: rgba(96, 165, 250, 0.12);
    color: var(--blue);
  }

  .command-empty {
    display: flex;
    align-items: center;
    padding: 0 10px;
    color: var(--muted);
  }

  @media (prefers-reduced-motion: no-preference) {
    .command-backdrop {
      animation: command-backdrop-in 120ms ease-out;
    }

    .command-palette {
      animation: command-palette-in 150ms cubic-bezier(0.2, 0.8, 0.2, 1);
    }
  }

  @keyframes command-backdrop-in {
    from {
      background: rgba(0, 0, 0, 0);
    }
  }

  @keyframes command-palette-in {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.985);
    }
  }
</style>
