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
    await command.action();
    onclose();
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
      {#if filteredCommands.length > 0}
        {#each filteredCommands as command, index (command.id)}
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
    background: rgba(24, 24, 27, 0.2);
  }

  .command-palette {
    width: min(560px, 100%);
    overflow: hidden;
    border: 1px solid rgba(24, 24, 27, 0.14);
    border-radius: 8px;
    background: #ffffff;
    box-shadow: 0 22px 60px rgba(24, 24, 27, 0.2);
    color: #18181b;
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
    border-bottom: 1px solid #e4e4e7;
    background: #fafafa;
  }

  .command-glyph,
  .command-palette kbd {
    border: 1px solid #d4d4d8;
    border-radius: 5px;
    background: #f4f4f5;
    color: #52525b;
    font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
    font-size: 11px;
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
    color: #18181b;
    font: inherit;
    font-size: 14px;
  }

  .command-palette input::placeholder {
    color: #a1a1aa;
  }

  .command-list {
    max-height: min(360px, calc(100vh - 160px));
    overflow-y: auto;
    padding: 6px;
    scrollbar-color: #d4d4d8 transparent;
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
    color: #3f3f46;
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
    background: #e8f0ff;
    color: #18181b;
    outline: 1px solid #93c5fd;
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
    font-size: 13px;
    font-weight: 650;
  }

  .command-copy span {
    color: #71717a;
    font-size: 12px;
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
    border-color: #bfdbfe;
    background: #eff6ff;
    color: #1d4ed8;
  }

  .command-empty {
    display: flex;
    align-items: center;
    padding: 0 10px;
    color: #71717a;
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
      background: rgba(24, 24, 27, 0);
    }
  }

  @keyframes command-palette-in {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.985);
    }
  }
</style>
