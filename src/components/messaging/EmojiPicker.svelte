<script lang="ts">
  // Compact emoji popover (US-025). Renders the CURATED ~24-emoji set as a grid
  // of tap targets. Deliberately NO heavy emoji-data dependency — the set is an
  // inline constant in lib/reactions.ts so the bundle stays under the <15MB
  // budget (tests/PERF.md). The frameless standalone window has no reliable
  // hover, so this is opened by an explicit tap on the add-reaction trigger and
  // closes on pick, Escape, or an outside click.
  import { CURATED_EMOJI } from '../../lib/reactions';

  interface Props {
    // Called with the chosen emoji. The parent (ReactionBar) toggles + closes.
    onpick: (emoji: string) => void;
    // Called when the popover should dismiss without a pick (Escape / outside
    // click).
    onclose: () => void;
  }

  let { onpick, onclose }: Props = $props();

  let rootEl = $state<HTMLDivElement | null>(null);

  // Outside-click + Escape dismissal. Registered while the popover is mounted;
  // the {#if} in ReactionBar unmounts this component to tear the listeners down.
  function onDocPointerDown(e: PointerEvent): void {
    if (rootEl && e.target instanceof Node && !rootEl.contains(e.target)) {
      onclose();
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
    }
  }

  $effect(() => {
    // Defer adding the pointer listener so the same tap that opened the picker
    // (still bubbling) doesn't immediately close it.
    const id = setTimeout(() => {
      document.addEventListener('pointerdown', onDocPointerDown, true);
    }, 0);
    document.addEventListener('keydown', onKeydown, true);
    // Focus the grid so keyboard users land inside and Escape works.
    rootEl?.focus();
    return () => {
      clearTimeout(id);
      document.removeEventListener('pointerdown', onDocPointerDown, true);
      document.removeEventListener('keydown', onKeydown, true);
    };
  });
</script>

<div
  class="emoji-picker"
  bind:this={rootEl}
  role="menu"
  tabindex="-1"
  aria-label="Add a reaction"
>
  {#each CURATED_EMOJI as emoji (emoji)}
    <button
      class="emoji-cell"
      type="button"
      role="menuitem"
      onclick={() => onpick(emoji)}
      aria-label={`React with ${emoji}`}
    >
      {emoji}
    </button>
  {/each}
</div>

<style>
  .emoji-picker {
    position: absolute;
    z-index: 30;
    bottom: calc(100% + 0.25rem);
    left: 0;
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 0.125rem;
    padding: 0.375rem;
    width: max-content;
    max-width: 13.5rem;
    border-radius: 12px;
    background: var(--popover-bg, #1c1c24);
    border: 1px solid rgba(255, 255, 255, 0.12);
    box-shadow:
      0 8px 28px rgba(0, 0, 0, 0.45),
      0 0 0 1px rgba(0, 0, 0, 0.25);
    backdrop-filter: blur(20px) saturate(1.4);
    -webkit-backdrop-filter: blur(20px) saturate(1.4);
  }

  .emoji-picker:focus {
    outline: none;
  }

  .emoji-cell {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem; /* 32px tap target */
    height: 2rem;
    padding: 0;
    border: none;
    border-radius: 8px;
    background: transparent;
    font-size: var(--text-lg);
    line-height: 1;
    cursor: pointer;
    transition: background-color 0.1s ease, transform 0.06s ease;
  }

  .emoji-cell:hover,
  .emoji-cell:focus-visible {
    background: rgba(255, 255, 255, 0.12);
    outline: none;
  }

  .emoji-cell:active {
    transform: scale(0.9);
  }
</style>
