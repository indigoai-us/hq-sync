<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import {
    buildNotificationGroups,
    type DmEvent,
    type ShareEvent,
    type Item,
    type Kind,
  } from '../lib/notificationGroups';

  // Inline notifications feed. Extracted from the old standalone
  // `NotificationHistory` window (US-009 era) so the menubar popover can host
  // the timeline directly as its default body — no separate window. The pure
  // grouping logic still lives in `../lib/notificationGroups` (shared with its
  // unit tests); this component owns the data load, the day/cluster rendering,
  // and the row-tap routing into the DM/share detail windows plus V4 desktop
  // company Activity routes for synced-file rows.

  // ── Wire types (mirror the Rust structs, camelCase) ──────────────────────────
  interface ActivityEntry {
    company: string;
    path: string;
    bytes: number;
    direction: string;
    author?: string;
    isNew?: boolean;
    at: number;
  }
  interface FileHistoryItem {
    eventId: string;
    path: string;
    bytes?: number;
    addedBy?: string;
    companyUid?: string;
    companySlug?: string;
    createdAt: string;
  }
  interface NotificationHistoryResponse {
    dms: DmEvent[];
    shares: ShareEvent[];
    files: FileHistoryItem[];
  }

  let loading = $state(true);
  let error = $state<string | null>(null);
  let items = $state<Item[]>([]);

  function parseTs(iso: string): number {
    const t = Date.parse(iso);
    return Number.isNaN(t) ? 0 : t;
  }

  function dmItem(e: DmEvent): Item {
    return {
      id: `dm:${e.eventId}`,
      kind: 'dm',
      actor: e.fromDisplayName?.trim() || e.fromEmail || 'Someone',
      summary: e.body,
      ts: parseTs(e.createdAt),
      dm: e,
    };
  }
  function shareItem(e: ShareEvent): Item {
    const n = e.paths.length;
    const files = e.paths.join(', ');
    const base = n === 1 ? `Shared a file: ${files}` : `Shared ${n} files: ${files}`;
    const summary = e.note && e.note.trim() ? `${base} — “${e.note.trim()}”` : base;
    return {
      id: `share:${e.eventId}`,
      kind: 'share',
      actor: e.issuerDisplayName?.trim() || e.issuerEmail || 'Someone',
      summary,
      ts: parseTs(e.createdAt),
      share: e,
    };
  }
  function newFileItem(e: ActivityEntry): Item {
    return {
      id: `newfile:${e.company}/${e.path}:${e.at}`,
      kind: 'new-file',
      actor: e.author?.trim() || e.company,
      summary: `New file in ${e.company}: ${e.path}`,
      ts: e.at,
      file: { company: e.company, path: e.path },
    };
  }
  /** Cross-session new-file row from the server file-history feed. */
  function serverFileItem(f: FileHistoryItem): Item {
    const co = f.companySlug || f.companyUid || '';
    return {
      id: `filehist:${f.eventId}`,
      kind: 'new-file',
      actor: f.addedBy?.trim() || co || 'Sync',
      summary: co ? `New file in ${co}: ${f.path}` : `New file: ${f.path}`,
      ts: parseTs(f.createdAt),
      file: { company: co, path: f.path },
    };
  }
  /** Dedup key so a file present in BOTH the server feed and the current
   *  session's activity log isn't shown twice (server is authoritative). */
  function fileKey(company: string, path: string): string {
    return `${company} ${path}`;
  }

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      // Server-retained history: DMs + shares + cross-session new files (the
      // sync runner reports new files to the server, so they persist across
      // restarts). The local activity log adds any of THIS session's new files
      // not yet reflected server-side (e.g. an older CLI that doesn't emit, or
      // the just-finished sync) — deduped against the server feed by company+path.
      const [history, activity] = await Promise.all([
        invoke<NotificationHistoryResponse>('fetch_notification_history'),
        invoke<ActivityEntry[]>('get_activity_log').catch(() => [] as ActivityEntry[]),
      ]);

      const serverFiles = history.files ?? [];
      const seenFiles = new Set(
        serverFiles.map((f) => fileKey(f.companySlug || f.companyUid || '', f.path)),
      );
      const sessionNewFiles = (activity ?? [])
        .filter((a) => a.isNew === true && a.direction === 'down')
        .filter((a) => !seenFiles.has(fileKey(a.company, a.path)));

      const merged: Item[] = [
        ...(history.dms ?? []).map(dmItem),
        ...(history.shares ?? []).map(shareItem),
        ...serverFiles.map(serverFileItem),
        ...sessionNewFiles.map(newFileItem),
      ];
      merged.sort((a, b) => b.ts - a.ts);
      items = merged;
    } catch (e) {
      error = typeof e === 'string' ? e : 'Could not load notifications.';
      items = [];
    } finally {
      loading = false;
    }
  }

  /** Exposed so a parent can force a refresh (e.g. on popover focus). */
  export function reload(): void {
    void load();
  }

  // ── Day grouping (mirrors ActivityLog.svelte) ────────────────────────────────
  function formatTime(ms: number): string {
    try {
      return new Intl.DateTimeFormat(undefined, {
        hour: 'numeric',
        minute: '2-digit',
      }).format(new Date(ms));
    } catch {
      return '';
    }
  }

  // Day grouping + per-(company, actor) collapse of new-file rows lives in the
  // pure, unit-tested notificationGroups module.
  const groups = $derived(buildNotificationGroups(items));

  // Which new-file clusters are expanded inline (by cluster key).
  let expanded = $state(new Set<string>());
  function toggleCluster(key: string): void {
    const next = new Set(expanded);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expanded = next;
  }

  function kindGlyph(kind: Kind): string {
    switch (kind) {
      case 'dm':
        return '✦'; // message
      case 'share':
        return '⇲'; // shared-with-me
      case 'new-file':
        return '＋'; // new file
      default:
        return '•';
    }
  }
  function kindLabel(kind: Kind): string {
    switch (kind) {
      case 'dm':
        return 'Message';
      case 'share':
        return 'Shared';
      case 'new-file':
        return 'New file';
      default:
        return '';
    }
  }

  async function openItem(it: Item): Promise<void> {
    try {
      if (it.kind === 'dm' && it.dm) {
        await invoke('open_dm_detail', { event: it.dm });
      } else if (it.kind === 'share' && it.share) {
        await invoke('open_share_detail', { events: [it.share] });
      } else if (it.kind === 'new-file' && it.file?.company) {
        await invoke('open_desktop_alt_window', {
          route: `company:${it.file.company}:activity`,
        });
      }
    } catch (e) {
      console.error('notification-feed: open failed', e);
    }
  }

  const clickable = (it: Item) =>
    it.kind === 'dm' || it.kind === 'share' || (it.kind === 'new-file' && Boolean(it.file?.company));

  // Load on mount, then keep the feed fresh by reloading when new content
  // arrives. A DM lands as `dm:unread-summary`; new files land at `sync:complete`.
  // Both are cheap signals — debounce a single reload so a burst doesn't stack
  // fetches. Listeners are torn down with the component.
  $effect(() => {
    void load();

    let reloadTimer: ReturnType<typeof setTimeout> | null = null;
    const scheduleReload = () => {
      if (reloadTimer) clearTimeout(reloadTimer);
      reloadTimer = setTimeout(() => {
        reloadTimer = null;
        void load();
      }, 400);
    };

    const unlisteners: Array<() => void> = [];
    void listen('dm:unread-summary', scheduleReload).then((u) => unlisteners.push(u));
    void listen('sync:complete', scheduleReload).then((u) => unlisteners.push(u));

    return () => {
      if (reloadTimer) clearTimeout(reloadTimer);
      for (const u of unlisteners) u();
    };
  });
</script>

<div class="notif-feed">
  {#if loading && items.length === 0}
    <p class="notif-status">Loading…</p>
  {:else if error}
    <p class="notif-status notif-error" role="alert">{error}</p>
  {:else if items.length === 0}
    <p class="notif-status notif-empty">You're all caught up — no notifications yet.</p>
  {:else}
    {#each groups as group (group.key)}
      <div class="notif-day">
        <div class="notif-day-label">{group.label}</div>
        {#each group.rows as row (row.type === 'cluster' ? row.key : row.item.id)}
          {#if row.type === 'single'}
            {@const it = row.item}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <div
              class="notif-row notif-{it.kind}"
              class:clickable={clickable(it)}
              role={clickable(it) ? 'button' : undefined}
              tabindex={clickable(it) ? 0 : undefined}
              onclick={() => clickable(it) && openItem(it)}
              onkeydown={(e) => clickable(it) && (e.key === 'Enter' || e.key === ' ') && openItem(it)}
            >
              <span class="notif-glyph" aria-hidden="true">{kindGlyph(it.kind)}</span>
              <div class="notif-main">
                <div class="notif-line1">
                  <span class="notif-actor">{it.actor}</span>
                  <span class="notif-kind">{kindLabel(it.kind)}</span>
                </div>
                <div class="notif-summary">{it.summary}</div>
              </div>
              <span class="notif-time">{formatTime(it.ts)}</span>
            </div>
          {:else}
            {@const open = expanded.has(row.key)}
            <div
              class="notif-row notif-new-file notif-cluster clickable"
              role="button"
              tabindex="0"
              aria-expanded={open}
              onclick={() => toggleCluster(row.key)}
              onkeydown={(e) =>
                (e.key === 'Enter' || e.key === ' ') &&
                (e.preventDefault(), toggleCluster(row.key))}
            >
              <span class="notif-glyph" aria-hidden="true">{kindGlyph('new-file')}</span>
              <div class="notif-main">
                <div class="notif-line1">
                  <span class="notif-actor">{row.actor}</span>
                  <span class="notif-kind">{kindLabel('new-file')}</span>
                </div>
                <div class="notif-summary">
                  {row.count} new files in {row.company}
                </div>
              </div>
              <span class="notif-chevron" aria-hidden="true">{open ? '▾' : '▸'}</span>
              <span class="notif-time">{formatTime(row.latestTs)}</span>
            </div>
            {#if open}
              <div class="notif-cluster-files">
                {#each row.items as it (it.id)}
                  <div class="notif-file-row">
                    <span class="notif-file-path" title={it.file?.path}>{it.file?.path}</span>
                    <span class="notif-file-time">{formatTime(it.ts)}</span>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}
        {/each}
      </div>
    {/each}
  {/if}
</div>

<style>
  /* Inline feed — sized by the popover body, not a window. No 100vh root and
     no window-scoped html background (those lived on the old standalone
     window). Rows + accents are carried over verbatim so the timeline reads
     identically to the window it replaces. */
  .notif-feed {
    display: flex;
    flex-direction: column;
  }

  .notif-status {
    text-align: center;
    color: #8a8a90;
    font-size: var(--text-sm);
    padding: 22px 16px;
    margin: 0;
  }
  .notif-error {
    color: #f0a3a3;
  }

  .notif-day {
    margin-top: 2px;
  }
  .notif-day-label {
    position: sticky;
    top: 0;
    background: var(--popover-bg, #0b0b0d);
    color: #8a8a90;
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 7px 14px 4px;
    z-index: 1;
  }

  .notif-row {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: 11px;
    padding: 9px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }
  .notif-row.clickable {
    cursor: pointer;
  }
  .notif-row.clickable:hover {
    background: rgba(255, 255, 255, 0.05);
  }

  /* ── Per-type identity ───────────────────────────────────────────────────────
     Each notification kind carries its own accent. Human, actionable events
     (a DM you can reply to, a file shared with you) get a saturated accent +
     a left edge-bar so they pop in the scan. Ambient sync activity (new files)
     stays a quiet neutral so the wall of "44 files synced" recedes. */
  .notif-dm {
    --accent: #7e8cff;
    --accent-soft: rgba(126, 140, 255, 0.16);
  }
  .notif-share {
    --accent: #46d6a6;
    --accent-soft: rgba(70, 214, 166, 0.16);
  }
  .notif-new-file {
    --accent: #8a8a92;
    --accent-soft: rgba(255, 255, 255, 0.06);
  }

  /* Left edge-bar marks the human, actionable rows (dm + share). */
  .notif-dm::before,
  .notif-share::before {
    content: '';
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--accent);
    opacity: 0.85;
  }

  .notif-glyph {
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 7px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--text-sm);
    margin-top: 1px;
  }
  /* Ambient rows keep a neutral glyph — colour is reserved for human events. */
  .notif-new-file .notif-glyph {
    color: #c2c2c8;
  }

  .notif-main {
    flex: 1;
    min-width: 0;
  }
  .notif-line1 {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .notif-actor {
    font-size: var(--text-base);
    font-weight: 600;
    color: #f2f2f4;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .notif-kind {
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--accent);
    background: var(--accent-soft);
    padding: 1px 6px;
    border-radius: 4px;
    flex: 0 0 auto;
  }
  /* Ambient new-file label stays quiet — no colour, barely-there chip. */
  .notif-new-file .notif-kind {
    color: #8a8a90;
    background: rgba(255, 255, 255, 0.05);
  }
  .notif-summary {
    font-size: var(--text-sm);
    color: #b9b9c0;
    margin-top: 2px;
    line-height: 1.35;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .notif-time {
    flex: 0 0 auto;
    font-size: var(--text-xs);
    color: #76767c;
    margin-top: 1px;
  }

  .notif-chevron {
    flex: 0 0 auto;
    font-size: var(--text-xs);
    color: #76767c;
    margin-top: 2px;
    width: 10px;
    text-align: center;
  }

  /* Inline file list revealed when a new-file cluster is expanded. */
  .notif-cluster-files {
    padding: 2px 14px 6px 44px; /* indent under the glyph */
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }
  .notif-file-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 4px 0;
  }
  .notif-file-path {
    flex: 1;
    min-width: 0;
    font-size: var(--text-xs);
    color: #b9b9c0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .notif-file-time {
    flex: 0 0 auto;
    font-size: var(--text-xs);
    color: #76767c;
    font-variant-numeric: tabular-nums;
  }
</style>
