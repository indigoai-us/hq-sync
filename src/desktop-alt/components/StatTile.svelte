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
    gap: 7px;
    min-width: 0;
    min-height: 92px;
    padding: 13px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }

  .stat-label,
  .stat-hint {
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  strong {
    min-width: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 720;
    line-height: 30px;
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
      rgba(255, 255, 255, 0.05) 0%,
      rgba(255, 255, 255, 0.1) 46%,
      rgba(255, 255, 255, 0.05) 100%
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
