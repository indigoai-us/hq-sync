<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';

  // ── Wire types (mirror the Rust structs, camelCase) ──────────────────────────
  interface DmEvent {
    eventId: string;
    fromPersonUid: string;
    fromEmail: string;
    fromDisplayName: string;
    body: string;
    details?: string | null;
    prompt?: string | null;
    createdAt: string;
  }
  interface ShareEvent {
    eventId: string;
    issuerEmail: string;
    issuerDisplayName: string;
    paths: string[];
    note: string | null;
    permission: string;
    createdAt: string;
  }
  interface ActivityEntry {
    company: string;
    path: string;
    bytes: number;
    direction: string;
    author?: string;
    isNew?: boolean;
    at: number;
  }
  interface NotificationHistoryResponse {
    dms: DmEvent[];
    shares: ShareEvent[];
  }

  type Kind = 'dm' | 'share' | 'new-file';
  interface Item {
    id: string;
    kind: Kind;
    actor: string;
    summary: string;
    /** Epoch ms for sorting + day grouping. */
    ts: number;
    dm?: DmEvent;
    share?: ShareEvent;
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
    };
  }

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      // Server-retained history (DMs + shares). New-file rows are session-scoped
      // (the runner doesn't persist them server-side) — pulled from the local
      // activity log so they still appear in this session's timeline.
      const [history, activity] = await Promise.all([
        invoke<NotificationHistoryResponse>('fetch_notification_history'),
        invoke<ActivityEntry[]>('get_activity_log').catch(() => [] as ActivityEntry[]),
      ]);

      const merged: Item[] = [
        ...(history.dms ?? []).map(dmItem),
        ...(history.shares ?? []).map(shareItem),
        ...(activity ?? [])
          .filter((a) => a.isNew === true && a.direction === 'down')
          .map(newFileItem),
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

  // ── Day grouping (mirrors ActivityLog.svelte) ────────────────────────────────
  function dayKey(ms: number): string {
    const d = new Date(ms);
    return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
  }
  function dayLabel(ms: number): string {
    const d = new Date(ms);
    const today = new Date();
    const yest = new Date();
    yest.setDate(today.getDate() - 1);
    if (dayKey(ms) === dayKey(today.getTime())) return 'Today';
    if (dayKey(ms) === dayKey(yest.getTime())) return 'Yesterday';
    try {
      return new Intl.DateTimeFormat(undefined, {
        month: 'short',
        day: 'numeric',
        year: d.getFullYear() === today.getFullYear() ? undefined : 'numeric',
      }).format(d);
    } catch {
      return '';
    }
  }
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

  const groups = $derived(
    (() => {
      const out: Array<{ key: string; label: string; items: Item[] }> = [];
      let cur: { key: string; label: string; items: Item[] } | null = null;
      for (const it of items) {
        const k = dayKey(it.ts);
        if (!cur || cur.key !== k) {
          cur = { key: k, label: dayLabel(it.ts), items: [] };
          out.push(cur);
        }
        cur.items.push(it);
      }
      return out;
    })(),
  );

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
      }
      // new-file rows have no detail window — the file is already in the synced
      // folder; the row is informational.
    } catch (e) {
      console.error('notification-history: open failed', e);
    }
  }

  const clickable = (it: Item) => it.kind === 'dm' || it.kind === 'share';

  $effect(() => {
    void load();
  });
</script>

<div class="notif-root">
  <header class="notif-header" data-tauri-drag-region>
    <h1>Notifications</h1>
    <button class="notif-refresh" onclick={() => load()} disabled={loading} title="Refresh">
      ↻
    </button>
  </header>

  <div class="notif-body">
    {#if loading}
      <p class="notif-status">Loading…</p>
    {:else if error}
      <p class="notif-status notif-error" role="alert">{error}</p>
    {:else if items.length === 0}
      <p class="notif-status notif-empty">No past notifications.</p>
    {:else}
      {#each groups as group (group.key)}
        <div class="notif-day">
          <div class="notif-day-label">{group.label}</div>
          {#each group.items as it (it.id)}
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
          {/each}
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  :global(html[data-window='notification-history'], html[data-window='notification-history'] body) {
    background: #0b0b0d;
    margin: 0;
    height: 100%;
  }

  .notif-root {
    display: flex;
    flex-direction: column;
    height: 100vh;
    color: #e7e7ea;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    background: #0b0b0d;
  }

  .notif-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    padding-top: 28px; /* clear the overlay traffic-light area */
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .notif-header h1 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    letter-spacing: 0.01em;
  }
  .notif-refresh {
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: #c9c9cf;
    border-radius: 7px;
    width: 26px;
    height: 26px;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }
  .notif-refresh:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .notif-body {
    flex: 1;
    overflow-y: auto;
    padding: 8px 0 16px;
  }

  .notif-status {
    text-align: center;
    color: #8a8a90;
    font-size: 13px;
    margin-top: 40px;
  }
  .notif-error {
    color: #f0a3a3;
  }

  .notif-day {
    margin-top: 6px;
  }
  .notif-day-label {
    position: sticky;
    top: 0;
    background: #0b0b0d;
    color: #8a8a90;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 8px 16px 4px;
    z-index: 1;
  }

  .notif-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }
  .notif-row.clickable {
    cursor: pointer;
  }
  .notif-row.clickable:hover {
    background: rgba(255, 255, 255, 0.05);
  }

  .notif-glyph {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.08);
    color: #d8d8de;
    font-size: 12px;
    margin-top: 1px;
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
    font-size: 13px;
    font-weight: 600;
    color: #f2f2f4;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .notif-kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #7c7c82;
    flex: 0 0 auto;
  }
  .notif-summary {
    font-size: 12.5px;
    color: #b9b9c0;
    margin-top: 2px;
    line-height: 1.35;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .notif-time {
    flex: 0 0 auto;
    font-size: 11px;
    color: #76767c;
    margin-top: 1px;
  }
</style>
