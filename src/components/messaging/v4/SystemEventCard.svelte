<script lang="ts">
  export interface SystemAction {
    label: string;
    danger?: boolean;
    onselect?: () => void;
  }

  interface Props {
    title: string;
    detail?: string;
    tone?: 'ok' | 'warn' | 'error' | 'idle';
    timestamp?: string;
    actions?: SystemAction[];
  }

  let {
    title,
    detail = '',
    tone = 'idle',
    timestamp = '',
    actions = [],
  }: Props = $props();
</script>

<article class="system-event" data-tone={tone} data-testid="v4-system-event-card">
  <span class="dot" aria-hidden="true"></span>
  <div class="copy">
    <h3>{title}</h3>
    {#if detail}<p>{detail}</p>{/if}
    {#if timestamp}<time>{timestamp}</time>{/if}
  </div>
  {#if actions.length > 0}
    <div class="actions">
      {#each actions as action (action.label)}
        <button
          type="button"
          class:danger={action.danger}
          onclick={() => action.onselect?.()}
        >
          {action.label}
        </button>
      {/each}
    </div>
  {/if}
</article>

<style>
  .system-event {
    display: grid;
    grid-template-columns: 8px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: start;
    width: min(520px, 100%);
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-panel);
  }

  .dot {
    width: 7px;
    height: 7px;
    margin-top: 5px;
    border-radius: 50%;
    background: var(--muted-3);
  }

  [data-tone='ok'] .dot {
    background: var(--emerald);
  }

  [data-tone='warn'] .dot {
    background: var(--amber);
  }

  [data-tone='error'] .dot {
    background: var(--red);
  }

  h3,
  p {
    margin: 0;
  }

  h3 {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
  }

  p,
  time {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1.4;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  button {
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
  }

  button.danger {
    border-color: color-mix(in srgb, var(--red) 35%, transparent);
    color: var(--red);
  }
</style>
