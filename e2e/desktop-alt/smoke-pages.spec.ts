import { describe, expect, it } from 'vitest';
import { createDesktopAltHarness } from './live-driver';

describe('desktop-alt smoke pages', () => {
  it.each([
    // The legacy 'sync' route resolves to the V4 Home surface (US-002/US-003).
    ['sync', 'Home'],
    ['meetings', 'Meetings'],
    ['company', 'New project'],
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
