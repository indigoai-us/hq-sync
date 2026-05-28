<script lang="ts">
  import * as Sentry from '@sentry/svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import SignInPrompt from './components/SignInPrompt.svelte';
  import Popover from './components/Popover.svelte';
  import Settings from './components/Settings.svelte';
  import { conflictStore, type ConflictFile } from './stores/conflicts';
  import { shouldSkipSignIn } from './lib/auth';
  import type { Workspace, WorkspacesResult } from './lib/workspaces';
  import { loadMeetingDetectEligible } from './lib/permissionState.svelte';
  import { buildClaudeCodeUrl } from './lib/claude-code-link';
  import './styles/popover.css';

  interface Config {
    configured: boolean;
    companySlug: string;
    hqFolderPath: string;
    error?: string;
  }

  let authenticated = $state(false);
  let expiresAt = $state('');
  let checking = $state(true);
  let syncState = $state<'idle' | 'syncing' | 'error' | 'conflict' | 'setup-needed' | 'auth-error'>('idle');
  let config = $state<Config | null>(null);
  // Phase 7 runner protocol — progress is per-file with a path + bytes.
  // We also track the company currently syncing (last progress event) and
  // the count of companies in this fanout for "Syncing N of M" framing.
  let syncProgress = $state<{
    company: string;
    path: string;
    bytes: number;
  } | null>(null);
  let syncFanoutTotal = $state(0); // How many companies we're syncing
  let syncFanoutDoneCount = $state(0); // How many have hit sync:complete
  // Company list from the last fanout-plan event. `name` is optional —
  // runners < v5.1.9 only emit `uid` + `slug`, so the UI falls back to the
  // slug in that case. Rendered by Popover so the user sees *which* HQs
  // they're connected to.
  let syncCompanies = $state<Array<{ uid: string; slug: string; name?: string }>>([]);
  // Per-run cumulative file counter — incremented per sync:progress event so
  // the popover can show "234 files" alongside the current file. Reset on
  // each Sync Now click; not reset by sync:all-complete (the final summary
  // line takes over from there).
  let syncFilesProgressed = $state(0);
  // Personal first-push knows files_total upfront; we capture it so the
  // live-progress card can show "234 of 1,247 files" instead of just a
  // running count. Runner sync:progress events don't carry a total, so
  // these stay null during the runner phase and the UI falls back to
  // "234 files synced".
  let personalFilesDone = $state(0);
  let personalFilesTotal = $state<number | null>(null);
  // Latched flag for the unified progress bar — once the in-process Rust
  // personal first-push completes, this stays true until the next Sync
  // click. Lets the bar treat the personal phase as "fully filled (50%
  // slot)" even after personalFilesTotal has been reset, so there's no
  // visible drop between the Rust phase and the runner taking over.
  let personalFirstPushDone = $state(false);
  // Real total file count for the entire sync — emitted by the Rust pre-walk
  // BEFORE any uploads begin (sums personal allowlist + every local company
  // folder, after applying .hqignore + DEFAULT_IGNORES). Drives the unified
  // per-file progress bar. 0 means pre-walk hasn't fired yet (or hit an
  // error); the UI falls back to workspace-level progress in that case.
  let syncTotalFiles = $state(0);
  // Plan-event-derived denominator (hq-cloud@5.5.0+). Each plan event from
  // the runner adds (filesToDownload + filesToUpload + filesToConflict)
  // for one company / direction. When the runner is new enough to emit
  // these, this gives us an accurate denominator for the progress bar
  // that includes BOTH push and pull work — improving on the older
  // upload-only `syncTotalFiles` from the Rust pre-pass. When 0, the
  // UI falls back to `syncTotalFiles`.
  let syncPlanTotalFiles = $state(0);
  // filesSkipped is not on sync:all-complete (backend only aggregates
  // filesDownloaded), so we sum it client-side from per-company complete
  // events. Lets the popover surface "Up to date" when everything was
  // current instead of misreading as "Last sync · 0 files".
  let syncFanoutFilesSkipped = $state(0);
  let syncLastSummary = $state<{
    companiesAttempted: number;
    filesDownloaded: number;
    bytesDownloaded: number;
    filesSkipped: number;
  } | null>(null);
  let syncErrorMessage = $state(''); // Last auth-error or error message
  // Company slug attached to the last `sync:error` event, threaded into the
  // sync-failed Copy-Prompt so it can render `~/.hq/sync-journal.{slug}.json`
  // as a concrete path. Empty for auth errors / discovery-phase failures /
  // local catch-block failures where the slug isn't known.
  let syncErrorCompany = $state('');

  // New-files state — accumulated from `sync:new-files` events (one per
  // company). Cleared when a new sync starts. The badge in Popover renders
  // when the list is non-empty.
  let newFilesList = $state<Array<{ path: string; bytes: number; addedBy: string | null }>>([]);
  let newFilesCount = $derived(newFilesList.length);

  // Effective progress denominator. When the runner is new enough to emit
  // Stage-1 plan events (hq-cloud@5.5.0+), `syncPlanTotalFiles` accumulates
  // a more accurate count that includes both push and pull work; otherwise
  // we fall through to the older Rust-side pre-pass total in
  // `syncTotalFiles`. Either source is fine — the popover bar treats this
  // as the denominator and renders honestly when it's still 0.
  const effectiveTotalFiles = $derived(
    syncPlanTotalFiles > 0 ? syncPlanTotalFiles : syncTotalFiles
  );
  let showConflictModal = $state(false);
  let conflicts = $state<ConflictFile[]>([]);
  let showSettings = $state(false);
  let syncStatsRefresh = $state<(() => void) | null>(null);

  // Meetings feature flag — driven by `meetings_feature_enabled` (Rust side
  // decodes the cached Cognito id_token and checks @getindigo.ai). The icon
  // doesn't render at all when this is false. Click opens the standalone
  // `meetings-window` (mirrors the `new-files-detail` window pattern) — the
  // earlier modal-on-popover UX was too cramped.
  let meetingsEnabled = $state(false);

  // Memberships drive the company picker in the active-meetings row.
  // Loaded once on mount (same source as MeetingsWindow's URL-invite
  // dropdown). Errors degrade to an empty list — the row still renders
  // with Personal as the only option, never blocking detection or
  // recording on a vault hiccup.
  interface MembershipRow {
    companyUid: string;
    companyName: string | null;
    role: string | null;
    status: string;
  }
  let memberships = $state<MembershipRow[]>([]);
  // Default company UID for new recordings — read from menubar.json on
  // mount. Per-recording overrides happen in the popover row dropdown
  // and never write back here; that mutation belongs to Settings.
  let defaultRecordingCompanyUid = $state<string | null>(null);

  /**
   * Active meeting detections — populated as the Recall Desktop SDK fires
   * `meeting:detected` events. Lives only in-memory: we don't persist
   * across app restarts because the SDK re-emits detections after restart
   * if a meeting window is still active.
   *
   * Keyed implicitly by `windowId` (the SDK's stable handle for the
   * meeting window). Entries are added on `meeting:detected`, mutated
   * on `recording:started` / `recording:ended` / `recording:error`,
   * and removed on `meeting:closed`.
   *
   * Surfaced to the user via the Popover's "Active meetings" section,
   * where each entry gets a Record / Stop button wired back into the
   * `start_recording` / `stop_recording` Tauri commands.
   */
  interface ActiveMeeting {
    /** SDK window id (stable for the duration of the meeting). */
    windowId: string;
    /** Lowercase platform discriminator (`zoom`, `meet`, ...). */
    platform: string;
    /** Meeting URL — real or synthetic `recall-window:<id>`. */
    meetingUrl: string;
    /** ISO 8601 timestamp when the detection fired. */
    detectedAt: string;
    /** Lifecycle state — drives the Record/Stop button label. */
    state: 'detected' | 'starting' | 'recording' | 'stopping' | 'error';
    /** Recall.ai recording id (returned by start_recording). */
    recordingId?: string;
    /** Last error message from a failed start/stop, if any. */
    error?: string;
    /**
     * Company attribution for this recording. `null` = Personal vault.
     * Seeded from `defaultRecordingCompanyUid` at detection, but the
     * default may not have loaded yet when a detection fires — so the
     * authoritative resolution happens at `handleStartRecording` time
     * (and we back-fill rows when the default finishes loading).
     */
    companyUid: string | null;
    /**
     * True once the user has explicitly picked a company for this row via
     * the dropdown. Distinguishes an intentional "Personal" (companyUid =
     * null, userSet = true) from "default hasn't loaded yet" (companyUid =
     * null, userSet = false). Only the latter gets back-filled / resolved
     * to the default.
     */
    companyUserSet?: boolean;
  }
  let activeMeetings = $state<ActiveMeeting[]>([]);

  function upsertActiveMeeting(m: ActiveMeeting) {
    const idx = activeMeetings.findIndex((x) => x.windowId === m.windowId);
    if (idx >= 0) {
      activeMeetings[idx] = m;
    } else {
      activeMeetings = [...activeMeetings, m];
    }
  }
  function updateActiveMeeting(
    windowId: string,
    patch: Partial<ActiveMeeting>,
  ) {
    const idx = activeMeetings.findIndex((x) => x.windowId === windowId);
    if (idx < 0) return;
    activeMeetings[idx] = { ...activeMeetings[idx], ...patch };
  }
  function removeActiveMeeting(windowId: string) {
    activeMeetings = activeMeetings.filter((x) => x.windowId !== windowId);
  }

  /**
   * The persisted default company, but only if it's still a company the
   * user is an active member of. Returns null (Personal) otherwise — never
   * pre-select a stale uid the user can't record to (hq-pro would 403).
   */
  function resolveValidDefault(): string | null {
    return defaultRecordingCompanyUid &&
      memberships.some((m) => m.companyUid === defaultRecordingCompanyUid)
      ? defaultRecordingCompanyUid
      : null;
  }

  async function handleStartRecording(windowId: string) {
    updateActiveMeeting(windowId, { state: 'starting', error: undefined });
    // Resolve the company at START time, not just whatever was frozen on
    // the row at detection. The detection may have fired before the
    // default-company context finished loading (it's a fire-and-forget
    // load after an async feature-gate check), leaving companyUid null on
    // the row. Unless the user *explicitly* picked a company, fall back to
    // the current valid default — this is what fixes the notification
    // "Record" path attributing to Personal even when a default is set.
    const row = activeMeetings.find((m) => m.windowId === windowId);
    const companyUid = row?.companyUserSet
      ? (row.companyUid ?? null)
      : (resolveValidDefault() ?? row?.companyUid ?? null);
    // Reflect the resolved attribution back onto the row so the popover
    // dropdown shows what we actually recorded against.
    if (row && row.companyUid !== companyUid) {
      updateActiveMeeting(windowId, { companyUid });
    }
    try {
      const recordingId = await invoke<string>('start_recording', {
        windowId,
        companyUid,
      });
      updateActiveMeeting(windowId, { recordingId });
      // The actual flip to `recording` happens on the `recording:started`
      // event listener below — that confirms the SDK accepted the start,
      // not just that the bridge dispatched it.
    } catch (err) {
      console.error('start_recording failed:', err);
      updateActiveMeeting(windowId, {
        state: 'error',
        error: typeof err === 'string' ? err : String(err),
      });
    }
  }

  function handleChangeRecordingCompany(windowId: string, companyUid: string | null) {
    // User explicitly picked a company — mark it so the start-time resolver
    // and the default back-fill both respect this choice (including an
    // intentional "Personal").
    //
    // The dropdown is editable during recording too. NOTE: changing it
    // mid-recording updates the row's intent but does NOT yet re-attribute
    // the recording — the Recall metadata is baked at upload-token mint
    // (start) time. True "company at end" requires the hq-pro `/finalize`
    // endpoint (a tracked follow-up); until that ships, the START company
    // is what routes. We still capture the value here so the finalize
    // wiring is a drop-in once that endpoint exists.
    updateActiveMeeting(windowId, { companyUid, companyUserSet: true });
  }

  /**
   * Load the memberships list + the persisted default-recording-company
   * UID into module state. Called once on mount when the meeting-detect
   * feature is enabled for this user. Best-effort — both reads degrade
   * to empty / null on error so a vault hiccup never blocks the popover
   * from rendering or the user from recording (the row just shows
   * Personal as the only option, which is the safe default).
   */
  async function loadRecordingCompanyContext() {
    try {
      const [list, settings] = await Promise.all([
        invoke<MembershipRow[]>('meetings_list_memberships').catch(() => []),
        invoke<{ defaultRecordingCompanyUid?: string | null }>('get_settings').catch(
          () => ({} as { defaultRecordingCompanyUid?: string | null }),
        ),
      ]);
      memberships = (list ?? []).filter((m) => m.status === 'active');
      const storedUid = settings?.defaultRecordingCompanyUid ?? null;
      defaultRecordingCompanyUid = storedUid && memberships.some((m) => m.companyUid === storedUid)
        ? storedUid
        : null;
      // Back-fill any detections that fired before this load completed:
      // rows the user hasn't explicitly touched should reflect the default
      // (or stay Personal if there's no valid default). Without this, a
      // meeting detected during cold-start keeps companyUid=null and the
      // notification "Record" path attributes it to Personal.
      const validDefault = resolveValidDefault();
      if (validDefault) {
        for (const m of activeMeetings) {
          if (!m.companyUserSet && m.companyUid !== validDefault) {
            updateActiveMeeting(m.windowId, { companyUid: validDefault });
          }
        }
      }
    } catch (err) {
      console.warn('loadRecordingCompanyContext failed (non-blocking):', err);
    }
  }
  async function handleStopRecording(windowId: string) {
    updateActiveMeeting(windowId, { state: 'stopping' });
    try {
      await invoke('stop_recording', { windowId });
      // Flip to detected/closed on `recording:ended` event.
    } catch (err) {
      console.error('stop_recording failed:', err);
      // Roll back to recording — the bridge errored before the SDK got
      // the stop, so we're still recording.
      updateActiveMeeting(windowId, {
        state: 'recording',
        error: typeof err === 'string' ? err : String(err),
      });
    }
  }

  // Workspaces — populated by `list_syncable_workspaces` (Rust). Replaces the
  // legacy "No companies yet" dead-end with a union over Person + memberships
  // + local company folders. `null` = first invocation in flight; non-null
  // (even empty) = command completed at least once.
  let workspaces = $state<Workspace[] | null>(null);
  let workspacesCloudReachable = $state(true);
  let workspacesError = $state<string | null>(null);
  // Top-level manifest parse/IO error from list_syncable_workspaces. Distinct
  // from workspacesError (which surfaces cloud-side failure). Both can fire
  // independently — a broken manifest doesn't prevent us from talking to the
  // cloud, and an unreachable cloud doesn't make the manifest unreadable.
  let workspacesManifestError = $state<string | null>(null);

  // Updater state — populated by the `update:available` event from the Rust
  // background checker (launch+10s, then every 6h). Non-null means the user
  // is on an older version and the banner should be shown.
  let updateAvailable = $state<{ version: string; body?: string; date?: string } | null>(null);
  // True while `invoke('install_update')` is in-flight — blocks duplicate
  // clicks and lets the button show a spinner. On macOS the process usually
  // terminates before the promise resolves, so this rarely flips back.
  let updateInstalling = $state(false);

  // hq CLI updater state — populated by `hq-cli-update:available` from the
  // Rust background checker (launch+15s, then every 6h). Non-null means
  // the user's globally-installed `hq` is behind npm `latest`. The banner
  // can't auto-install (npm globals require shell access we don't have),
  // so it surfaces a copy-able upgrade command instead.
  let hqCliUpdateAvailable = $state<{ local: string | null; latest: string } | null>(null);
  // True while `invoke('install_hq_cli_update')` is in flight — disables
  // the banner button and flips its label to "Installing…".
  let hqCliUpdateInstalling = $state(false);
  // Last error returned from `install_hq_cli_update`. When non-null, the
  // banner switches to its error state and shows a Copy-command fallback
  // (typical failure: EACCES against a system-prefix npm that needs sudo).
  let hqCliUpdateError = $state<string | null>(null);

  // Unified HQ-core state — replaces the pre-refactor quad
  // (hqCoreUpdateAvailable + hqCoreDrift + stagingDrift + stagingReplace)
  // with one struct emitted on `core-state:changed` from the Rust
  // background checker (see commands/hq_core_state.rs). The pill labels +
  // visibility derive entirely from this one source of truth.
  //
  // `channel` chooses the comparison target:
  //   * "release" → drift vs latest tag on indigoai-us/hq-core
  //   * "staging" → drift vs main HEAD on hq-core-staging
  //
  // `driftReport.count` is THE drift count (USER-EDIT files only); MISSING +
  // USER-ONLY are listed in modified/missing/added but don't add to count.
  // `versionBehind` is true when the user is on an older release/SHA than
  // the target. `needsUpdate = versionBehind || driftReport.count > 0` —
  // i.e. the Update pill shows whenever the rescue would do something.
  type DriftEntry = {
    path: string;
    size: number;
    gitShaLocal: string | null;
    gitShaUpstream: string | null;
  };
  type DriftReport = {
    count: number;
    modified: DriftEntry[];
    missing: DriftEntry[];
    added: DriftEntry[];
    scannedAt: string;
    hqVersion: string;
    targetRepo: string;
    targetRef: string;
  };
  type CoreState = {
    channel: 'release' | 'staging';
    targetRepo: string;
    targetVersion: string;
    targetRef: string;
    localVersion: string | null;
    floorSha: string | null;
    isEligible: boolean;
    versionBehind: boolean;
    driftReport: DriftReport;
    unchangedCount: number;
    userOnlyCount: number;
    scannedAt: string;
  };
  let coreState = $state<CoreState | null>(null);
  // Spinner / disable flag for the Update pill while the rescue script
  // (release `install_hq_core_update` or staging `run_replace_from_staging`)
  // is running.
  let coreInstalling = $state<boolean>(false);
  // Last install-run summary (success or error). Surfaced via Popover so
  // the user sees ✓ / ✗ in the same row the pill lives in. Cleared at the
  // start of a new run.
  let coreInstallLastResult = $state<{
    kind: 'ok' | 'err';
    exitCode: number;
    logTail: string;
    logPath: string;
  } | null>(null);

  // Locally-installed hq-core `hqVersion` (or null when core.yaml is
  // missing/unparseable). Always populated by a cheap on-disk read at app
  // mount — independent of the unified state's 6h cadence. Drives the
  // "HQ v14.2.1" footer row in Popover; null surfaces the repair affordance
  // instead of silently hiding the row.
  let hqVersion = $state<string | null>(null);

  // Collected unlisten handles for cleanup
  let unlisteners: UnlistenFn[] = [];

  async function loadConfig() {
    try {
      config = await invoke<Config>('get_config');
    } catch (err) {
      console.error('Failed to load config:', err);
    }
  }

  // Cheap on-disk read of hq-core's `hqVersion` from `core.yaml`. Null when
  // unreadable — see `hqVersion` state declaration for why null is surfaced
  // rather than swallowed. Errors are logged but treated as null so a
  // transient Rust failure doesn't blank the row mid-session.
  async function loadHqVersion() {
    try {
      hqVersion = await invoke<string | null>('get_hq_version');
    } catch (err) {
      console.error('Failed to load hq version:', err);
      hqVersion = null;
    }
  }

  // Refresh the unified core state on demand (mount, post-settings,
  // post-rescue). Errors swallowed — the background listener will
  // repopulate on the next 6h tick. Maps snake_case Rust → camelCase JS.
  async function loadCoreState() {
    try {
      const s = await invoke<CoreState | null>('check_core_state');
      coreState = s;
    } catch (err) {
      console.error('check_core_state failed:', err);
    }
  }

  // Unified "Update" action — dispatches to the right rescue command based
  // on the active channel. Release channel runs `install_hq_core_update`
  // (overlays the latest hq-core release tag); staging channel runs
  // `run_replace_from_staging` (overlays staging main). Both return the
  // same RescueRunResult shape so the surface is identical.
  //
  // Long-running (30-90s on first run because of the full-history clone +
  // scan). The pill is disabled while the promise is pending; the result
  // lands in `coreInstallLastResult` for Popover to surface. On success we
  // refresh `hqVersion` + re-run the state check so drift + version pills
  // both swing to the post-rescue truth without waiting for the 6h tick.
  async function handleInstallCore() {
    if (coreInstalling) return;
    if (!coreState) return;
    coreInstalling = true;
    coreInstallLastResult = null;
    const command =
      coreState.channel === 'staging'
        ? 'run_replace_from_staging'
        : 'install_hq_core_update';
    try {
      const result = await invoke<{
        exit_code: number;
        log_tail: string;
        log_path: string;
      }>(command);
      coreInstallLastResult = {
        kind: result.exit_code === 0 ? 'ok' : 'err',
        exitCode: result.exit_code,
        logTail: result.log_tail,
        logPath: result.log_path,
      };
      await loadHqVersion();
      if (result.exit_code === 0) {
        // Re-run unified state so version_behind + drift both swing to
        // post-rescue truth. Fire-and-forget — failure leaves the prior
        // state until the next background tick.
        invoke('check_core_state').catch((e) =>
          console.error('post-install core-state refresh failed:', e)
        );
      }
    } catch (err) {
      console.error(`${command} failed:`, err);
      coreInstallLastResult = {
        kind: 'err',
        exitCode: -1,
        logTail: String(err),
        logPath: '',
      };
    } finally {
      coreInstalling = false;
    }
  }

  /**
   * Fetch the workspaces union (Personal + memberships + local folders).
   * Called on mount, after sync completes, and after settings change. Errors
   * surface via the `cloudReachable` flag in the result — the Rust command
   * never throws for cloud-side problems, only for environment failures
   * (e.g. cannot resolve hq folder path).
   */
  async function loadWorkspaces() {
    try {
      const result = await invoke<WorkspacesResult>('list_syncable_workspaces');
      workspaces = result.workspaces;
      workspacesCloudReachable = result.cloudReachable;
      workspacesError = result.error;
      workspacesManifestError = result.manifestError;
    } catch (err) {
      // Hard failure (e.g. couldn't resolve hq_root). Keep prior workspaces
      // visible if we had any, but flag the error so the UI can soften.
      console.error('list_syncable_workspaces failed:', err);
      workspacesCloudReachable = false;
      workspacesError = String(err);
      // Don't null out `workspaces` — last-good is better than empty.
    }
  }

  async function handleSyncNow() {
    if (syncState === 'syncing') return;
    syncState = 'syncing';
    syncProgress = null;
    syncFanoutTotal = 0;
    syncFanoutDoneCount = 0;
    syncCompanies = [];
    syncFanoutFilesSkipped = 0;
    syncFilesProgressed = 0;
    personalFilesDone = 0;
    personalFilesTotal = null;
    personalFirstPushDone = false;
    syncTotalFiles = 0;
    syncPlanTotalFiles = 0;
    syncLastSummary = null;
    syncErrorMessage = '';
    syncErrorCompany = '';
    newFilesList = [];
    await invoke('set_tray_state', { state: 'syncing' });
    try {
      await invoke('start_sync');
    } catch (err) {
      console.error('start_sync failed:', err);
      syncState = 'error';
      syncErrorMessage = String(err);
      syncErrorCompany = '';
      await invoke('set_tray_state', { state: 'error' });
    }
  }

  async function handleCancel() {
    if (syncState !== 'syncing') return;
    try {
      await invoke('cancel_sync');
      // Don't flip syncState here — the runner's exit triggers the
      // existing "runner exited" path which emits sync:all-complete (or
      // sync:error) and resets state. Avoids a race where cancel returns
      // before the kill propagates.
    } catch (err) {
      console.error('cancel_sync failed:', err);
    }
  }

  function handleSettings() {
    showSettings = true;
  }

  function handleBackFromSettings() {
    showSettings = false;
    // User may have changed the HQ folder path in Settings; the header in
    // Popover renders from `config.hqFolderPath`, which was snapshotted at
    // mount. Re-read menubar.json so the change is visible without a quit.
    // Workspaces depend on hq_root too — local folder enumeration would point
    // at the wrong tree otherwise. hqVersion also follows the HQ root —
    // re-tethering to a different folder may surface a different (or now-
    // readable) `core.yaml`.
    loadConfig();
    loadWorkspaces();
    loadHqVersion();
    // User may have just flipped the staging-channel toggle — re-run the
    // unified state check so channel + target + drift all swing without
    // waiting for the 6h background tick.
    loadCoreState();
  }

  function handleSignOut() {
    // Placeholder: clear auth state, return to sign-in
    authenticated = false;
    expiresAt = '';
    console.log('Sign out requested — clearing local auth state');
  }

  async function handleResolveConflict(path: string, strategy: 'keep-local' | 'keep-remote') {
    await conflictStore.resolveConflict(path, strategy);
    conflicts = conflictStore.conflicts;
    if (conflictStore.allResolved) {
      syncState = 'idle';
      await invoke('set_tray_state', { state: 'idle' });
    }
  }

  async function handleOpenInEditor(path: string) {
    await conflictStore.openInEditor(path);
  }

  function handleDismissConflicts() {
    showConflictModal = false;
  }

  async function handleInstallHqCliUpdate() {
    if (hqCliUpdateInstalling) return;
    hqCliUpdateInstalling = true;
    hqCliUpdateError = null;
    try {
      // Backend spawns `npm install -g @indigoai-us/hq-cli@latest` and
      // re-checks on success. We clear the banner on success; on failure
      // we surface the stderr so the banner can fall back to its
      // copy-the-command affordance. See
      // src-tauri/src/commands/hq_cli_update.rs:install_hq_cli_update.
      const info = await invoke<{ local: string | null; latest: string }>(
        'install_hq_cli_update'
      );
      // npm exited 0 but the version might still lag (e.g., npm picked
      // up a cached resolution). Compare and only clear the banner when
      // the local version is actually current.
      if (info.local && info.local === info.latest) {
        hqCliUpdateAvailable = null;
      } else {
        hqCliUpdateAvailable = info;
      }
    } catch (err) {
      console.error('install_hq_cli_update failed:', err);
      hqCliUpdateError = String(err);
    } finally {
      hqCliUpdateInstalling = false;
    }
  }

  async function handleInstallUpdate() {
    if (updateInstalling) return;
    updateInstalling = true;
    try {
      // Backend re-runs updater.check() inside install_update because
      // tauri_plugin_updater::Update is not Clone — we can't stash the
      // Update object across IPC. See src-tauri/src/updater.rs:41-60.
      // On macOS the app process is typically replaced before this
      // promise resolves; updateInstalling stays true by design.
      await invoke('install_update');
    } catch (err) {
      console.error('install_update failed:', err);
      updateInstalling = false;
    }
  }

  async function handleCheckForUpdates() {
    try {
      const info = await invoke<{ version: string; body?: string; date?: string } | null>(
        'check_for_updates'
      );
      // Backend also emits `update:available` on hit, so the listener
      // picks it up — but set it here too in case the listener races.
      if (info) updateAvailable = info;
    } catch (err) {
      console.error('check_for_updates failed:', err);
    }
  }

  async function setupTrayListeners() {
    // Refresh workspaces every time the menubar popover gains focus. Cheap
    // (single Tauri command + small vault round-trip) and catches external
    // mutations: a new company added via /newcompany, a manifest patch from
    // a CLI tool, or any folder created outside the app between popover
    // openings. Without this, the list only refreshes on mount and after a
    // sync — a brand-new company added between syncs would stay invisible
    // until the next sync click.
    unlisteners.push(
      await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) {
          loadWorkspaces();
        }
      })
    );

    // Tray menu events
    unlisteners.push(
      await listen('tray:sync-now', () => {
        handleSyncNow();
      })
    );

    unlisteners.push(
      await listen('tray:open-settings', () => {
        handleSettings();
      })
    );

    // --- Phase 7 runner event listeners ---
    // Protocol (see src-tauri/src/events.rs):
    //   sync:setup-needed  -- signed in, no person entity yet
    //   sync:auth-error    -- token invalid and can't refresh
    //   sync:fanout-plan   -- list of companies about to sync
    //   sync:progress      -- per-file download in-flight
    //   sync:error         -- per-file or per-company error
    //   sync:complete      -- per-company summary (fires N times in a fanout)
    //   sync:all-complete  -- aggregate summary; this is the real "done"

    unlisteners.push(
      await listen('sync:setup-needed', async () => {
        // Runner emits this when the caller has no memberships AND no
        // pending invites. As of the Rust auto-create patch, the personal
        // first-push provisions the person entity itself before the runner
        // even starts — so by the time we see setup-needed here, the only
        // remaining gap is "no companies yet", which is a perfectly normal
        // state for a brand-new account, not an error. Don't flip the tray
        // to red; just stay in syncing until all-complete fires.
        syncState = 'syncing';
        syncProgress = null;
      })
    );

    unlisteners.push(
      await listen<{ message: string }>('sync:auth-error', async (event) => {
        syncState = 'auth-error';
        syncProgress = null;
        syncErrorMessage = event.payload.message;
        syncErrorCompany = '';
        await invoke('set_tray_state', { state: 'error' });
      })
    );

    // Pre-walk total — fired once after JWT resolution, before any uploads.
    // Carries the real file count for this entire sync so the UI bar can
    // show actual per-file progress instead of fake workspace thirds.
    unlisteners.push(
      await listen<{ totalFiles: number }>('sync:totals', async (event) => {
        syncTotalFiles = event.payload.totalFiles;
      })
    );

    // Stage-1 plan events from the runner (hq-cloud@5.5.0+). Each plan
    // event covers one company / direction (push or pull). Accumulating
    // gives a denominator that includes BOTH push and pull work — the
    // older `sync:totals` event only counted uploads. When connected to
    // an older runner that doesn't emit plan, this stays at 0 and the
    // UI falls back to syncTotalFiles automatically (the renderer below
    // picks the larger of the two — see `progressDenominator` derived).
    unlisteners.push(
      await listen<{
        company: string;
        filesToDownload: number;
        bytesToDownload: number;
        filesToUpload: number;
        bytesToUpload: number;
        filesToSkip: number;
        filesToConflict: number;
      }>('sync:plan', async (event) => {
        const { filesToDownload, filesToUpload, filesToConflict } = event.payload;
        // Sum work across the run: each plan event adds its own slice.
        syncPlanTotalFiles += filesToDownload + filesToUpload + filesToConflict;
      })
    );

    unlisteners.push(
      await listen<{ companies: Array<{ uid: string; slug: string; name?: string }> }>(
        'sync:fanout-plan',
        async (event) => {
          syncState = 'syncing';
          syncFanoutTotal = event.payload.companies.length;
          syncFanoutDoneCount = 0;
          syncCompanies = event.payload.companies;
          await invoke('set_tray_state', { state: 'syncing' });
        }
      )
    );

    unlisteners.push(
      await listen<{ company: string; path: string; bytes: number; message?: string }>(
        'sync:progress',
        async (event) => {
          syncState = 'syncing';
          syncProgress = {
            company: event.payload.company,
            path: event.payload.path,
            bytes: event.payload.bytes,
          };
          // Cumulative file counter — every per-file event from the runner
          // (or personal first-push) bumps this. The popover surfaces it as
          // "234 files" alongside the current path so the user always has
          // something moving even when individual paths scroll by.
          syncFilesProgressed += 1;
          await invoke('set_tray_state', { state: 'syncing' });
        }
      )
    );

    // ── Personal-first-push events ────────────────────────────────────────
    // The in-process Rust personal first-push fires its own progress events
    // (not routed through the runner's sync:progress channel) and carries
    // an upfront filesTotal — we feed both into the live-progress card so
    // the personal phase shows "234 of 1,247 files" while the (unknown-
    // total) runner phase shows just "234 files synced".
    unlisteners.push(
      await listen<{
        personUid: string;
        filesDone: number;
        filesTotal: number;
        currentFile: string | null;
      }>('sync:personal-first-push-progress', async (event) => {
        syncState = 'syncing';
        personalFilesDone = event.payload.filesDone;
        personalFilesTotal = event.payload.filesTotal;
        if (event.payload.currentFile) {
          syncProgress = {
            company: 'personal',
            path: event.payload.currentFile,
            bytes: 0, // personal-first-push doesn't carry per-file bytes
          };
          syncFilesProgressed += 1;
        }
        await invoke('set_tray_state', { state: 'syncing' });
      })
    );

    unlisteners.push(
      await listen<{ personUid: string; filesUploaded: number; filesSkipped: number }>(
        'sync:personal-first-push-complete',
        async () => {
          // Latch the done flag so the unified bar treats the personal
          // slot as 100% filled while the runner spins up. Don't clear
          // personalFilesTotal/Done — leaving them in place keeps the
          // file-level caption visible until the runner takes over with
          // its own caption.
          personalFirstPushDone = true;
        }
      )
    );

    unlisteners.push(
      await listen<{
        company: string;
        filesDownloaded: number;
        bytesDownloaded: number;
        filesSkipped: number;
        conflicts: number;
        aborted: boolean;
      }>('sync:complete', async (event) => {
        // Per-company event — just tick the counter. Don't go idle yet;
        // wait for sync:all-complete to know the whole fanout is done.
        // We do NOT add filesSkipped to syncFilesProgressed: the runner
        // only emits per-file `progress` events for transfers, not skips,
        // and the new pre-walk denominator counts only transfers too.
        // Adding skips here would inflate the numerator and break the
        // ratio.
        syncFanoutDoneCount += 1;
        syncFanoutFilesSkipped += event.payload.filesSkipped;
        if (event.payload.aborted) {
          // Conflict-aborted: show the conflict state so the user knows
          // something needs attention. ConflictModal wiring is follow-up
          // work (runner doesn't emit per-file conflict events anymore);
          // for now the tray + banner is enough signal.
          syncState = 'conflict';
          await invoke('set_tray_state', { state: 'conflict' });
        }
      })
    );

    unlisteners.push(
      await listen<{
        companiesAttempted: number;
        filesDownloaded: number;
        bytesDownloaded: number;
        errors: Array<{ company: string; message: string }>;
      }>('sync:all-complete', async (event) => {
        syncLastSummary = {
          companiesAttempted: event.payload.companiesAttempted,
          filesDownloaded: event.payload.filesDownloaded,
          bytesDownloaded: event.payload.bytesDownloaded,
          filesSkipped: syncFanoutFilesSkipped,
        };
        syncProgress = null;
        // Only flip to idle if nothing raised conflict/error mid-stream
        if (syncState !== 'conflict' && syncState !== 'error') {
          syncState = 'idle';
          await invoke('set_tray_state', { state: 'idle' });
        }
        // Refresh SyncStats so "last synced" updates immediately
        syncStatsRefresh?.();
        // Refresh workspaces — sync may have created new local folders
        // (for newly-provisioned companies) or updated last-synced timestamps.
        loadWorkspaces();
      })
    );

    unlisteners.push(
      await listen<{ company?: string; path: string; message: string }>(
        'sync:error',
        async (event) => {
          // Defence-in-depth: the Rust side already captures this via
          // `report_sync_error` in src-tauri/src/commands/sync.rs (which fires
          // for the same payload that produced this Tauri event). We capture
          // here too so the renderer-tagged Sentry project (`hq-sync-web`)
          // still receives the issue when the Rust build is missing
          // `HQ_SYNC_SENTRY_DSN` or its DSN parses to None at startup. Sentry
          // groups by message text so duplicate captures merge into one issue.
          Sentry.captureMessage(`[sync] ${event.payload.message}`, {
            level: 'error',
            tags: {
              path: event.payload.path,
              ...(event.payload.company ? { company: event.payload.company } : {}),
            },
          });
          syncState = 'error';
          syncProgress = null;
          syncErrorMessage = event.payload.message;
          syncErrorCompany = event.payload.company ?? '';
          await invoke('set_tray_state', { state: 'error' });
        }
      )
    );

    // --- Updater event listener ---
    // Protocol (see src-tauri/src/updater.rs):
    //   update:available — payload { version, body?, date? }
    //     Emitted by setup_update_checker (launch+10s, every 6h) and
    //     also by check_for_updates (on-demand). Render a banner.
    unlisteners.push(
      await listen<{ version: string; body?: string; date?: string }>(
        'update:available',
        (event) => {
          updateAvailable = event.payload;
        }
      )
    );

    // --- hq CLI updater event listener ---
    // Protocol (see src-tauri/src/commands/hq_cli_update.rs):
    //   hq-cli-update:available — payload { local: string | null, latest: string }
    //     `local` is null when the user doesn't have `hq` on PATH; the
    //     checker doesn't emit in that case, but we type it permissively.
    //   hq-cli-update:cleared — payload { local, latest } after an in-app
    //     `npm install -g` finishes successfully. The handler returning
    //     the same info already clears state, but we also listen here so
    //     a background tray check that ran in parallel can't re-show the
    //     banner stale.
    unlisteners.push(
      await listen<{ local: string | null; latest: string }>(
        'hq-cli-update:available',
        (event) => {
          // A fresh check arrived — discard any stale error from a
          // previous failed install so the button is clickable again.
          hqCliUpdateError = null;
          hqCliUpdateAvailable = event.payload;
        }
      )
    );
    unlisteners.push(
      await listen<{ local: string | null; latest: string }>(
        'hq-cli-update:cleared',
        (event) => {
          // Backend says install succeeded. Trust the version it
          // reports — only clear the banner when local actually
          // matches latest (a re-resolution that lagged the install
          // would leave local stale).
          if (event.payload.local && event.payload.local === event.payload.latest) {
            hqCliUpdateAvailable = null;
            hqCliUpdateError = null;
          } else {
            hqCliUpdateAvailable = event.payload;
          }
        }
      )
    );

    // --- unified hq-core state listener ---
    // Protocol (see src-tauri/src/commands/hq_core_state.rs):
    //   core-state:changed — full CoreState payload. Emitted on every
    //   background tick + every on-demand `check_core_state` invoke,
    //   including the "no drift, on latest" case so the pill can swing
    //   back to "in sync" after the user resolves.
    unlisteners.push(
      await listen<CoreState>('core-state:changed', (event) => {
        coreState = event.payload;
      })
    );

    // Tray menu "Check for Updates" → on-demand check.
    unlisteners.push(
      await listen('tray:check-for-updates', () => {
        handleCheckForUpdates();
      })
    );

    // --- New-files event listener (US-004) ---
    // The Rust backend emits one `sync:new-files` event per company that has
    // new files (detected by diffing the journal). Multiple events can fire
    // per sync run — we accumulate them into a single flat list.
    unlisteners.push(
      await listen<{ company: string; files: Array<{ path: string; bytes: number; addedBy?: string }> }>(
        'sync:new-files',
        (event) => {
          const incoming = event.payload.files.map((f) => ({
            path: f.path,
            bytes: f.bytes,
            addedBy: f.addedBy ?? null,
          }));
          newFilesList = [...newFilesList, ...incoming];
        }
      )
    );

    // --- Meeting-detection listener ---
    // Fired by the Recall Desktop SDK sidecar when a supported video-call app
    // becomes active. Flow:
    //   1. Check hq-pro for an already-active bot (dedup signal)
    //   2. If no bot, fire a macOS notification + bump the tray Prompt badge
    //      (both handled inside `meetings_notify_detected` which also gates on
    //      the user's notification prefs + the per-meeting ledger)
    unlisteners.push(
      await listen<{
        meetingUrl?: string;
        platform?: string;
        summary?: string;
        sourceEventId?: string;
        // Synthetic key carried in `meetingUrl` for URL-less detections;
        // SDK windowId is what we key on for recording control. The bridge
        // emits `meetingUrl: "recall-window:<windowId>"` in that case, so
        // we can extract it back out.
        windowId?: string;
      }>('meeting:detected', async (event) => {
        const { meetingUrl, platform, summary, sourceEventId } = event.payload;
        const directWindowId = event.payload.windowId;

        // Prefer the direct `windowId` field — it's the canonical SDK
        // handle that newer bridge versions include on every detection.
        // Fall back to extracting it from the synthetic
        // `recall-window:<id>` URL for backward compat (URL-less
        // detections; older bridge). Last-resort use the meetingUrl
        // itself as a stable key for dedup-only purposes (real URLs).
        const isSyntheticUrl = typeof meetingUrl === 'string'
          && meetingUrl.startsWith('recall-window:');
        const windowId = directWindowId
          ?? (isSyntheticUrl
            ? meetingUrl!.slice('recall-window:'.length)
            : (meetingUrl ?? ''));

        if (windowId) {
          // Seed the row with the current valid default. This may be null
          // if the default-company context hasn't loaded yet — that's fine:
          // `loadRecordingCompanyContext` back-fills unset rows when it
          // completes, and `handleStartRecording` re-resolves the default
          // at start time. `companyUserSet: false` marks this as a
          // non-explicit seed so both of those can safely overwrite it.
          upsertActiveMeeting({
            windowId,
            platform: platform ?? 'other',
            meetingUrl: meetingUrl ?? '',
            detectedAt: new Date().toISOString(),
            state: 'detected',
            companyUid: resolveValidDefault(),
            companyUserSet: false,
          });
        }

        try {
          // Synthetic `recall-window:<id>` URLs come from URL-less SDK
          // detections (unscheduled Zoom meetings, etc.). hq-pro will reject
          // them as invalid input, and there's no way a bot got provisioned
          // against that key anyway — skip the dedup check and go straight
          // to notify. Same fallback applies if the bot check itself
          // throws (network, auth, etc.) — better to over-notify once than
          // swallow the detection entirely.
          if (meetingUrl && !isSyntheticUrl) {
            try {
              const bot = await invoke<{ botId: string } | null>('meetings_check_bot_for_url', {
                meetingUrl,
                eventId: sourceEventId ?? null,
              });
              if (bot) return;
            } catch (botErr) {
              console.warn('meetings_check_bot_for_url failed, continuing to notify:', botErr);
            }
          }
          await invoke('meetings_notify_detected', {
            payload: {
              meetingUrl: meetingUrl ?? null,
              // Pass through so the notification's action-button thread
              // can route Record clicks back to start_recording.
              windowId: windowId || null,
              platform: platform ?? null,
              summary: summary ?? null,
              sourceEventId: sourceEventId ?? null,
            },
          });
        } catch (err) {
          console.error('meeting:detected handler error:', err);
        }
      })
    );

    // Recording lifecycle — flip the active-meeting row state machine as
    // the bridge confirms each transition. The Tauri commands above
    // (handleStart/StopRecording) only know "we asked the bridge to do
    // this"; these events confirm the SDK accepted it. We keep the row
    // in `starting` / `stopping` until the SDK confirms, then flip to
    // `recording` / removed.
    unlisteners.push(
      await listen<{ windowId: string; platform: string; startedAt: string }>(
        'recording:started',
        (event) => {
          updateActiveMeeting(event.payload.windowId, {
            state: 'recording',
            error: undefined,
          });
        },
      ),
    );
    unlisteners.push(
      await listen<{ windowId: string; platform: string; endedAt: string }>(
        'recording:ended',
        (event) => {
          // Recording over — drop the row. (Future: keep the row for a
          // few seconds showing "Saved" so the user gets confirmation
          // before it disappears.)
          removeActiveMeeting(event.payload.windowId);
        },
      ),
    );
    unlisteners.push(
      await listen<{ cmd: string; windowId: string; message: string }>(
        'recording:error',
        (event) => {
          updateActiveMeeting(event.payload.windowId, {
            state: 'error',
            error: `${event.payload.cmd}: ${event.payload.message}`,
          });
        },
      ),
    );
    unlisteners.push(
      await listen<{ windowId: string; platform: string; closedAt: string }>(
        'meeting:closed',
        (event) => {
          // User closed the meeting app without recording — drop the row
          // so the popover doesn't show stale detections.
          removeActiveMeeting(event.payload.windowId);
        },
      ),
    );

    // Notification action dispatch — fired by the Rust mac-notification-sys
    // worker thread when the user interacts with a "Meeting detected"
    // notification. Two cases:
    //   action="open"   → user clicked the notification body. Open the
    //                     popover so the active-meetings row is visible.
    //   action="record" → user clicked the Record action button. Skip
    //                     the popover and start recording directly.
    unlisteners.push(
      await listen<{ action: string; windowId: string; platform: string }>(
        'notification:meeting-action',
        async (event) => {
          const { action, windowId } = event.payload;
          if (action === 'record' && windowId) {
            await handleStartRecording(windowId);
            // Clear the prompt-tray badge — the user acted on the
            // detection, even if the recording itself errors.
            invoke('meetings_clear_prompt_badge').catch(() => {});
            return;
          }
          if (action === 'open') {
            // Pop the main popover into view. Tauri doesn't expose a
            // direct "open popover" command — the tray click is what
            // normally toggles visibility. We invoke `show_main_window`
            // (defined in main.rs) which focuses the popover window.
            invoke('show_main_window').catch((err) => {
              console.warn('show_main_window failed:', err);
            });
            invoke('meetings_clear_prompt_badge').catch(() => {});
          }
        },
      ),
    );

    // --- Share-notification event listener (US-005) ---
    // Rust emits `share:new-events` after each poll when new events are found.
    // The Rust side has already fired one macOS notification per event AND
    // primed the pending-events state for the detail-window ready-handshake.
    //
    // We deliberately do NOT open the ShareDetail window here. The window
    // opens only via user-initiated paths:
    //   1. notification click → `share-notify:detail-requested` listener
    //   2. notification action button "Open details" → same path
    //   3. tray click on the share-notify badge
    //
    // Auto-opening on every poll was a UX bug discovered during dogfood
    // (2026-05-26): combined with the cursor re-fire bug, the ShareDetail
    // window re-appeared every ~20s. Even with the cursor bug fixed, eager
    // open is wrong UX — the notification is the lightweight surface and
    // the detail window is opt-in.
    unlisteners.push(
      await listen<Array<{
        eventId: string;
        issuerEmail: string;
        issuerDisplayName: string;
        paths: string[];
        note: string | null;
        permission: string;
        createdAt: string;
      }>>('share:new-events', async (_event) => {
        // No-op for now — the notification handler in Rust owns the side
        // effects (notification.show(), pending-events state, tray badge).
        // This listener stays subscribed so a future in-popover share-
        // events list can hook here without needing a second registration.
      })
    );

    // --- Share-notification action handler (Fix D, 2026-05-26) ---
    // Rust spawns a thread per macOS notification that blocks on
    // mac-notification-sys `wait_for_click(true).send()`. When the user
    // hovers the notification and picks "Copy prompt" / "Open details"
    // from the Actions dropdown (or body-clicks for the open path), the
    // thread emits `notification:share-action` with the full event payload.
    //
    // "copy" → write the templated prompt to the system clipboard.
    // "open" → invoke open_share_detail with this single event so the
    //          ShareDetail window focuses or opens with the right context.
    unlisteners.push(
      await listen<{
        action: 'claude' | 'copy' | 'open';
        eventId: string;
        event: {
          eventId: string;
          issuerEmail: string;
          issuerDisplayName: string;
          paths: string[];
          note: string | null;
          permission: string;
          createdAt: string;
        };
      }>('notification:share-action', async (e) => {
        const { action, event: evt } = e.payload;

        // Shared prompt-template helper. Kept in sync with
        // ShareDetail.svelte::buildPrompt (which still owns the in-window
        // "Copy prompt" button). Moving to a shared module is a TODO once
        // a third consumer appears.
        const buildPrompt = () => {
          const pathList = evt.paths.join(', ');
          const note = evt.note?.trim() || '(no note)';
          return `${evt.issuerDisplayName} shared these files with me: ${pathList}\n\nTheir note: ${note}.`;
        };

        if (action === 'claude') {
          // Body-click → open Claude Code with the templated prompt
          // pre-filled in the input. Mirrors the pattern used by:
          //   * OpenInClaudeCodeButton (`lib/claude-code-link.ts`)
          //   * hq-installer's launch_claude_code_link flow
          //   * Popover.svelte fixHqCliUpdateInHq CTA
          //
          // User feedback 2026-05-26: prefer opening Claude Code over a
          // bare clipboard copy — the recipient almost always wants to
          // continue the share in an LLM session, so save them the
          // paste step.
          const folder = config?.hqFolderPath ?? '';
          try {
            const url = buildClaudeCodeUrl({ folder, prompt: buildPrompt() });
            await invoke('open_claude_code_link', { url });
          } catch (err) {
            console.error('share-notify: open_claude_code_link failed', err);
          }
        } else if (action === 'copy') {
          // Dropdown "Copy prompt" → clipboard write (no app launch).
          // Intentionally redundant with the body-click → Claude path for
          // users who already have a Claude session running, are pasting
          // into a different app, or want the literal text without any
          // side effects (user direction 2026-05-26).
          try {
            await navigator.clipboard.writeText(buildPrompt());
          } catch (err) {
            console.error('share-notify: clipboard write failed', err);
          }
        } else if (action === 'open') {
          try {
            await invoke('open_share_detail', { events: [evt] });
          } catch (err) {
            console.error('share-notify: open_share_detail failed', err);
          }
        }
      })
    );
  }

  $effect(() => {
    // Performance: mark app init
    performance.mark('app-init');

    checkAuth();
    loadConfig();
    loadWorkspaces();
    loadHqVersion();
    // Fire-and-forget — background listener will overwrite on the next
    // tick. Calling here gives the popover a populated state on first
    // open instead of waiting 30s for the bg checker.
    loadCoreState();
    setupTrayListeners();
    // Resolve the Phase-0 meeting-detect eligibility flag once on mount.
    // Settings.svelte hides the meeting-detect toggle when this is false.
    // (Per-permission TCC status tracking was removed 2026-05-25 — see
    // permissionState.svelte.ts for why; native macOS prompts are
    // sufficient.)
    loadMeetingDetectEligible();
    // One-time OS notification permission prompt. macOS only shows the system
    // dialog while status is `prompt` (not determined); once granted/denied it
    // returns silently, so calling this every launch never re-nags. Fire-and-
    // forget — errors are non-fatal (the Settings monitor lets the user retry).
    requestNotificationPermissionOnce();
    // Fire-and-forget: gate is a process-lifetime cache on the Rust side,
    // so subsequent reads are O(1). Errors silently treated as not-enabled.
    invoke<boolean>('meetings_feature_enabled')
      .then((v) => {
        meetingsEnabled = v;
        // Lazy-load the recording-company picker data only for users who
        // can actually trigger a detection. Saves a vault round-trip for
        // non-eligible accounts and keeps the cold-start trace cleaner.
        if (v) {
          void loadRecordingCompanyContext();
        }
      })
      .catch(() => {
        meetingsEnabled = false;
      });

    return () => {
      unlisteners.forEach((unlisten) => unlisten());
      unlisteners = [];
    };
  });

  // Ask macOS for notification authorization once. The Rust command wraps
  // tauri-plugin-notification's request_permission(); macOS shows the system
  // dialog only when the status is `prompt`, so this is safe to call on every
  // launch (no re-nag). We skip the request entirely when already determined
  // to avoid a needless IPC round-trip.
  async function requestNotificationPermissionOnce() {
    try {
      const state = await invoke<string>('notification_permission_state');
      if (state === 'prompt') {
        await invoke<string>('notification_request_permission');
      }
    } catch (err) {
      console.error('notification permission request failed', err);
    }
  }

  async function checkAuth() {
    try {
      // Skip the sign-in step when cognito-tokens.json already holds a
      // non-empty token. See `shouldSkipSignIn` for the ordering: we
      // prefer `get_auth_state`'s verdict (it tries a silent refresh) and
      // only fall back to raw token presence when it reports
      // unauthenticated — a stored token that's actually unusable will
      // raise `sync:auth-error` on first sync and route back through
      // sign-in from there.
      const [hasToken, state] = await Promise.all([
        invoke<boolean>('has_stored_token'),
        invoke<{
          authenticated: boolean;
          expiresAt: string | null;
        }>('get_auth_state'),
      ]);

      authenticated = shouldSkipSignIn(hasToken, state);
      expiresAt = state.expiresAt ?? '';
    } catch {
      authenticated = false;
    } finally {
      checking = false;
    }
  }

  function handleAuthSuccess(auth: { authenticated: boolean; expiresAt: string }) {
    authenticated = auth.authenticated;
    expiresAt = auth.expiresAt;
  }
</script>

<main>
  {#if checking}
    <div class="loading">
      <span class="dot-spinner"></span>
    </div>
  {:else if authenticated && showSettings}
    <Settings onback={handleBackFromSettings} />
  {:else if authenticated}
    <Popover
      {syncState}
      {config}
      progress={syncProgress}
      fanoutTotal={syncFanoutTotal}
      fanoutDoneCount={syncFanoutDoneCount}
      {syncFilesProgressed}
      {personalFilesDone}
      {personalFilesTotal}
      {personalFirstPushDone}
      syncTotalFiles={effectiveTotalFiles}
      {syncPlanTotalFiles}
      companies={syncCompanies}
      {workspaces}
      cloudReachable={workspacesCloudReachable}
      cloudError={workspacesError}
      manifestError={workspacesManifestError}
      onworkspacesrefresh={loadWorkspaces}
      lastSummary={syncLastSummary}
      errorMessage={syncErrorMessage}
      errorCompany={syncErrorCompany}
      {newFilesCount}
      {newFilesList}
      {conflicts}
      {showConflictModal}
      {updateAvailable}
      {updateInstalling}
      {hqCliUpdateAvailable}
      {hqCliUpdateInstalling}
      {hqCliUpdateError}
      {coreState}
      {coreInstalling}
      {coreInstallLastResult}
      {hqVersion}
      onsync={handleSyncNow}
      oncancel={handleCancel}
      onsettings={handleSettings}
      onsignout={handleSignOut}
      onresolve={handleResolveConflict}
      onopen={handleOpenInEditor}
      ondismissconflicts={handleDismissConflicts}
      oninstallupdate={handleInstallUpdate}
      oninstallhqcliupdate={handleInstallHqCliUpdate}
      oninstallcore={handleInstallCore}
      bindStatsRefresh={(fn) => (syncStatsRefresh = fn)}
      {meetingsEnabled}
      onmeetingsclick={() => {
        // Spawn the detached Upcoming Meetings window (label: meetings-window).
        // Fire-and-forget — the Rust handler focuses an existing window if
        // already open, otherwise creates a fresh one. Errors are swallowed
        // since they'd be infra-level (Tauri failure) and there's nothing
        // useful to show inline.
        // Also clear the tray Prompt badge — the user is now acting on any
        // pending meeting detections.
        invoke('open_meetings_window').catch(() => {});
        invoke('meetings_clear_prompt_badge').catch(() => {});
      }}
      {activeMeetings}
      onstartrecording={handleStartRecording}
      onstoprecording={handleStopRecording}
      recordingCompanies={memberships}
      onchangerecordingcompany={handleChangeRecordingCompany}
    />
  {:else}
    <SignInPrompt onsuccess={handleAuthSuccess} />
  {/if}
</main>

<style>
  /* Scoped to the main popover window via `data-window` (set in main.ts)
     so MeetingsWindow's opaque #18181b body bg can't bleed across CSS
     bundle order and turn the transparent popover into a black box. */
  :global(html[data-window='main']),
  :global(html[data-window='main'] body) {
    margin: 0;
    padding: 0;
    width: 100vw;
    height: 100vh;
    /* overflow:hidden prevents scrollbars from appearing on the root
       document. The popover's own scroll container (.popover-body) is
       the only legitimate scrollable region. */
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto,
      Oxygen, Ubuntu, Cantarell, sans-serif;
    /* Transparent so the Popover's rounded corners show the desktop
       behind them (tauri window is transparent). The popover root
       component paints its own background + border-radius. */
    background: transparent;
    color: var(--popover-text, #e0e0e0);
  }

  main {
    /* Fill the window exactly; popover sizes itself via 100vw/100vh.
       No centering flex — that created a sub-viewport box that could
       clip the popover if it ever exceeded window size. */
    width: 100vw;
    height: 100vh;
    padding: 0;
    overflow: hidden;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
  }

  .dot-spinner {
    display: inline-block;
    width: 20px;
    height: 20px;
    border: 2.5px solid var(--popover-progress-track, rgba(255, 255, 255, 0.14));
    border-top-color: var(--popover-progress-fill, #ffffff);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
