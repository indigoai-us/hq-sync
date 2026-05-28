import { describe, expect, it } from 'vitest';
import { DesktopAltHarness } from './harness';

describe('desktop-alt window lifecycle', () => {
  it('opens independently, closes without killing the popover or tray, and reopens', () => {
    const app = new DesktopAltHarness('qa@getindigo.ai');
    expect(app.bootPopover().toggleVisible).toBe(true);

    const firstWindow = app.clickDesktopAltToggle();
    expect(firstWindow.created).toBe(true);
    expect(app.snapshot()).toMatchObject({
      popoverAlive: true,
      trayAlive: true,
      desktopAltWindow: { id: firstWindow.id, focused: true },
    });

    app.closeDesktopAltWindow();
    expect(app.snapshot()).toEqual({
      popoverAlive: true,
      trayAlive: true,
      desktopAltWindow: null,
    });

    const reopenedWindow = app.clickDesktopAltToggle();
    expect(reopenedWindow.created).toBe(true);
    expect(reopenedWindow.id).not.toBe(firstWindow.id);
    expect(app.snapshot()).toMatchObject({
      popoverAlive: true,
      trayAlive: true,
      desktopAltWindow: { id: reopenedWindow.id, focused: true },
    });
  });

  it('focuses an existing desktop-alt window when the toggle is clicked again', () => {
    const app = new DesktopAltHarness('qa@getindigo.ai');

    const firstWindow = app.clickDesktopAltToggle();
    const focusedWindow = app.clickDesktopAltToggle();

    expect(focusedWindow.created).toBe(false);
    expect(focusedWindow.id).toBe(firstWindow.id);
    expect(focusedWindow.focused).toBe(true);
  });
});
