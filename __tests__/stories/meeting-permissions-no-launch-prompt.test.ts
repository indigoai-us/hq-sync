import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Regression guard: HQ Sync must NOT request any macOS permission when it is
 * installed or opened. Asking for Accessibility / Screen Recording /
 * Microphone / Notifications now lives exclusively behind Settings → Meeting
 * permissions (and the Settings "Enable notifications" button). A clean-room
 * VM opening the app for the first time should see zero permission dialogs.
 *
 * These are source-contract assertions (same style as the US-* story tests):
 * the launch path is wiring inside Tauri `.setup()` + the App `$effect`, which
 * can't be exercised headlessly without a real signed bundle, so we lock the
 * contract at the source level instead.
 */

const mainRs = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/main.rs'),
  'utf8',
);
const recallSdkRs = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/commands/recall_sdk.rs'),
  'utf8',
);
const appSvelte = readFileSync(
  resolve(process.cwd(), 'src/App.svelte'),
  'utf8',
);
const settingsSvelte = readFileSync(
  resolve(process.cwd(), 'src/components/Settings.svelte'),
  'utf8',
);
const wizardSvelte = readFileSync(
  resolve(process.cwd(), 'src/components/MeetingPermissionsWindow.svelte'),
  'utf8',
);

describe('No permission prompts on install / open', () => {
  it('does not fire the native TCC prompt (permissions_force_native_register) on launch', () => {
    // The prompting call (with parens) must not appear anywhere in main.rs —
    // it now belongs to the Settings wizard only. The command may still be
    // REGISTERED in the invoke_handler list (a bare path + comma, no parens).
    expect(mainRs).not.toContain('permissions_force_native_register()');
    expect(mainRs).toContain(
      'commands::permissions::permissions_force_native_register,',
    );
  });

  it('does not auto-open the meeting-permissions wizard on launch', () => {
    // No invocation of the wizard opener in main.rs's setup path. It remains
    // registered for the Settings button to call.
    expect(mainRs).not.toContain('open_meeting_permissions_window(');
    expect(mainRs).toContain(
      'commands::permissions::open_meeting_permissions_window,',
    );
  });

  it('only starts the Recall SDK on launch once required permissions are already granted', () => {
    // The launch path reads TCC status (prompt-less) and only starts the SDK
    // when all_required_granted — so the SDK never triggers prompts on a fresh
    // open.
    expect(mainRs).toContain('commands::permissions::meetings_permissions_state()');
    expect(mainRs).toContain('state.all_required_granted');
    expect(mainRs).toContain('commands::recall_sdk::start_recall_sdk(handle.clone())');
    // The gate must be evaluated before the SDK is started.
    const gateIdx = mainRs.indexOf('state.all_required_granted');
    const startIdx = mainRs.indexOf('start_recall_sdk(handle.clone())');
    expect(gateIdx).toBeGreaterThan(-1);
    expect(startIdx).toBeGreaterThan(gateIdx);
  });

  it('exposes start_recall_sdk as a Tauri command so Settings can start it after grant', () => {
    expect(recallSdkRs).toContain('#[tauri::command]\npub async fn start_recall_sdk');
    expect(mainRs).toContain('commands::recall_sdk::start_recall_sdk,');
  });

  it('does not request notification permission from App.svelte on mount', () => {
    expect(appSvelte).not.toContain('requestNotificationPermissionOnce');
    expect(appSvelte).not.toContain("'notification_request_permission'");
  });
});

describe('Settings remains the only place permissions are requested', () => {
  it('keeps the user-initiated notification request in Settings', () => {
    expect(settingsSvelte).toContain("'notification_request_permission'");
  });

  it('keeps the Settings entry point to the meeting-permissions wizard', () => {
    expect(settingsSvelte).toContain('open_meeting_permissions_window');
  });

  it('lets the wizard fire the native prompts on explicit user action', () => {
    expect(wizardSvelte).toContain("invoke('permissions_force_native_register')");
  });

  it('starts the SDK from the wizard once all required permissions are granted', () => {
    expect(wizardSvelte).toContain("invoke('start_recall_sdk')");
    expect(wizardSvelte).toContain('if (allGranted && !sdkStartAttempted)');
  });
});
