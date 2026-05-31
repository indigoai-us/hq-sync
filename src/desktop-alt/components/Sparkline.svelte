<script lang="ts">
  interface Props {
    data: number[];
    width?: number;
    height?: number;
  }

  let { data, width = 80, height = 22 }: Props = $props();

  const max = $derived(Math.max(1, ...data));
  const stepX = $derived(data.length > 1 ? width / (data.length - 1) : width);
  const points = $derived(
    data
      .map((value, index) =>
        `${(index * stepX).toFixed(1)},${(height - (value / max) * (height - 2) - 1).toFixed(1)}`,
      )
      .join(' '),
  );
</script>

<svg class="sparkline" {width} {height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true">
  <polyline points={points} fill="none" stroke="currentColor" stroke-width="1" opacity="0.85" />
</svg>

<style>
  .sparkline {
    display: block;
    color: currentColor;
  }
</style>
