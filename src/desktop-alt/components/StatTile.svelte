<script lang="ts">
  interface Props {
    label: string;
    value: string | number;
    hint?: string | null;
    loading?: boolean;
  }

  let { label, value, hint = null, loading = false }: Props = $props();
</script>

<article class="stat-tile" aria-busy={loading}>
  {#if loading}
    <span class="skeleton label-skeleton" aria-hidden="true"></span>
    <span class="skeleton value-skeleton" aria-hidden="true"></span>
  {:else}
    <span class="stat-label">{label}</span>
    <strong>{value}</strong>
    {#if hint}
      <span class="stat-hint">{hint}</span>
    {/if}
  {/if}
</article>

<style>
  .stat-tile {
    display: grid;
    align-content: start;
    gap: var(--space-1);
    min-width: 0;
    padding: var(--space-3) var(--space-3) calc(var(--space-3) - 1px);
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--row-active);
  }

  .stat-label,
  .stat-hint {
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.06em;
    line-height: 14px;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
  }

  strong {
    min-width: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 20px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .stat-hint {
    font-weight: 600;
  }

  .skeleton {
    display: block;
    overflow: hidden;
    border-radius: 999px;
    background: linear-gradient(
      90deg,
      var(--v4-control-faint) 0%,
      var(--v4-hairline) 46%,
      var(--v4-control-faint) 100%
    );
    background-size: 180% 100%;
    animation: skeleton-pulse 1100ms ease-in-out infinite;
  }

  .label-skeleton {
    width: 72%;
    height: 14px;
  }

  .value-skeleton {
    width: 46%;
    height: 28px;
    margin-top: 4px;
  }

  @keyframes skeleton-pulse {
    from {
      background-position: 100% 0;
    }

    to {
      background-position: 0 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton {
      animation: none;
    }
  }
</style>
