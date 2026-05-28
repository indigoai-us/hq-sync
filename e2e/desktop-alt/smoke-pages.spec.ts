import { describe, expect, it } from 'vitest';
import { DesktopAltHarness } from './harness';

describe('desktop-alt smoke pages', () => {
  it.each([
    ['sync', 'Sync'],
    ['meetings', 'Meetings'],
    ['company', 'CompanyTabs'],
  ] as const)('renders %s without console errors', (route, expectedMarker) => {
    const app = new DesktopAltHarness('qa@getindigo.ai');
    app.clickDesktopAltToggle();

    const page = app.navigate(route);

    expect(page.consoleErrors).toEqual([]);
    expect(page.text.some((text) => text.includes(expectedMarker))).toBe(true);
  });
});
