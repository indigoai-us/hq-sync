import { describe, it, expect } from 'vitest';
import { buildNotificationGroups, type Item, type Row } from './notificationGroups';

// Fixed reference clock so day labels are deterministic.
// 2026-06-04 18:00 local.
const NOW = new Date(2026, 5, 4, 18, 0, 0).getTime();
const at = (h: number, m: number, dayOffset = 0): number =>
  new Date(2026, 5, 4 + dayOffset, h, m, 0).getTime();

function newFile(
  id: string,
  company: string,
  actor: string,
  path: string,
  ts: number,
): Item {
  return {
    id,
    kind: 'new-file',
    actor,
    summary: `New file in ${company}: ${path}`,
    ts,
    file: { company, path },
  };
}

function dm(id: string, actor: string, ts: number): Item {
  return { id, kind: 'dm', actor, summary: 'hi', ts };
}

const onlyCluster = (rows: Row[]) => rows.filter((r) => r.type === 'cluster');
const onlySingle = (rows: Row[]) => rows.filter((r) => r.type === 'single');

describe('buildNotificationGroups', () => {
  it('collapses N same-company same-actor new files in a day into one cluster', () => {
    const items: Item[] = [
      newFile('a', 'indigo', 'jacob@getindigo.ai', 'x/1.txt', at(13, 55)),
      newFile('b', 'indigo', 'jacob@getindigo.ai', 'x/2.txt', at(13, 54)),
      newFile('c', 'indigo', 'jacob@getindigo.ai', 'x/3.txt', at(13, 53)),
    ];
    const groups = buildNotificationGroups(items, NOW);
    expect(groups).toHaveLength(1);
    expect(groups[0].label).toBe('Today');
    const clusters = onlyCluster(groups[0].rows);
    expect(clusters).toHaveLength(1);
    const c = clusters[0];
    if (c.type !== 'cluster') throw new Error('expected cluster');
    expect(c.count).toBe(3);
    expect(c.company).toBe('indigo');
    expect(c.actor).toBe('jacob@getindigo.ai');
    expect(c.latestTs).toBe(at(13, 55)); // newest member
    expect(c.items.map((i) => i.id)).toEqual(['a', 'b', 'c']);
  });

  it('keeps different actors in the same company as separate clusters', () => {
    const items: Item[] = [
      newFile('a', 'indigo', 'jacob@getindigo.ai', 'x/1.txt', at(13, 55)),
      newFile('b', 'indigo', 'jacob@getindigo.ai', 'x/2.txt', at(13, 54)),
      newFile('c', 'indigo', 'corey@getindigo.ai', 'y/1.txt', at(13, 53)),
      newFile('d', 'indigo', 'corey@getindigo.ai', 'y/2.txt', at(13, 52)),
    ];
    const groups = buildNotificationGroups(items, NOW);
    const clusters = onlyCluster(groups[0].rows);
    expect(clusters).toHaveLength(2);
    const actors = clusters.map((c) => (c.type === 'cluster' ? c.actor : '')).sort();
    expect(actors).toEqual(['corey@getindigo.ai', 'jacob@getindigo.ai']);
  });

  it('leaves a single new file as a single row (no cluster)', () => {
    const items: Item[] = [newFile('a', 'indigo', 'jacob@getindigo.ai', 'x/1.txt', at(13, 55))];
    const groups = buildNotificationGroups(items, NOW);
    expect(onlyCluster(groups[0].rows)).toHaveLength(0);
    const singles = onlySingle(groups[0].rows);
    expect(singles).toHaveLength(1);
    if (singles[0].type !== 'single') throw new Error('expected single');
    expect(singles[0].item.id).toBe('a');
  });

  it('keeps DMs/shares as single rows and preserves chronological order', () => {
    const items: Item[] = [
      dm('dm1', 'Jacob Posel', at(14, 32)),
      newFile('f1', 'indigo', 'corey@getindigo.ai', 'x/1.txt', at(13, 55)),
      newFile('f2', 'indigo', 'corey@getindigo.ai', 'x/2.txt', at(13, 54)),
      dm('dm2', 'Someone', at(13, 0)),
    ];
    const groups = buildNotificationGroups(items, NOW);
    const rows = groups[0].rows;
    // dm row, then the cluster (positioned at its newest member f1), then dm row.
    expect(rows.map((r) => r.type)).toEqual(['single', 'cluster', 'single']);
    expect(rows[0].type === 'single' && rows[0].item.id).toBe('dm1');
    expect(rows[2].type === 'single' && rows[2].item.id).toBe('dm2');
    const c = rows[1];
    if (c.type !== 'cluster') throw new Error('expected cluster');
    expect(c.count).toBe(2);
  });

  it('collapses independently per day', () => {
    const items: Item[] = [
      newFile('a', 'indigo', 'jacob@getindigo.ai', 'x/1.txt', at(13, 55)),
      newFile('b', 'indigo', 'jacob@getindigo.ai', 'x/2.txt', at(13, 54)),
      newFile('c', 'indigo', 'jacob@getindigo.ai', 'x/3.txt', at(13, 55, -1)),
      newFile('d', 'indigo', 'jacob@getindigo.ai', 'x/4.txt', at(13, 54, -1)),
    ];
    const groups = buildNotificationGroups(items, NOW);
    expect(groups).toHaveLength(2);
    expect(groups[0].label).toBe('Today');
    expect(groups[1].label).toBe('Yesterday');
    for (const g of groups) {
      const clusters = onlyCluster(g.rows);
      expect(clusters).toHaveLength(1);
      if (clusters[0].type !== 'cluster') throw new Error('expected cluster');
      expect(clusters[0].count).toBe(2);
    }
  });
});
