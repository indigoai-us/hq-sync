// Mock of @tauri-apps/api/core for the browser preview harness.
// Returns plausible fixture data per command so components mount and render
// without a Tauri backend. Design-only: no real side effects.
import type { Workspace } from '../../src/lib/workspaces';

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
  { id: 'in-proj-201', title: 'Event-driven HQ-Cloud sync', description: 'Push-based sync — drop the 60s poll for instant fan-out.', company: 'indigo', status: 'active', prdPath: 'companies/indigo/projects/event-driven-hq-cloud-sync/prd.json', createdAt: '2026-06-01T00:00:00Z', updatedAt: '2026-06-12T00:00:00Z', storyCount: 8, storiesComplete: 3 },
  { id: 'in-proj-202', title: 'S3-versioned conflict handling', description: 'Use S3 object versions to resolve concurrent edits.', company: 'indigo', status: 'in_progress', prdPath: 'companies/indigo/projects/hq-sync-conflict-versioning/prd.json', createdAt: '2026-06-02T00:00:00Z', updatedAt: '2026-06-13T00:00:00Z', storyCount: 6, storiesComplete: 2 },
  { id: 'in-proj-203', title: 'Browse vs Sync — role-aware sharing', description: 'Let viewers browse a vault without a full local sync.', company: 'indigo', status: 'in_progress', prdPath: 'companies/indigo/projects/hq-sync-browse-vs-sync/prd.json', createdAt: '2026-06-03T00:00:00Z', updatedAt: '2026-06-11T00:00:00Z', storyCount: 5, storiesComplete: 1 },
  { id: 'in-proj-125', title: 'HQ Sync Desktop — Flagship Company OS', description: 'Top-level Board, Projects port, actionable surfaces.', company: 'indigo', status: 'completed', prdPath: 'companies/indigo/projects/hq-sync-desktop-flagship/prd.json', createdAt: '2026-05-30T00:00:00Z', updatedAt: '2026-06-09T00:00:00Z', storyCount: 12, storiesComplete: 12 },
  { id: 'in-proj-204', title: 'Instant DM delivery', description: 'MQTT-over-WSS wake signal for sub-second DMs.', company: 'indigo', status: 'completed', prdPath: 'companies/indigo/projects/instant-dm-delivery/prd.json', createdAt: '2026-06-04T00:00:00Z', updatedAt: '2026-06-10T00:00:00Z', storyCount: 5, storiesComplete: 5 },
  { id: 'in-proj-205', title: 'Meeting detect + notify', description: 'Clickable detected-meeting notifications + permissions wizard.', company: 'indigo', status: 'prd_created', prdPath: 'companies/indigo/projects/meeting-detect-notify/prd.json', createdAt: '2026-06-05T00:00:00Z', updatedAt: '2026-06-08T00:00:00Z', storyCount: 7, storiesComplete: 0 },
  { id: 'in-proj-206', title: 'S3 → Laptop Live Sync', description: 'Continuous background sync without manual triggers.', company: 'indigo', status: 'exploring', prdPath: null, createdAt: '2026-06-06T00:00:00Z', updatedAt: '2026-06-07T00:00:00Z', storyCount: 0, storiesComplete: 0 },
];

const LIBRARY_ROOT = {
  workers: [
    { id: 'architect', name: 'Architect', type: 'CodeWorker', description: 'Surface architecture tradeoffs and deep-module opportunities.', scope: 'root', status: 'active', path: 'core/workers/public/dev-team/architect/', team: 'dev-team' },
    { id: 'frontend-dev', name: 'Frontend Dev', type: 'CodeWorker', description: 'Build polished desktop and web interfaces.', scope: 'root', status: 'active', path: 'core/workers/public/dev-team/frontend-dev/', team: 'dev-team' },
    { id: 'paper-designer', name: 'Paper Designer', type: 'DesignWorker', description: 'Translate Paper references into implementable UI systems.', scope: 'root', status: 'active', path: 'core/workers/public/paper-designer/', team: 'design' },
    { id: 'indigo-cmo', name: 'Indigo CMO', type: 'OpsWorker', description: 'Company-scoped go-to-market planning.', scope: 'company', company: 'indigo', status: 'active', path: 'companies/indigo/workers/cmo/' },
    { id: 'liverecover-analyst', name: 'Liverecover Analyst', type: 'ResearchWorker', description: 'Analyze local company signals and market notes.', scope: 'company', company: 'liverecover', status: 'active', path: 'companies/liverecover/workers/analyst/' },
  ],
  skills: [
    { name: 'startwork', description: 'Start a work session from local HQ context.', scope: 'root', path: '.claude/skills/startwork/SKILL.md', allowedTools: ['Read', 'Bash'] },
    { name: 'plan', description: 'Create an execution-ready project plan.', scope: 'root', path: '.claude/skills/plan/SKILL.md', allowedTools: ['Read', 'Write'] },
    { name: 'search', description: 'Search across HQ knowledge and projects.', scope: 'root', path: '.claude/skills/search/SKILL.md', allowedTools: ['Read', 'Bash'] },
    { name: 'signals', description: 'Read company signals and action items.', scope: 'company', company: 'indigo', path: 'companies/indigo/skills/signals/SKILL.md', allowedTools: ['Read'] },
  ],
};

const LIBRARY_COMPANY = {
  workers: LIBRARY_ROOT.workers.filter((worker) => worker.company === 'indigo'),
  skills: LIBRARY_ROOT.skills.filter((skill) => skill.company === 'indigo'),
};

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

function minutesAgo(mins: number): string {
  return new Date(Date.now() - mins * 60 * 1000).toISOString();
}

function workspace(overrides: Partial<Workspace>): Workspace {
  return {
    slug: 'indigo',
    displayName: 'Indigo',
    kind: 'company',
    state: 'synced',
    cloudUid: 'cmp_indigo',
    bucketName: 'hq-vault-indigo',
    hasLocalFolder: true,
    localPath: '/Users/corey/Documents/HQ/companies/indigo',
    membershipStatus: 'active',
    role: 'owner',
    lastSyncedAt: minutesAgo(7),
    brokenReason: null,
    invitedBy: null,
    invitedAt: null,
    ...overrides,
  };
}

const HARNESS_WORKSPACES: Workspace[] = [
  workspace({
    slug: 'personal',
    displayName: 'Corey Epstein',
    kind: 'personal',
    state: 'personal',
    cloudUid: 'cmp_personal',
    bucketName: 'hq-vault-personal',
    localPath: '/Users/corey/Documents/HQ/personal',
    role: null,
    lastSyncedAt: minutesAgo(3),
  }),
  workspace({}),
  workspace({
    slug: 'liverecover',
    displayName: 'Liverecover',
    cloudUid: 'cmp_liverecover',
    bucketName: 'hq-vault-liverecover',
    localPath: '/Users/corey/Documents/HQ/companies/liverecover',
    role: 'member',
    lastSyncedAt: minutesAgo(18),
  }),
  workspace({
    slug: 'moonflow',
    displayName: 'Moonflow',
    cloudUid: 'cmp_moonflow',
    bucketName: 'hq-vault-moonflow',
    localPath: '/Users/corey/Documents/HQ/companies/moonflow',
    role: 'admin',
    lastSyncedAt: minutesAgo(41),
  }),
  workspace({
    slug: 'westbound',
    displayName: 'Westbound',
    state: 'cloud-only',
    cloudUid: 'cmp_westbound',
    bucketName: 'hq-vault-westbound',
    hasLocalFolder: false,
    localPath: null,
    role: 'member',
    lastSyncedAt: null,
  }),
  workspace({
    slug: 'holler-mgmt',
    displayName: 'Holler Mgmt',
    state: 'local-only',
    cloudUid: null,
    bucketName: null,
    localPath: '/Users/corey/Documents/HQ/companies/holler-mgmt',
    membershipStatus: null,
    role: null,
    lastSyncedAt: null,
  }),
  workspace({
    slug: 'newco',
    displayName: 'New Co',
    state: 'local-only',
    cloudUid: null,
    bucketName: null,
    localPath: '/Users/corey/Documents/HQ/companies/newco',
    membershipStatus: null,
    role: null,
    lastSyncedAt: null,
  }),
  workspace({
    slug: 'sender-agency',
    displayName: 'Sender Agency',
    state: 'cloud-only',
    cloudUid: 'cmp_sender',
    bucketName: 'hq-vault-sender',
    hasLocalFolder: false,
    localPath: null,
    membershipStatus: 'pending',
    role: null,
    lastSyncedAt: null,
    invitedBy: 'maya@getindigo.ai',
    invitedAt: minutesAgo(60 * 25),
  }),
  workspace({
    slug: 'archive-labs',
    displayName: 'Archive Labs',
    state: 'broken',
    cloudUid: 'cmp_archive_old',
    bucketName: 'hq-vault-archive-old',
    localPath: '/Users/corey/Documents/HQ/companies/archive-labs',
    role: 'member',
    lastSyncedAt: null,
    brokenReason: 'manifest cloud_uid does not match the current vault membership',
  }),
  ...Array.from({ length: 15 }, (_, index) =>
    workspace({
      slug: `local-company-${index + 1}`,
      displayName: `Local Company ${index + 1}`,
      state: 'local-only',
      cloudUid: null,
      bucketName: null,
      hasLocalFolder: index % 3 !== 0,
      localPath: `/Users/corey/Documents/HQ/companies/local-company-${index + 1}`,
      membershipStatus: null,
      role: null,
      lastSyncedAt: null,
    }),
  ),
];

const handlers: Record<string, Handler> = {
  // Company-board path (?view=company)
  list_syncable_workspaces: () => ({
    workspaces: HARNESS_WORKSPACES,
    cloudReachable: true,
    error: null,
    hqFolderPath: '/Users/corey/Documents/HQ',
    manifestError: null,
  }),
  connect_workspace_to_cloud: () => null,
  get_sync_mode: (args) => ({
    syncMode: args?.companySlug === 'liverecover' ? 'shared' : 'all',
  }),
  get_config: () => ({ hqFolderPath: '/Users/corey/Documents/HQ', companySlug: 'indigo', configured: true }),
  get_library_root: () => LIBRARY_ROOT,
  get_library_company: () => LIBRARY_COMPANY,
  get_library_worker_detail: (args) => {
    const path = String(args?.workerPath ?? '');
    const worker = LIBRARY_ROOT.workers.find((item) => item.path === path) ?? LIBRARY_ROOT.workers[0];
    return {
      id: worker.id,
      name: worker.name,
      type: worker.type,
      description: worker.description,
      team: worker.team ?? null,
      skills: [{ name: 'startwork', description: 'Load the right company and repo context.' }],
      instructions: `# ${worker.name}\n\nUse local HQ files first and keep work scoped to the selected company.`,
    };
  },
  get_library_skill_detail: (args) => {
    const path = String(args?.skillPath ?? '');
    const skill = LIBRARY_ROOT.skills.find((item) => item.path === path) ?? LIBRARY_ROOT.skills[0];
    return {
      name: skill.name,
      description: skill.description,
      allowedTools: skill.allowedTools,
      body: `# ${skill.name}\n\nRepresentative harness detail for ${skill.description}`,
    };
  },
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
  // Notifications feed (the popover's default body since the redesign). Returns
  // a representative mix of DMs, shares, and cross-session new-file rows so the
  // inline feed renders fully in the browser harness (?view=popover).
  fetch_notification_history: () => {
    const iso = (minsAgo: number) => new Date(Date.now() - minsAgo * 60 * 1000).toISOString();
    return {
      dms: [
        {
          eventId: 'dm-1',
          fromPersonUid: 'prs_grace',
          fromDisplayName: 'Maya Chen',
          fromEmail: 'grace@getindigo.ai',
          body: 'Pushed the conflict-versioning notes — take a look when you get a sec?',
          createdAt: iso(6),
        },
        {
          eventId: 'dm-2',
          fromPersonUid: 'prs_alan',
          fromDisplayName: 'Sam Rivera',
          fromEmail: 'alan@example.com',
          body: 'Meeting recap is synced to the Liverecover folder.',
          createdAt: iso(48),
        },
      ],
      shares: [
        {
          eventId: 'share-1',
          issuerDisplayName: 'Jacob Patel',
          issuerEmail: 'jacob@getindigo.ai',
          paths: ['companies/indigo/financials/Q2-model.xlsx'],
          note: 'Latest forecast for review',
          createdAt: iso(20),
        },
      ],
      files: [
        {
          eventId: 'file-1',
          path: 'companies/indigo/knowledge/prd.json',
          bytes: 4096,
          addedBy: 'maya@getindigo.ai',
          companySlug: 'indigo',
          createdAt: iso(40),
        },
        {
          eventId: 'file-2',
          path: 'companies/indigo/policies/e2e-testing.md',
          bytes: 5120,
          addedBy: 'maya@getindigo.ai',
          companySlug: 'indigo',
          createdAt: iso(75),
        },
      ],
    };
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

  // -------------------------------------------------------------------------
  // Messaging window fixtures (US-008→011) — representative DMs, a thread, and
  // pending requests so the Messages window renders populated in the harness.
  // -------------------------------------------------------------------------
  get_unread_summary: () => ({ unreadDms: 2, pendingRequests: 2 }),
  list_contacts: () => ({
    contacts: [
      { personUid: 'prs_ada', email: 'ada@getindigo.ai', displayName: 'Ada Lovelace', companyUid: 'cmp_indigo', source: 'company', lastMessageAt: '2026-06-09T19:43:10.000Z', lastMessageBody: 'Please do — I’m restyling it to match the desktop view right now.', lastMessageDirection: 'out' },
      { personUid: 'prs_grace', email: 'grace@getindigo.ai', displayName: 'Grace Hopper', companyUid: 'cmp_indigo', source: 'company' },
      { personUid: 'prs_alan', email: 'alan@example.com', displayName: 'Alan Turing', companyUid: null, source: 'connection' },
      { personUid: 'prs_katherine', email: 'katherine@getindigo.ai', displayName: 'Katherine Johnson', companyUid: 'cmp_indigo', source: 'company', lastMessageAt: '2026-06-08T19:43:10.000Z' },
    ],
  }),
  list_company_members: () => ({
    members: [
      { personUid: 'prs_grace', email: 'grace@getindigo.ai', displayName: 'Grace Hopper', companyUid: 'cmp_indigo', companyName: 'Indigo' },
      { personUid: 'prs_katherine', email: 'katherine@getindigo.ai', displayName: 'Katherine Johnson', companyUid: 'cmp_indigo', companyName: 'Indigo' },
    ],
  }),
  fetch_dm_thread: (args) => {
    const peer = String(args?.withPersonUid ?? 'prs_ada');
    const people: Record<string, { name: string; email: string; latest: string }> = {
      prs_ada: {
        name: 'Ada Lovelace',
        email: 'ada@getindigo.ai',
        latest: 'Please do — I’m restyling it to match the desktop view right now.',
      },
      prs_grace: {
        name: 'Grace Hopper',
        email: 'grace@getindigo.ai',
        latest: 'Pushed the conflict-versioning notes — take a look when you get a sec?',
      },
      prs_alan: {
        name: 'Alan Turing',
        email: 'alan@example.com',
        latest: 'Meeting recap is synced to the Liverecover folder.',
      },
      prs_katherine: {
        name: 'Katherine Johnson',
        email: 'katherine@getindigo.ai',
        latest: 'Orbit math notes are ready for review.',
      },
    };
    const person = people[peer] ?? people.prs_ada;
    return {
      messages: [
        { eventId: 'm1', fromPersonUid: peer, fromDisplayName: person.name, fromEmail: person.email, body: 'Hey — did the Phase 1 backend land in prod?', createdAt: '2026-06-09T19:40:00.000Z', direction: 'in' },
        { eventId: 'm2', fromPersonUid: 'prs_me', fromDisplayName: 'You', fromEmail: 'me@coreyepstein.com', body: 'Yep, just went live. Connection routes are up and the send path is verified.', createdAt: '2026-06-09T19:41:00.000Z', direction: 'out' },
        { eventId: 'm3', fromPersonUid: peer, fromDisplayName: person.name, fromEmail: person.email, body: 'Amazing. Want me to take the Messages window for a spin?', createdAt: '2026-06-09T19:42:30.000Z', direction: 'in' },
        { eventId: 'm4', fromPersonUid: peer === 'prs_katherine' ? peer : 'prs_me', fromDisplayName: peer === 'prs_katherine' ? person.name : 'You', fromEmail: peer === 'prs_katherine' ? person.email : 'me@coreyepstein.com', body: person.latest, createdAt: '2026-06-09T19:43:10.000Z', direction: peer === 'prs_katherine' ? 'in' : 'out' },
      ],
      nextCursor: null,
    };
  },
  list_dm_requests: () => ({
    requests: [
      { pairKey: 'pk1', fromPersonUid: 'prs_lin', fromEmail: 'lin@northwind.co', fromDisplayName: 'Lin Manuel', message: 'Hi! We met at the HQ demo — would love to connect here.', sharedCompany: null, createdAt: '2026-06-09T18:10:00.000Z' },
      { pairKey: 'pk2', fromPersonUid: 'prs_rao', fromEmail: 'rao@getindigo.ai', fromDisplayName: 'Rao Patel', message: 'Pinging you about the messaging rollout.', sharedCompany: 'Indigo', createdAt: '2026-06-09T17:05:00.000Z' },
    ],
  }),
  send_dm: () => ({ eventId: 'sent-1', createdAt: '2026-06-09T19:44:00.000Z' }),
  send_dm_to_email: () => ({ state: 'connection_requested' }),
  respond_dm_request: () => null,
  messages_window_ready: () => null,
  open_messages_window: () => null,
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

// Some Tauri plugins (e.g. plugin-notification) import these from core. The
// harness has no backend, so they resolve to inert no-ops.
export class PluginListener {
  constructor(
    public plugin: string,
    public event: string,
    public channelId: number,
  ) {}
  async unregister(): Promise<void> {}
}

export async function addPluginListener<T>(
  _plugin: string,
  _event: string,
  _cb: (payload: T) => void,
): Promise<PluginListener> {
  return new PluginListener(_plugin, _event, 0);
}

export function transformCallback(_cb?: (response: unknown) => void, _once = false): number {
  return 0;
}

export function convertFileSrc(filePath: string, _protocol = 'asset'): string {
  return filePath;
}
