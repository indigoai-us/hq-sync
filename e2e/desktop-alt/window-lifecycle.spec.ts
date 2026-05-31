import { describe, expect, it } from 'vitest';
import { createDesktopAltHarness } from './live-driver';

describe('desktop-alt window lifecycle', () => {
  it('opens independently, closes without killing the popover or tray, and reopens', async () => {
    const app = await createDesktopAltHarness('qa@getindigo.ai');

    try {
      expect((await app.bootPopover()).toggleVisible).toBe(true);

      const firstWindow = await app.clickDesktopAltToggle();
      expect(firstWindow.created).toBe(true);
      expect(await app.snapshot()).toMatchObject({
        popoverAlive: true,
        trayAlive: true,
        desktopAltWindow: { id: firstWindow.id, focused: true },
      });

      await app.closeDesktopAltWindow();
      expect(await app.snapshot()).toEqual({
        popoverAlive: true,
        trayAlive: true,
        desktopAltWindow: null,
      });

      const reopenedWindow = await app.clickDesktopAltToggle();
      expect(reopenedWindow.created).toBe(true);
      expect(reopenedWindow.id).not.toBe(firstWindow.id);
      expect(await app.snapshot()).toMatchObject({
        popoverAlive: true,
        trayAlive: true,
        desktopAltWindow: { id: reopenedWindow.id, focused: true },
      });
    } finally {
      await app.dispose?.();
    }
  });

  it('focuses an existing desktop-alt window when the toggle is clicked again', async () => {
    const app = await createDesktopAltHarness('qa@getindigo.ai');

    try {
      const firstWindow = await app.clickDesktopAltToggle();
      const focusedWindow = await app.clickDesktopAltToggle();

      expect(focusedWindow.created).toBe(false);
      expect(focusedWindow.id).toBe(firstWindow.id);
      expect(focusedWindow.focused).toBe(true);
    } finally {
      await app.dispose?.();
    }
  });
});
