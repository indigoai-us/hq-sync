import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Source-contract regression guard for Sentry HQ-SYNC-WEB-3:
//
//   UnhandledRejection: Non-Error promise rejection captured with value:
//   Command plugin:window|start_dragging not allowed by ACL
//
// The Recent Changes window (label `activity-log`, built in
// commands/activity.rs, routed to ActivityLog.svelte in main.ts) renders its
// <header> as a Tauri drag region (data-tauri-drag-region). Tauri's drag region
// invokes the core `start_dragging` window command on mousedown — which is ONLY
// allowed when the window's capability grants `core:window:allow-start-dragging`.
// The window originally had no capability granting it (the old separate
// `notification-history` window was retired and its capability was orphaned),
// so every attempt to drag the window was denied by the ACL and bubbled up as
// an unhandled rejection to Sentry.
//
// These assertions pin the facts that, together, make the drag work and keep
// the rejection from recurring. They are source-contract checks (the unit
// suite never boots a real Tauri window), mirroring e2e/desktop-alt/titlebar-drag.spec.ts.

const root = (rel: string) => fileURLToPath(new URL(`../../${rel}`, import.meta.url));

const cap = JSON.parse(readFileSync(root('src-tauri/capabilities/activity-log.json'), 'utf8'));
const activityLog = readFileSync(root('src/components/ActivityLog.svelte'), 'utf8');
const mainTs = readFileSync(root('src/main.ts'), 'utf8');
const builder = readFileSync(root('src-tauri/src/commands/activity.rs'), 'utf8');
const retiredCapPath = root('src-tauri/capabilities/notification-history.json');

describe('HQ-SYNC-WEB-3: activity-log window drag capability', () => {
  it('targets the activity-log window', () => {
    expect(cap.windows).toContain('activity-log');
  });

  it('grants start-dragging (the ActivityLog drag region is inert + rejects without it)', () => {
    expect(cap.permissions).toContain('core:window:allow-start-dragging');
  });

  it('renders the ActivityLog header as a Tauri drag region', () => {
    expect(activityLog).toMatch(/<header[^>]*\bdata-tauri-drag-region\b/);
  });

  it('routes the activity-log window label to ActivityLog', () => {
    expect(mainTs).toMatch(/windowLabel === 'activity-log'[\s\S]*?Component = ActivityLog/);
  });

  it('builds the window under the same label the capability grants', () => {
    expect(builder).toMatch(/ACTIVITY_WINDOW_LABEL:\s*&str\s*=\s*"activity-log"/);
  });

  it('removes the orphaned notification-history capability (its window was retired)', () => {
    expect(existsSync(retiredCapPath)).toBe(false);
  });
});
