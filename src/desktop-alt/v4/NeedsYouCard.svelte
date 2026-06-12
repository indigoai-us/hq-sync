<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HomeCardModel } from './home-model';
  import './tokens.css';

  /**
   * V4 inline-action card — the NEEDS YOU queue unit (home-healthy.png) and
   * the error card (home-error.png). Raised surface, 13px title, 11px sub,
   * a right-aligned action row (one primary fill, secondary outline, quiet
   * text actions), and an optional extra region (the error card's collapsible
   * "Technical details" inset renders through the snippet).
   *
   * Tone tints the border at ~0.3 alpha — allowed ONLY on needs-attention /
   * error cards per SPEC section 2.
   */
  interface Props {
    card: HomeCardModel;
    onaction?: (id: string) => void;
    children?: Snippet;
  }

  let { card, onaction, children }: Props = $props();
</script>

<div class={`v4-card ${card.tone}`} data-testid="needs-you-card">
  <div class="v4-card-row">
    <div class="v4-card-copy">
      <p class="v4-card-title">{card.title}</p>
      {#if card.sub}
        <p class="v4-card-sub">{card.sub}</p>
      {/if}
    </div>
    {#if card.actions.length > 0}
      <div class="v4-card-actions">
        {#each card.actions as action (action.id)}
          <button
            type="button"
            class={`v4-card-action ${action.kind}`}
            disabled={action.disabled}
            onclick={() => onaction?.(action.id)}
          >
            {action.label}
          </button>
        {/each}
      </div>
    {/if}
  </div>
  {#if children}
    {@render children()}
  {/if}
</div>

<style>
  .v4-card {
    padding: 12px 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
  }

  .v4-card.warn {
    border-color: rgba(254, 188, 46, 0.3);
  }

  .v4-card.error {
    border-color: rgba(255, 69, 58, 0.3);
  }

  .v4-card-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .v4-card-copy {
    min-width: 0;
    display: grid;
    gap: 3px;
  }

  .v4-card-title {
    margin: 0;
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 400;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }

  .v4-card-sub {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    line-height: 1.4;
  }

  .v4-card-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 0 auto;
  }

  .v4-card-action {
    padding: 5px 10px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-1);
    font: inherit;
    font-size: 13px;
    font-weight: 400;
    line-height: 1;
    white-space: nowrap;
    cursor: pointer;
  }

  .v4-card-action.primary {
    background: var(--v4-control-bg);
  }

  .v4-card-action.secondary {
    border-color: var(--v4-control-border);
  }

  .v4-card-action.text {
    color: var(--v4-text-2);
  }

  .v4-card-action:hover:not(:disabled) {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  .v4-card-action:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
