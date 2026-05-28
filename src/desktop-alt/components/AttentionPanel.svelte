<script lang="ts">
  import type { AttentionItem } from '../lib/sync-model';

  interface Props {
    items: AttentionItem[];
    onaction: () => void;
  }

  let { items, onaction }: Props = $props();
</script>

<section class="attention-panel" aria-labelledby="attention-title">
  <div class="panel-header">
    <h2 id="attention-title">Attention</h2>
    <span>{items.length}</span>
  </div>

  {#if items.length === 0}
    <div class="attention-empty">
      <strong>No action needed</strong>
      <span>Reauth and paused source signals will appear here.</span>
    </div>
  {:else}
    <div class="attention-list">
      {#each items as item (item.key)}
        <article class="attention-item {item.tone}">
          <div>
            <strong>{item.title}</strong>
            <p>{item.detail}</p>
          </div>
          {#if item.actionLabel}
            <button type="button" onclick={onaction}>{item.actionLabel}</button>
          {/if}
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .attention-panel {
    min-width: 0;
  }

  .panel-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 10px;
  }

  .panel-header h2 {
    margin: 0;
    color: #18181b;
    font-size: 15px;
    font-weight: 680;
    line-height: 22px;
  }

  .panel-header span {
    color: #71717a;
    font-size: 12px;
  }

  .attention-empty,
  .attention-item {
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #ffffff;
  }

  .attention-empty {
    display: grid;
    gap: 3px;
    min-height: 74px;
    align-content: center;
    padding: 14px;
  }

  .attention-empty strong,
  .attention-item strong {
    color: #18181b;
    font-size: 13px;
    font-weight: 650;
    line-height: 18px;
  }

  .attention-empty span,
  .attention-item p {
    margin: 0;
    color: #71717a;
    font-size: 12px;
    line-height: 17px;
  }

  .attention-list {
    display: grid;
    gap: 8px;
  }

  .attention-item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: start;
    gap: 10px;
    padding: 12px;
    transition: transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .attention-item:hover {
    box-shadow: 0 1px 2px rgb(24 24 27 / 0.05);
    transform: translateY(-1px);
  }

  .attention-item.warn {
    border-color: #fecdd3;
    background: #fff7f8;
  }

  .attention-item.paused {
    border-color: #e4e4e7;
    background: #f8fafc;
  }

  .attention-item button {
    height: 26px;
    padding: 0 9px;
    border: 1px solid #d4d4d8;
    border-radius: 6px;
    background: #ffffff;
    color: #3f3f46;
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    white-space: nowrap;
    transition: transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .attention-item button:hover {
    background: #f4f4f5;
    transform: translateY(-1px);
  }

  .attention-item button:focus-visible {
    outline: 2px solid #2563eb;
    outline-offset: 2px;
  }

  @media (prefers-reduced-motion: reduce) {
    .attention-item,
    .attention-item button {
      transition: none;
    }

    .attention-item:hover,
    .attention-item button:hover {
      transform: none;
    }
  }
</style>
