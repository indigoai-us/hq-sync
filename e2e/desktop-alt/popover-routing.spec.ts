import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('menubar popover routes into V4 desktop surfaces', () => {
  const feed = readRepoFile('src/components/NotificationFeed.svelte');
  const route = readRepoFile('src/desktop-alt/route.ts');

  it('opens new-file notifications in the desktop company Activity screen', () => {
    expect(feed).toContain("invoke('open_desktop_alt_window'");
    expect(feed).toContain('route: `company:${it.file.company}:activity`');
    expect(feed).toContain("it.kind === 'new-file'");
    expect(feed).toContain('Boolean(it.file?.company)');
    expect(route).toContain("kind === 'company'");
    expect(route).toContain('isCompanyTab(second)');
  });
});
