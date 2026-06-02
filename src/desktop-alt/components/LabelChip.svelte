<script lang="ts">
  import { labelColor } from '../lib/projects-model';

  interface Props {
    /** The label string to render. */
    label: string;
  }

  let { label }: Props = $props();

  // Deterministic monochrome-glass color from the US-004 label palette. Same
  // string always resolves to the same shade. We feed the resolved hsla() values
  // into inline CSS custom properties so the chip stays token/identity-driven
  // (no hardcoded hex, no indigo/Tailwind palette) while still being per-label.
  const color = $derived(labelColor(label));
</script>

<span
  class="label-chip"
  title={label}
  style={`--chip-bg: ${color.background}; --chip-border: ${color.border}; --chip-fg: ${color.foreground};`}
>
  {label}
</span>

<style>
  .label-chip {
    display: inline-flex;
    align-items: center;
    max-width: 100%;
    overflow: hidden;
    padding: 1px 7px;
    border: 1px solid var(--chip-border);
    border-radius: var(--radius-sm);
    background: var(--chip-bg);
    color: var(--chip-fg);
    font-size: var(--text-xs);
    font-weight: 600;
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
