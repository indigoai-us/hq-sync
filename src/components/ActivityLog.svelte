<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  interface ActivityEntry {
    company: string;
    path: string;
    bytes: number;
    /** "up" | "down" | "deleted" */
    direction: string;
    /** Email of the file's author (from S3 created-by). Only download rows. */
    author?: string;
    /** True if a downloaded file was new to the drive ("added"), false if it
     *  was an update ("updated"), undefined when not yet known. */
    isNew?: boolean;
    /** epoch millis */
    at: number;
  }

  let entries = $state<ActivityEntry[]>([]);

  /** Only the most recent N changes are shown; older ones scroll off. */
  const MAX_VISIBLE = 100;

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

  // Newest-first, capped at MAX_VISIBLE, grouped by day. Recomputed whenever
  // `entries` changes.
  const groups = $derived.by(() => {
    const sorted = [...entries]
      .sort((a, b) => b.at - a.at)
      .slice(0, MAX_VISIBLE);
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

  /**
   * Past-tense action verb describing what the author did, for the attribution
   * line ("Tom added" / "Tom updated"). Downloads distinguish added (new file)
   * from updated (changed file) when the runner's new-files event has reconciled
   * `isNew`; an un-reconciled download falls back to "updated".
   */
  function actionVerb(item: ActivityEntry): string {
    switch (item.direction) {
      case 'up':
        return 'uploaded';
      case 'deleted':
        return 'deleted';
      case 'down':
      default:
        return item.isNew ? 'added' : 'updated';
    }
  }

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
    let offAppend: (() => void) | undefined;
    let offList: (() => void) | undefined;

    // Pull the current snapshot on mount. This is the authoritative load —
    // robust against emit-timing races (the old emit-on-ready handshake could
    // fire before this webview's listener registered, leaving the window
    // empty). The window is shown by Rust on open; no ready-handshake needed.
    invoke<ActivityEntry[]>('get_activity_log').then((list) => {
      entries = list;
    });

    // Live updates: new entries recorded while the window is open are pushed
    // via `activity:append`. We also keep `activity:list` as a re-sync hook
    // (emitted when an already-open window is re-focused).
    listen<ActivityEntry>('activity:append', (event) => {
      entries = [...entries, event.payload];
    }).then((off) => {
      offAppend = off;
    });
    listen<ActivityEntry[]>('activity:list', (event) => {
      entries = event.payload;
    }).then((off) => {
      offList = off;
    });

    return () => {
      offAppend?.();
      offList?.();
    };
  });
</script>

<div class="detail-window">
  <header class="detail-header" data-tauri-drag-region>
    <h1>Recent Changes</h1>
    <span class="detail-count">
      {#if entries.length > MAX_VISIBLE}
        latest {MAX_VISIBLE} of {entries.length} this session
      {:else}
        {entries.length} change{entries.length === 1 ? '' : 's'} this session
      {/if}
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
              <span class="path-company">
                {#if item.author}<span class="path-author">{item.author}</span> {actionVerb(item)}{:else}{item.company}{/if}
              </span>
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
  /* Reset the root document for THIS window only (scoped by the
     data-window attribute main.ts stamps), mirroring App.svelte's main-window
     reset. Without this the default 8px <body> margin offsets our content and
     the transparent+vibrant window shows a gray strip of bare NSVisualEffect
     along the top/left edges. Scoped so it can't bleed into other windows. */
  :global(html[data-window='activity-log']),
  :global(html[data-window='activity-log'] body) {
    margin: 0;
    padding: 0;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    background: transparent;
  }

  .detail-window {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    /* Sit a mostly-opaque dark layer over the NSVisualEffect vibrancy so the
       glass reads as a consistent dark surface — just a hint of translucency —
       rather than letting the busy content behind the window bleed through as
       colored blotches. Higher alpha than the popover (0.68) because this
       window is large and often sits over a terminal/editor. */
    background: rgba(20, 20, 24, 0.88);
    backdrop-filter: blur(30px) saturate(1.2);
    -webkit-backdrop-filter: blur(30px) saturate(1.2);
    color: var(--popover-text, #e0e0e0);
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    overflow: hidden;
  }

  .detail-header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    /* Extra top padding clears the macOS traffic-light buttons that the
       Overlay title-bar style floats over the body's top-left. */
    padding: 2.25rem 1.25rem 0.75rem;
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
    /* Slightly more opaque than the body so rows scrolling under it stay
       legible; same hue as the window surface to avoid a seam. */
    background: rgba(20, 20, 24, 0.95);
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
  .path-author {
    /* The "who" — slightly brighter than the trailing verb so it reads as the
       subject of "{author} added/updated". */
    color: var(--popover-text, #c8c8d2);
    font-weight: 500;
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
