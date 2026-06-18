<script lang="ts">
  import type { V4DotTone, V4SecondaryFooter, V4SecondaryItem } from './model';
  import './tokens.css';

  /**
   * V4 secondary (contextual) sidebar — SPEC section 4 + chrome-master.png:
   * 200px, inset background, hairline right border. Header + meta line share
   * the same 13px body size; hierarchy comes from weight and text color.
   * with hairline top border. Exactly one active row, driven by `activeId`.
   * Only rendered on surfaces that need it (company / library / settings).
   */
  interface Props {
    header: string;
    /** Optional status dot beside the header (company pages). */
    headerTone?: V4DotTone | null;
    /** Context line, e.g. "Owner · 3 members · synced just now". */
    meta?: string | null;
    items: V4SecondaryItem[];
    activeId: string | null;
    /** e.g. { label: "Company settings", meta: "sync rules · members · roles" }. */
    footer?: V4SecondaryFooter | null;
    onselect?: (id: string) => void;
    onfooterselect?: () => void;
  }

  let {
    header,
    headerTone = null,
    meta = null,
    items,
    activeId,
    footer = null,
    onselect,
    onfooterselect,
  }: Props = $props();
</script>

<aside class="v4-secondary" aria-label={`${header} sections`}>
  <header class="v4-context">
    <div class="v4-context-name">
      <span class="v4-context-title">{header}</span>
      {#if headerTone}
        <span class={`v4-dot ${headerTone}`} aria-hidden="true"></span>
      {/if}
    </div>
    {#if meta}
      <p class="v4-context-meta">{meta}</p>
    {/if}
  </header>

  <nav class="v4-menu" aria-label={header}>
    {#each items as item (item.id)}
      <button
        type="button"
        class="v4-row"
        class:active={item.id === activeId}
        aria-current={item.id === activeId ? 'page' : undefined}
        onclick={() => onselect?.(item.id)}
      >
        <span class="v4-row-label">{item.label}</span>
        {#if item.note}
          <span class="v4-row-note">{item.note}</span>
        {/if}
      </button>
    {/each}
  </nav>

  <div class="v4-spacer"></div>

  {#if footer}
    <button type="button" class="v4-footer" onclick={() => onfooterselect?.()}>
      <span class="v4-footer-label">{footer.label}</span>
      {#if footer.meta}
        <span class="v4-footer-meta">{footer.meta}</span>
      {/if}
    </button>
  {/if}
</aside>

<style>
  .v4-secondary {
    display: flex;
    flex-direction: column;
    flex: 0 0 200px;
    width: 200px;
    min-height: 0;
    height: 100%;
    padding: 16px 10px 0;
    border-right: 1px solid var(--v4-hairline);
    background: var(--v4-inset);
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .v4-context {
    margin-bottom: 14px;
    padding: 0 8px;
  }

  .v4-context-name {
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .v4-context-title {
    overflow: hidden;
    min-width: 0;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .v4-dot {
    flex: 0 0 6px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .v4-dot.ok {
    background: var(--v4-ok);
  }

  .v4-dot.warn {
    background: var(--v4-warn);
  }

  .v4-dot.error {
    background: var(--v4-error);
  }

  .v4-dot.idle {
    background: var(--v4-idle);
  }

  .v4-context-meta {
    margin: 4px 0 0;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.4;
  }

  .v4-menu {
    display: flex;
    flex-direction: column;
    gap: var(--v4-row-gap);
  }

  .v4-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: var(--v4-row-h);
    padding: 0 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1;
    text-align: left;
    cursor: pointer;
  }

  .v4-row:hover {
    background: var(--v4-control-faint);
    color: var(--v4-text-1);
  }

  .v4-row.active {
    background: var(--v4-active-row);
    color: var(--v4-text-1);
    font-weight: 500;
  }

  .v4-row-label {
    overflow: hidden;
    min-width: 0;
    flex: 1 1 auto;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .v4-row-note {
    flex: 0 0 auto;
    color: var(--v4-text-3);
    font-size: var(--text-base);
  }

  .v4-spacer {
    flex: 1 1 auto;
    min-height: 14px;
  }

  .v4-footer {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    margin: 0 -10px;
    padding: 12px 18px 14px;
    border: none;
    border-top: 1px solid var(--v4-hairline);
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .v4-footer:hover .v4-footer-label {
    color: var(--v4-text-1);
  }

  .v4-footer-label {
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.2;
  }

  .v4-footer-meta {
    overflow: hidden;
    max-width: 100%;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.2;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
