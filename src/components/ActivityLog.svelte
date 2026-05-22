<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  interface ActivityEntry {
    company: string;
    path: string;
    bytes: number;
    /** "up" | "down" | "deleted" */
    direction: string;
    /** epoch millis */
    at: number;
  }

  let entries = $state<ActivityEntry[]>([]);

  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  function formatTime(ms: number): string {
    return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  /** Stable day key (local) for grouping. */
  function dayKey(ms: number): string {
    const d = new Date(ms);
    return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
  }

  /** Human label for a day key relative to today. */
  function dayLabel(ms: number): string {
    const d = new Date(ms);
    const today = new Date();
    const yesterday = new Date();
    yesterday.setDate(today.getDate() - 1);
    if (dayKey(ms) === dayKey(today.getTime())) return 'Today';
    if (dayKey(ms) === dayKey(yesterday.getTime())) return 'Yesterday';
    return d.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
  }

  // Newest-first, grouped by day. Recomputed whenever `entries` changes.
  const groups = $derived.by(() => {
    const sorted = [...entries].sort((a, b) => b.at - a.at);
    const out: { key: string; label: string; items: ActivityEntry[] }[] = [];
    for (const e of sorted) {
      const key = dayKey(e.at);
      let g = out.find((x) => x.key === key);
      if (!g) {
        g = { key, label: dayLabel(e.at), items: [] };
        out.push(g);
      }
      g.items.push(e);
    }
    return out;
  });

  function dirMeta(direction: string): { label: string; cls: string; glyph: string } {
    switch (direction) {
      case 'up':
        return { label: 'Uploaded', cls: 'dir-up', glyph: '↑' };
      case 'deleted':
        return { label: 'Deleted', cls: 'dir-del', glyph: '✕' };
      case 'down':
      default:
        return { label: 'Downloaded', cls: 'dir-down', glyph: '↓' };
    }
  }

  $effect(() => {
    let offList: (() => void) | undefined;
    let offAppend: (() => void) | undefined;

    Promise.all([
      listen<ActivityEntry[]>('activity:list', (event) => {
        entries = event.payload;
      }),
      listen<ActivityEntry>('activity:append', (event) => {
        entries = [...entries, event.payload];
      }),
    ]).then(([offL, offA]) => {
      offList = offL;
      offAppend = offA;
      // Handshake: tell Rust our listeners are registered so it can emit the
      // current snapshot and show the window (race-free, mirrors New Files).
      invoke('activity_window_ready');
    });

    return () => {
      offList?.();
      offAppend?.();
    };
  });
</script>

<div class="detail-window">
  <header class="detail-header">
    <h1>Recent Changes</h1>
    <span class="detail-count">
      {entries.length} change{entries.length === 1 ? '' : 's'} this session
    </span>
  </header>

  {#if entries.length === 0}
    <div class="detail-empty">
      <p>No file changes synced yet this session.</p>
    </div>
  {:else}
    <div class="detail-list">
      {#each groups as group (group.key)}
        <div class="day-header">{group.label}</div>
        {#each group.items as item (item.path + item.at)}
          {@const meta = dirMeta(item.direction)}
          <div class="detail-row">
            <span class="col-dir {meta.cls}" title={meta.label}>
              <span class="dir-glyph">{meta.glyph}</span>
              <span class="dir-label">{meta.label}</span>
            </span>
            <span class="col-path detail-path" title={`${item.company}/${item.path}`}>
              <span class="path-main">{item.path}</span>
              <span class="path-company">{item.company}</span>
            </span>
            <span class="col-time">{formatTime(item.at)}</span>
            <span class="col-size">{formatBytes(item.bytes)}</span>
          </div>
        {/each}
      {/each}
    </div>
  {/if}
</div>

<style>
  .detail-window {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    background: var(--popover-bg, rgba(18, 18, 20, 0.68));
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    color: var(--popover-text, #e0e0e0);
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    overflow: hidden;
  }

  .detail-header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    padding: 1rem 1.25rem 0.75rem;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    flex-shrink: 0;
  }

  .detail-header h1 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  .detail-count {
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .detail-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .detail-empty p {
    font-size: 0.8125rem;
    color: var(--popover-text-muted, #a0a0b0);
    margin: 0;
  }

  .detail-list {
    flex: 1;
    overflow-y: auto;
    padding: 0.25rem 0 0.75rem;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
  }

  .detail-list::-webkit-scrollbar {
    width: 6px;
  }
  .detail-list::-webkit-scrollbar-track {
    background: transparent;
  }
  .detail-list::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
  }
  .detail-list:hover::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.22);
  }

  .day-header {
    position: sticky;
    top: 0;
    z-index: 1;
    padding: 0.5rem 1.25rem 0.3rem;
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--popover-text-muted, #a0a0b0);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    background: var(--popover-bg, rgba(18, 18, 20, 0.92));
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
  }

  .detail-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1.25rem;
    font-size: 0.8125rem;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.05));
    transition: background-color 0.1s ease;
  }
  .detail-row:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
  }

  .col-dir {
    width: 104px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.7rem;
    font-weight: 600;
  }
  .dir-glyph {
    font-size: 0.8rem;
    line-height: 1;
  }
  .dir-up {
    color: #5ad27e;
  }
  .dir-down {
    color: #6ab3ff;
  }
  .dir-del {
    color: #ff8a8a;
  }

  .col-path {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .path-main {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--popover-text, #e0e0e0);
  }
  .path-company {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #8a8a98);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .col-time {
    width: 58px;
    flex-shrink: 0;
    text-align: right;
    font-size: 0.7rem;
    color: var(--popover-text-muted, #a0a0b0);
    font-variant-numeric: tabular-nums;
  }

  .col-size {
    width: 66px;
    flex-shrink: 0;
    text-align: right;
    font-size: 0.7rem;
    color: var(--popover-text-muted, #a0a0b0);
    font-variant-numeric: tabular-nums;
  }
</style>
