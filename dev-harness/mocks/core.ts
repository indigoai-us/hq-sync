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

// ---------------------------------------------------------------------------
// Company-board fixtures (representative Indigo data) — for the ?view=company
// harness. Wire shapes mirror the US-003/US-011 Rust commands.
// ---------------------------------------------------------------------------

const COMPANY_GOALS = {
  objectives: [
    {
      id: 'in-obj-001',
      title: 'Desktop Experience',
      description: 'Native macOS/desktop apps that make HQ feel like a product, not a CLI.',
      status: 'on_track',
      timeframe: '2026',
      owner: null,
      initiativeIds: ['in-init-001'],
      keyResults: [
        { id: 'kr-1', title: 'Desktop weekly active users', target: 500, current: 310 },
        { id: 'kr-2', title: 'Popover open latency budget met', target: 100, current: 100 },
      ],
    },
    {
      id: 'in-obj-002',
      title: 'Platform Stability',
      description: 'Sync reliability and zero data loss across every workspace.',
      status: 'on_track',
      timeframe: '2026',
      owner: null,
      initiativeIds: ['in-init-002'],
      keyResults: [],
    },
    {
      id: 'in-obj-003',
      title: 'AI Features',
      description: 'Agentic workflows woven across the HQ surface.',
      status: 'at_risk',
      timeframe: '2026',
      owner: null,
      initiativeIds: ['in-init-003'],
      keyResults: [],
    },
    {
      id: 'in-obj-004',
      title: 'Go-to-Market',
      description: 'Grow HQ adoption beyond the dogfood team.',
      status: 'on_track',
      timeframe: '2026',
      owner: null,
      initiativeIds: ['in-init-004'],
      keyResults: [],
    },
  ],
  initiatives: [
    { id: 'in-init-001', title: 'Desktop Experience', description: '', status: 'active' },
    { id: 'in-init-002', title: 'Platform Stability', description: '', status: 'active' },
  ],
};

const COMPANY_PROJECTS = [
  { id: 'in-proj-201', title: 'Event-driven HQ-Cloud sync', description: 'Push-based sync — drop the 60s poll for instant fan-out.', company: 'indigo', status: 'active', prdPath: 'companies/indigo/projects/event-driven-hq-cloud-sync/prd.json', storyCount: 8, storiesComplete: 3 },
  { id: 'in-proj-202', title: 'S3-versioned conflict handling', description: 'Use S3 object versions to resolve concurrent edits.', company: 'indigo', status: 'in_progress', prdPath: 'companies/indigo/projects/hq-sync-conflict-versioning/prd.json', storyCount: 6, storiesComplete: 2 },
  { id: 'in-proj-203', title: 'Browse vs Sync — role-aware sharing', description: 'Let viewers browse a vault without a full local sync.', company: 'indigo', status: 'in_progress', prdPath: 'companies/indigo/projects/hq-sync-browse-vs-sync/prd.json', storyCount: 5, storiesComplete: 1 },
  { id: 'in-proj-125', title: 'HQ Sync Desktop — Flagship Company OS', description: 'Top-level Board, Projects port, actionable surfaces.', company: 'indigo', status: 'completed', prdPath: 'companies/indigo/projects/hq-sync-desktop-flagship/prd.json', storyCount: 12, storiesComplete: 12 },
  { id: 'in-proj-204', title: 'Instant DM delivery', description: 'MQTT-over-WSS wake signal for sub-second DMs.', company: 'indigo', status: 'completed', prdPath: 'companies/indigo/projects/instant-dm-delivery/prd.json', storyCount: 5, storiesComplete: 5 },
  { id: 'in-proj-205', title: 'Meeting detect + notify', description: 'Clickable detected-meeting notifications + permissions wizard.', company: 'indigo', status: 'prd_created', prdPath: 'companies/indigo/projects/meeting-detect-notify/prd.json', storyCount: 7, storiesComplete: 0 },
  { id: 'in-proj-206', title: 'S3 → Laptop Live Sync', description: 'Continuous background sync without manual triggers.', company: 'indigo', status: 'exploring', prdPath: null, storyCount: 0, storiesComplete: 0 },
];

// PRDs keyed by prdPath — enough stories for classifyStories to surface an
// in-progress one (its title shows on the in-flight row + drill-in Kanban).
function prdFor(name: string, current: string, done: number, total: number) {
  const stories = [] as Array<Record<string, unknown>>;
  for (let i = 0; i < total; i++) {
    stories.push({
      id: `US-${String(i + 1).padStart(3, '0')}`,
      title: i === done ? current : i < done ? `Completed step ${i + 1}` : `Planned step ${i + 1}`,
      description: '',
      acceptanceCriteria: ['Behaves as specified', 'Has a regression test'],
      passes: i < done,
      priority: i < 2 ? '1' : '2',
      labels: i < done ? ['done'] : ['todo'],
      dependsOn: i > 0 ? [`US-${String(i).padStart(3, '0')}`] : [],
    });
  }
  return { name, description: '', branchName: null, userStories: stories, metadata: {} };
}

const COMPANY_PRDS: Record<string, unknown> = {
  'companies/indigo/projects/event-driven-hq-cloud-sync/prd.json': prdFor('Event-driven HQ-Cloud sync', 'Wire the IoT push receiver into the sync loop', 3, 8),
  'companies/indigo/projects/hq-sync-conflict-versioning/prd.json': prdFor('S3-versioned conflict handling', 'Resolve conflicts from the S3 version cursor', 2, 6),
  'companies/indigo/projects/hq-sync-browse-vs-sync/prd.json': prdFor('Browse vs Sync', 'Add the role-aware browse-only vault view', 1, 5),
};

type Handler = (args?: Record<string, unknown>) => unknown;

const handlers: Record<string, Handler> = {
  // Company-board path (?view=company)
  list_syncable_workspaces: () => ({
    workspaces: [
      { slug: 'personal', displayName: 'Corey Epstein', kind: 'personal', state: 'personal', hasLocalFolder: true },
      { slug: 'indigo', displayName: 'Indigo', kind: 'company', state: 'synced', hasLocalFolder: true },
      { slug: 'liverecover', displayName: 'Liverecover', kind: 'company', state: 'synced', hasLocalFolder: true },
    ],
    cloudReachable: true,
    error: null,
    hqFolderPath: '/Users/corey/Documents/HQ',
  }),
  get_config: () => ({ hqFolderPath: '/Users/corey/Documents/HQ', companySlug: 'indigo', configured: true }),
  get_company_summary: () => ({ board: 7, activity: { last7d: 34 }, deployments: 3, secrets: 12 }),
  get_local_company_goals: () => COMPANY_GOALS,
  get_local_projects: () => COMPANY_PROJECTS,
  get_local_project_prd: (args) =>
    COMPANY_PRDS[(args?.prdPath as string) ?? ''] ?? prdFor('Project', 'Current step in progress', 1, 4),
  get_local_project_readme: () => '# Project\n\nA representative README for the preview harness.',
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
  get_activity_log: () => {
    const now = Date.now();
    return [
      { company: 'indigo', path: 'companies/indigo/knowledge/prd.json', bytes: 4096, direction: 'down', author: 'maya@getindigo.ai', isNew: true, at: now - 40 * 1000 },
      { company: 'indigo', path: 'companies/indigo/projects/event-driven/notes.md', bytes: 2210, direction: 'down', author: 'corey@getindigo.ai', isNew: false, at: now - 3 * 60 * 1000 },
      { company: 'liverecover', path: 'companies/liverecover/sources/meetings/2026-06-04.md', bytes: 18400, direction: 'down', author: 'sam@liverecover.com', isNew: true, at: now - 9 * 60 * 1000 },
      { company: 'personal', path: 'personal/projects/redesign/sketch.md', bytes: 980, direction: 'up', author: undefined, isNew: false, at: now - 14 * 60 * 1000 },
      { company: 'indigo', path: 'companies/indigo/policies/e2e-testing.md', bytes: 5120, direction: 'down', author: 'maya@getindigo.ai', isNew: false, at: now - 22 * 60 * 1000 },
    ];
  },
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
  // Meeting-permissions wizard (?view=permissions) — a representative
  // not-yet-granted snapshot so the friendly "why we ask" notice is exercised.
  meeting_detect_feature_enabled: () => true,
  meetings_permissions_state: () => ({
    accessibility: 'prompt',
    screenCapture: 'denied',
    microphone: 'prompt',
    fullDiskAccess: 'prompt',
    allRequiredGranted: false,
  }),
  permissions_open_settings: () => null,
  permissions_force_native_register: () => null,
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
