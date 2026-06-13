<script lang="ts">
  // Reaction pills + add-reaction trigger under one message bubble (US-025).
  // Mounted by <Conversation/> beneath every bubble across DMs, channels, and
  // threads. Pure presentation: the parent owns the reaction map and the toggle
  // call (optimistic, reconciled by the message:reaction event) — this component
  // just renders the pills and bubbles a (messageId, emoji) toggle up.
  //
  // Tap-visible affordances (NOT hover-gated): the standalone window is frameless
  // and has no reliable hover, so the add-reaction "+" trigger is ALWAYS rendered
  // (mirrors the thread-affordance pattern in Conversation.svelte). Pills wrap at
  // narrow widths; every interactive element is a >=28px tap target.
  import { type ReactionAggregate } from '../../lib/reactions';
  import EmojiPicker from './EmojiPicker.svelte';

  interface Props {
    // This bubble's eventId — bubbled back with the chosen emoji so the host can
    // toggle the right message.
    messageId: string;
    // Aggregated reactions for this message (already sorted by the host). Empty
    // when the message has none — only the add-reaction trigger renders then.
    reactions?: ReactionAggregate[];
    // Called with (messageId, emoji) when a pill or a picked emoji is tapped. The
    // host performs the optimistic toggle + the toggle_reaction invoke.
    ontoggle: (messageId: string, emoji: string) => void;
  }

  let { messageId, reactions = [], ontoggle }: Props = $props();

  let pickerOpen = $state(false);

  function toggle(emoji: string): void {
    ontoggle(messageId, emoji);
  }

  function pick(emoji: string): void {
    pickerOpen = false;
    ontoggle(messageId, emoji);
  }
</script>

<div class="reaction-bar">
  {#each reactions as r (r.emoji)}
    <button
      class="reaction-pill"
      class:reacted={r.reactedByMe}
      type="button"
      onclick={() => toggle(r.emoji)}
      aria-pressed={r.reactedByMe}
      aria-label={`${r.emoji} ${r.count} ${r.count === 1 ? 'reaction' : 'reactions'}${r.reactedByMe ? ', you reacted' : ''}`}
    >
      <span class="reaction-emoji">{r.emoji}</span>
      <span class="reaction-count">{r.count}</span>
    </button>
  {/each}

  <div class="reaction-add-wrap">
    <button
      class="reaction-add"
      type="button"
      onclick={() => (pickerOpen = !pickerOpen)}
      aria-haspopup="menu"
      aria-expanded={pickerOpen}
      aria-label="Add a reaction"
      title="Add a reaction"
    >
      <span class="reaction-add-glyph" aria-hidden="true">☺</span>
      <span class="reaction-add-plus" aria-hidden="true">+</span>
    </button>
    {#if pickerOpen}
      <EmojiPicker onpick={pick} onclose={() => (pickerOpen = false)} />
    {/if}
  </div>
</div>

<style>
  /* Tap-visible (NOT hover-gated) — always rendered so the frameless window's
     missing hover never hides the affordance. Mirrors .thread-affordance. */
  .reaction-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem;
    align-self: inherit; /* hug the bubble side (in/out) it sits under */
    margin: 0.25rem 0.125rem 0;
  }

  .reaction-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    min-height: 1.75rem; /* 28px tap target */
    padding: 0.125rem 0.5rem;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.06);
    color: var(--popover-text, #e0e0e0);
    font-family: inherit;
    font-size: var(--text-base);
    line-height: 1;
    cursor: pointer;
    transition: background-color 0.12s ease, border-color 0.12s ease;
  }

  .reaction-pill:hover,
  .reaction-pill:focus-visible {
    background: rgba(255, 255, 255, 0.12);
    outline: none;
  }

  /* Highlighted when the caller is among the reactors. */
  .reaction-pill.reacted {
    background: rgba(120, 170, 255, 0.22);
    border-color: rgba(120, 170, 255, 0.5);
    color: #dce8ff;
  }

  .reaction-pill.reacted:hover,
  .reaction-pill.reacted:focus-visible {
    background: rgba(120, 170, 255, 0.3);
  }

  .reaction-emoji {
    font-size: var(--text-base);
    line-height: 1;
  }

  .reaction-count {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .reaction-add-wrap {
    position: relative;
    display: inline-flex;
  }

  .reaction-add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.0625rem;
    min-width: 1.75rem; /* 28px tap target */
    min-height: 1.75rem;
    padding: 0 0.375rem;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.04);
    color: var(--popover-text-muted, #a0a0b0);
    font-family: inherit;
    cursor: pointer;
    transition: background-color 0.12s ease, color 0.12s ease;
  }

  .reaction-add:hover,
  .reaction-add:focus-visible,
  .reaction-add[aria-expanded='true'] {
    background: rgba(255, 255, 255, 0.1);
    color: var(--popover-text, #e0e0e0);
    outline: none;
  }

  .reaction-add-glyph {
    font-size: var(--text-base);
    line-height: 1;
  }

  .reaction-add-plus {
    font-size: var(--text-base);
    font-weight: 700;
    line-height: 1;
  }
</style>
