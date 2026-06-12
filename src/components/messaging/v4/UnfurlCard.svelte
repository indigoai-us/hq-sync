<script lang="ts">
  export interface UnfurlMetric {
    label: string;
    value: string;
  }

  interface Props {
    kind?: 'story' | 'goal' | 'share';
    title: string;
    subtitle?: string;
    status?: string;
    acl?: string | null;
    metrics?: UnfurlMetric[];
    onopen?: () => void;
  }

  let {
    kind = 'story',
    title,
    subtitle = '',
    status = '',
    acl = null,
    metrics = [],
    onopen,
  }: Props = $props();
</script>

<article class="unfurl-card" data-kind={kind} data-testid="v4-unfurl-card">
  <header>
    <span class="eyebrow">{kind}</span>
    {#if status}
      <span class="status">{status}</span>
    {/if}
  </header>
  <h3>{title}</h3>
  {#if subtitle}
    <p>{subtitle}</p>
  {/if}
  {#if metrics.length > 0}
    <dl>
      {#each metrics as metric (metric.label)}
        <div>
          <dt>{metric.label}</dt>
          <dd>{metric.value}</dd>
        </div>
      {/each}
    </dl>
  {/if}
  {#if acl}
    <footer>{acl}</footer>
  {/if}
  {#if onopen}
    <button type="button" onclick={() => onopen?.()}>Open</button>
  {/if}
</article>

<style>
  .unfurl-card {
    display: grid;
    gap: 9px;
    width: min(420px, 100%);
    padding: 13px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-raise);
    color: var(--fg);
  }

  header,
  dl {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .eyebrow,
  .status,
  dt,
  footer {
    color: var(--muted-3);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    line-height: 1.2;
    text-transform: uppercase;
  }

  .status {
    margin-left: auto;
    color: var(--muted-2);
  }

  h3,
  p,
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  h3 {
    font-size: var(--text-base);
    font-weight: 600;
  }

  p {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1.45;
  }

  dl {
    flex-wrap: wrap;
    margin: 0;
  }

  dl div {
    display: grid;
    gap: 2px;
    min-width: 72px;
  }

  dd {
    color: var(--fg-data);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  button {
    justify-self: start;
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
