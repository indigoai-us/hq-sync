// Fixture props for rendering Popover in the browser preview harness.
import type { Workspace } from '../src/lib/workspaces';

const minsAgo = (mins: number) => new Date(Date.now() - mins * 60 * 1000).toISOString();

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
    lastSyncedAt: minsAgo(7),
    brokenReason: null,
    invitedBy: null,
    invitedAt: null,
    ...overrides,
  };
}

export const workspaces: Workspace[] = [
  workspace({
    slug: 'personal',
    displayName: 'Personal',
    kind: 'personal',
    state: 'personal',
    cloudUid: 'cmp_personal',
    bucketName: 'hq-vault-personal',
    localPath: '/Users/corey/Documents/HQ/personal',
    role: null,
    lastSyncedAt: minsAgo(3),
  }),
  workspace({}),
  workspace({
    slug: 'liverecover',
    displayName: 'LiveRecover',
    state: 'synced',
    cloudUid: 'cmp_liverecover',
    bucketName: 'hq-vault-liverecover',
    localPath: '/Users/corey/Documents/HQ/companies/liverecover',
    role: 'member',
    lastSyncedAt: minsAgo(18),
  }),
  workspace({
    slug: 'moonflow',
    displayName: 'Moonflow',
    cloudUid: 'cmp_moonflow',
    bucketName: 'hq-vault-moonflow',
    localPath: '/Users/corey/Documents/HQ/companies/moonflow',
    role: 'admin',
    lastSyncedAt: minsAgo(41),
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
    invitedAt: minsAgo(60 * 25),
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
];

export const coreState = {
  channel: 'staging' as const,
  targetRepo: 'indigoai-us/hq-core-staging',
  targetVersion: '15.0.1',
  targetRef: 'staging',
  localVersion: '15.0.1',
  floorSha: null,
  isEligible: true,
  versionBehind: true,
  driftReport: {
    count: 14,
    modified: [],
    missing: [],
    added: [],
    scannedAt: new Date().toISOString(),
    hqVersion: '15.0.1',
    targetRepo: 'indigoai-us/hq-core-staging',
    targetRef: 'staging',
  },
  unchangedCount: 1200,
  userOnlyCount: 30,
  scannedAt: new Date().toISOString(),
};

export const popoverProps = {
  syncState: 'idle' as const,
  config: {
    configured: true,
    companySlug: 'indigo',
    hqFolderPath: '/Users/corey/Documents/HQ',
  },
  workspaces,
  cloudReachable: true,
  cloudError: null,
  manifestError: null,
  lastSummary: {
    companiesAttempted: 3,
    filesDownloaded: 12,
    bytesDownloaded: 348_000,
    filesSkipped: 2,
  },
  conflicts: [],
  showConflictModal: false,
  updateAvailable: null,
  updateInstalling: false,
  hqCliUpdateAvailable: null,
  hqCliUpdateInstalling: false,
  hqCliUpdateError: null,
  hqVersion: '15.0.1',
  coreState,
  coreInstalling: false,
  coreInstallLastResult: null,
  meetingsEnabled: true,
  // Indigo-gated header extras, on by default in the harness so the full
  // header (meeting + desktop-view + Sync) is previewable. One detected
  // meeting exercises the monochrome "detected" state.
  desktopAltEnabled: true,
  activeMeetings: [
    {
      windowId: 'w1',
      platform: 'zoom',
      meetingUrl: 'https://zoom.us/j/1',
      detectedAt: '2026-06-01T09:00:00Z',
      state: 'detected' as const,
      companyUid: null,
    },
  ],
  onsync: () => console.debug('[harness] sync'),
  oncancel: () => console.debug('[harness] cancel'),
  onsettings: () => (window.location.search = '?view=settings'),
  onsignout: () => console.debug('[harness] signout'),
  onresolve: () => {},
  onopen: () => {},
  ondismissconflicts: () => {},
  oninstallupdate: () => {},
  oninstallhqcliupdate: () => {},
  ondismisshqcliupdate: () => console.debug('[harness] dismiss hq cli update'),
  oninstallcore: () => console.debug('[harness] install core'),
  onworkspacesrefresh: () => {},
  bindStatsRefresh: () => {},
  onmeetingsclick: () => {},
};

/**
 * A stale-CLI fixture for the dev-harness `?view=popover&state=cli-update`
 * preview — drives the "CLI update available" banner (copyable one-liner +
 * dismiss ×). `local` is deliberately behind `latest` so the "You're on vX"
 * line renders. Mirrors the real `HqCliUpdateInfo` shape (`{ local, latest }`).
 */
export const hqCliUpdateAvailable = {
  local: '5.38.2',
  latest: '5.41.0',
};

/**
 * Banner-notification fixtures for `?view=banner&kind=...`. Shapes mirror the
 * Rust `BannerPayload` (camelCase) so the harness preview matches production.
 */
export const bannerFixtures: Record<string, Record<string, unknown>> = {
  share: {
    kind: 'share',
    title: 'Stefan Schmidt',
    body: 'Sharing the Q1 forecast — take a look before our sync.',
    iconText: '●',
    actionLabel: 'Open',
    actionId: 'open',
    clickActionId: 'open',
    data: {},
  },
  meeting: {
    kind: 'meeting',
    title: 'Zoom meeting detected',
    body: 'Zoom: Weekly sync',
    iconText: '●',
    actionLabel: 'Record',
    actionId: 'record',
    clickActionId: 'open',
    data: { windowId: 'preview-window-1', platform: 'zoom' },
  },
  dm: {
    kind: 'dm',
    title: 'Corey Epstein',
    body: 'Can you review the notification banner change when you get a sec?',
    iconText: '●',
    actionLabel: 'Copy prompt',
    actionId: 'copy',
    clickActionId: 'open',
    data: {},
  },
  update: {
    kind: 'update',
    title: 'New version',
    body: 'Version 0.4.4 is ready — custom HQ-branded notification banners.',
    iconText: '⬆',
    actionLabel: 'Update now',
    actionId: 'update',
    clickActionId: 'open',
    data: { version: '0.4.4' },
  },
};
