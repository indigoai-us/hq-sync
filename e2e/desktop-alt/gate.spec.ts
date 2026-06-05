import { describe, expect, it } from 'vitest';
import { DesktopAltHarness } from './harness';

// The expanded desktop window graduated from the Indigo-only dogfood to GA:
// the popover toggle is now visible for ANY signed-in user (non-empty email
// claim), regardless of domain, and hidden only when signed out. This mirrors
// the Rust gate `feature_gate::desktop_features_enabled` / `email_present`.
describe('desktop-alt gate visibility (GA)', () => {
  it('shows the popover toggle for an Indigo email', () => {
    const app = new DesktopAltHarness('qa@getindigo.ai');

    expect(app.bootPopover().toggleVisible).toBe(true);
  });

  it('shows the popover toggle for a non-Indigo email (GA)', () => {
    const app = new DesktopAltHarness('qa@example.com');

    expect(app.bootPopover().toggleVisible).toBe(true);
  });

  it('shows the popover toggle for the former dogfood look-alike (GA)', () => {
    // `attacker@forgetindigo.ai` was blocked under the Indigo dogfood gate;
    // under GA the gate only checks email presence, so it is now visible.
    const app = new DesktopAltHarness('attacker@forgetindigo.ai');

    expect(app.bootPopover().toggleVisible).toBe(true);
  });

  it('hides the popover toggle when signed out (no email)', () => {
    const app = new DesktopAltHarness('');

    expect(app.bootPopover().toggleVisible).toBe(false);
  });

  it('hides the popover toggle when the email is whitespace-only', () => {
    const app = new DesktopAltHarness('   ');

    expect(app.bootPopover().toggleVisible).toBe(false);
  });
});
