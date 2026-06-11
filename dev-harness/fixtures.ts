// Fixture props for rendering Popover in the browser preview harness.
import type { Workspace } from '../src/lib/workspaces';

export const workspaces: Workspace[] = [
  {
    slug: 'indigo',
    displayName: 'Indigo',
    kind: 'company',
    state: 'synced',
    cloudUid: 'cmp_indigo',
    bucketName: 'hq-vault-indigo',
    hasLocalFolder: true,
    localPath: '/Users/corey/Documents/HQ/companies/indigo',
    membershipStatus: 'active',
    lastSyncedAt: new Date().toISOString(),
    brokenReason: null,
  },
  {
    slug: 'personal',
    displayName: 'Personal',
    kind: 'personal',
    state: 'personal',
    cloudUid: 'cmp_personal',
    bucketName: 'hq-vault-personal',
    hasLocalFolder: true,
    localPath: '/Users/corey/Documents/HQ/companies/personal',
    membershipStatus: 'active',
    lastSyncedAt: new Date().toISOString(),
    brokenReason: null,
  },
  {
    slug: 'liverecover',
    displayName: 'LiveRecover',
    kind: 'company',
    state: 'cloud-only',
    cloudUid: 'cmp_liverecover',
    bucketName: 'hq-vault-liverecover',
    hasLocalFolder: false,
    localPath: null,
    membershipStatus: 'active',
    lastSyncedAt: null,
    brokenReason: null,
  },
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
