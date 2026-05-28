import { describe, expect, it } from 'vitest';
import { DesktopAltHarness } from './harness';

describe('desktop-alt gate visibility', () => {
  it('shows the popover toggle for a mocked Indigo email', () => {
    const app = new DesktopAltHarness('qa@getindigo.ai');

    expect(app.bootPopover().toggleVisible).toBe(true);
  });

  it('hides the popover toggle for a mocked non-Indigo email', () => {
    const app = new DesktopAltHarness('qa@example.com');

    expect(app.bootPopover().toggleVisible).toBe(false);
  });

  it('does not allow lookalike domains through the dogfood gate', () => {
    const app = new DesktopAltHarness('attacker@forgetindigo.ai');

    expect(app.bootPopover().toggleVisible).toBe(false);
  });
});
