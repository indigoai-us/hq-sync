import { describe, expect, it } from 'vitest';
import { createDesktopAltHarness } from './live-driver';

describe('desktop-alt smoke pages', () => {
  it.each([
    ['sync', 'Sync'],
    ['meetings', 'Meetings'],
    ['company', 'Companies'],
  ] as const)('renders %s without console errors', async (route, expectedMarker) => {
    const app = await createDesktopAltHarness('qa@getindigo.ai');

    try {
      await app.clickDesktopAltToggle();

      const page = await app.navigate(route);

      expect(page.consoleErrors).toEqual([]);
      expect(page.text.some((text) => text.includes(expectedMarker))).toBe(true);
    } finally {
      await app.dispose?.();
    }
  });
});
