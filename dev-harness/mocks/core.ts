// Mock of @tauri-apps/api/core for the browser preview harness.
// Returns plausible fixture data per command so components mount and render
// without a Tauri backend. Design-only: no real side effects.

const settings = {
  hqPath: '/Users/corey/Documents/HQ',
  syncOnLaunch: false,
  notifications: true,
  startAtLogin: true,
  realtimeSync: true,
  personalSyncEnabled: true,
  instantSync: true,
  shareNotifications: true,
  dmNotifications: true,
  stagingChannel: true,
  releaseChannel: null as string | null,
};

type Handler = (args?: Record<string, unknown>) => unknown;

const handlers: Record<string, Handler> = {
  get_settings: () => ({ ...settings }),
  save_settings: (args) => {
    const prefs = (args?.prefs ?? {}) as Partial<typeof settings>;
    Object.assign(settings, prefs);
    return null;
  },
  get_sync_status: () => ({
    lastSyncAt: new Date(Date.now() - 7 * 60 * 1000).toISOString(),
    pendingFiles: 0,
    conflicts: 0,
    daemonRunning: true,
    source: 'menubar',
  }),
  get_autostart_enabled: () => true,
  set_autostart_enabled: () => null,
  meetings_feature_enabled: () => true,
  available_channels: () => ['stable', 'beta', 'alpha'],
  notification_permission_state: () => 'prompt',
  notification_request_permission: () => 'granted',
  pick_folder: () => null,
  check_for_updates: () => null,
  start_daemon: () => null,
  stop_daemon: () => null,
  daemon_status: () => ({ running: true }),
  open_activity_log: () => null,
  open_meetings_window: () => null,
  open_drift_detail: () => null,
  quit_app: () => null,
};

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const handler = handlers[cmd];
  if (handler) return handler(args) as T;
  // Unknown command: log once and resolve null so mount paths don't throw.
  console.debug('[harness] unhandled invoke:', cmd, args);
  return null as T;
}

export class Channel<T = unknown> {
  onmessage: ((msg: T) => void) | null = null;
}
