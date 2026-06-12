import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const meetingsPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/MeetingsPage.svelte'),
  'utf8',
);
const meetingsStore = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/lib/meetings-store.svelte.ts'),
  'utf8',
);
const meetingsAgenda = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/MeetingsAgenda.svelte'),
  'utf8',
);
const liveNowCard = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/LiveNowCard.svelte'),
  'utf8',
);
const permissionWizard = readFileSync(
  resolve(process.cwd(), 'src/components/MeetingPermissionsWindow.svelte'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-015: Meetings in V4 remains gated and action-complete', () => {
  it('hides the V4 Meetings surface when the backend feature gate is off', () => {
    const page = normalize(meetingsPage);

    expect(meetingsPage).toContain("invoke<boolean>('meetings_feature_enabled')");
    expect(meetingsPage).toContain('meetingsFeatureEnabled = false');
    expect(meetingsPage).toContain('data-testid="meetings-feature-hidden"');
    expect(page).toContain(
      '<div class="meetings" class:hidden-by-gate={meetingsFeatureEnabled === false} aria-label="Meetings">',
    );
    expect(meetingsPage).toContain('.meetings.hidden-by-gate { display: none; }');
  });

  it('keeps invite, scheduled, join-now, recording, and stop states wired to existing meeting commands', () => {
    expect(meetingsStore).toContain("invoke<MeetingEvent[]>('meetings_list_upcoming')");
    expect(meetingsStore).toContain("invoke<ScheduledBot[]>('meetings_list_scheduled_bots'");
    expect(meetingsStore).toContain("invoke<ScheduledBot>('meetings_invite_bot'");
    expect(meetingsStore).toContain("await invoke('meetings_cancel_bot'");
    expect(meetingsStore).toContain("invoke<ScheduledBot>('meetings_join_bot_now'");

    expect(meetingsPage).toContain('const liveMeeting = $derived(pickLiveMeeting');
    expect(meetingsPage).toContain('activeRecordingsFromScheduledBots(events, botsByEventId)');
    expect(meetingsPage).toContain('onstart={startRecording}');
    expect(meetingsPage).toContain('onstop={stopRecording}');

    expect(meetingsAgenda).toContain("aria-label=\"Invite bot\"");
    expect(meetingsAgenda).toContain("aria-label=\"Uninvite bot\"");
    expect(meetingsAgenda).toContain("aria-label=\"Tell bot to join now\"");
    expect(meetingsAgenda).toContain("<span class=\"pill\">Scheduled</span>");
    expect(liveNowCard).toContain('Start recording');
    expect(liveNowCard).toContain('Stop recording');
    expect(liveNowCard).toContain('onclick={() => onstop(meeting.windowId)}');
  });

  it('keeps the permission wizard user-driven with TCC states and grant actions', () => {
    expect(permissionWizard).toContain("invoke('permissions_force_native_register')");
    expect(permissionWizard).toContain("invoke('permissions_open_settings'");
    expect(permissionWizard).toContain("invoke('start_recall_sdk')");
    expect(permissionWizard).toContain('snapshot.microphone');
    expect(permissionWizard).toContain('snapshot.screenCapture');
    expect(permissionWizard).toContain('snapshot.accessibility');
    expect(permissionWizard).toContain('snapshot.fullDiskAccess');
    expect(permissionWizard).toContain('Trigger prompts');
    expect(permissionWizard).toContain('Open Settings');
    expect(permissionWizard).toContain('Manage in Settings');
  });
});
