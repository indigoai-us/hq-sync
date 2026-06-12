<script lang="ts">
  export interface AgentThreadAction {
    label: string;
    onselect?: () => void;
  }

  interface Props {
    enabled?: boolean;
    statusLines?: string[];
    resultTitle?: string;
    resultDetail?: string;
    handoffTitle?: string;
    actions?: AgentThreadAction[];
  }

  let {
    enabled = false,
    statusLines = [],
    resultTitle = '',
    resultDetail = '',
    handoffTitle = '',
    actions = [
      { label: 'Run now' },
      { label: 'Schedule' },
      { label: 'Decline' },
    ],
  }: Props = $props();
</script>

{#if enabled}
  <section class="agent-thread" data-testid="v4-agent-thread">
    <header>
      <span class="avatar" aria-hidden="true">⚡</span>
      <div>
        <h2>Your agent</h2>
        <p>{statusLines[0] ?? 'Ready when you are.'}</p>
      </div>
    </header>
    {#if statusLines.length > 1}
      <ul>
        {#each statusLines.slice(1) as line (line)}
          <li>{line}</li>
        {/each}
      </ul>
    {/if}
    {#if resultTitle}
      <article class="result-card">
        <h3>{resultTitle}</h3>
        {#if resultDetail}<p>{resultDetail}</p>{/if}
      </article>
    {/if}
    {#if handoffTitle}
      <article class="handoff-card">
        <h3>{handoffTitle}</h3>
        <div class="actions">
          {#each actions as action (action.label)}
            <button type="button" onclick={() => action.onselect?.()}>{action.label}</button>
          {/each}
        </div>
      </article>
    {/if}
  </section>
{/if}

<style>
  .agent-thread {
    display: grid;
    gap: 12px;
    width: min(520px, 100%);
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-panel);
  }

  header,
  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: var(--accent-soft);
    color: var(--fg);
  }

  h2,
  h3,
  p,
  ul {
    margin: 0;
  }

  h2,
  h3 {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
  }

  p,
  li {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1.45;
  }

  ul {
    display: grid;
    gap: 5px;
    padding-left: 18px;
  }

  .result-card,
  .handoff-card {
    display: grid;
    gap: 8px;
    padding: 11px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--surface-raise);
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
</style>
