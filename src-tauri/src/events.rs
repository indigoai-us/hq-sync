//! Sync event types emitted to the Svelte frontend.
//!
//! The `hq-sync-runner --companies` subprocess emits ndjson lines with a
//! `"type"` field (ADR-0001). We parse each line into a [`SyncEvent`] and
//! re-emit typed Tauri events.
//!
//! Phase 7 (2026-04-19): protocol realigned with `hq-sync-runner`. Previously
//! the menubar spawned `hq sync --json` (never shipped) with a different event
//! shape. The runner now drives this. Legacy `SyncConflictEvent` remains as a
//! no-op stub for frontend compatibility — the runner does not emit per-file
//! conflict events (conflicts are handled inline via `--on-conflict <strategy>`
//! and surface as aborts via `complete.aborted: true`).
//!
//! Source of truth for the protocol:
//!   packages/hq-cloud/src/bin/sync-runner.ts :: `RunnerEvent`

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Individual event payloads (frontend-facing)
// ─────────────────────────────────────────────────────────────────────────────

/// `{type: "auth-error", message}`
/// Emitted when the caller has no valid Cognito token (interactive login
/// disabled in runner mode). Menubar should surface the sign-in CTA.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncAuthErrorEvent {
    pub message: String,
}

/// `{type: "fanout-plan", companies: [{uid, slug}]}`
/// Emitted once per run, after memberships resolve. Lets the UI build a
/// per-company progress column before any `progress` events arrive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncFanoutPlanEvent {
    pub companies: Vec<SyncCompanyRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompanyRef {
    pub uid: String,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
}

/// `{type: "progress", company, path, bytes, message?, direction?, deleted?}`
/// Per-file transfer event. One per file, per company.
///
/// `direction` (hq-cloud ≥5.29.0) is `"up"` when the file was uploaded
/// (push leg) or `"down"` when downloaded (pull leg). `None` on older
/// runners that don't stamp it — the activity log then falls back to
/// treating the event as a download (the historical assumption). `deleted`
/// is `Some(true)` when this event reports a remote delete-marker rather
/// than a transfer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressEvent {
    pub company: String,
    pub path: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
    /// hq-cloud ≥5.29.0: `"up"` (uploaded) | `"down"` (downloaded). None on older runners.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub direction: Option<String>,
    /// True when this event reports a remote DeleteObject (no transfer).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub deleted: Option<bool>,
    /// hq-cloud ≥5.31.0: email of the file's author, read from the S3 object's
    /// `created-by` user-metadata. Only set on download (pull) events — a
    /// downloaded file was authored by whoever uploaded it. None on push events
    /// (the uploader is the local user) and on older runners. The activity log
    /// shows this so the user sees who authored each file they received.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
}

/// `{type: "error", company?, path, message}`
/// Per-file or per-company error. `company` is absent only for discovery-
/// phase failures (before the fanout plan resolved).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncErrorEvent {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub company: Option<String>,
    pub path: String,
    pub message: String,
}

/// Legacy conflict event — kept for frontend-shape compatibility but the
/// runner does not emit per-file conflicts. Menubar infers conflicts from
/// `complete.aborted` and `complete.conflicts > 0`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Legacy no-op stub retained for frontend compatibility — see module doc
pub struct SyncConflictEvent {
    pub path: String,
    pub local_hash: String,
    pub remote_hash: String,
    pub can_auto_resolve: bool,
}

/// `{type: "complete", company, filesDownloaded, bytesDownloaded, filesSkipped, conflicts, aborted, filesTombstoned?, filesRefusedStale?}`
/// Emitted once per company after that company's sync finishes (or aborts).
///
/// `files_tombstoned` (hq-cloud ≥5.24.0) is the count of journal entries
/// dropped because the remote was already 404 at HEAD time (cleaned
/// out-of-band). `files_refused_stale` is the count of delete candidates
/// refused by the `currency-gated` policy because the remote etag drifted.
/// Both are optional so pre-5.24 runners deserialize cleanly with `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompleteEvent {
    pub company: String,
    pub files_downloaded: u32,
    pub bytes_downloaded: u64,
    pub files_skipped: u32,
    pub conflicts: u32,
    pub aborted: bool,
    /// hq-cloud ≥5.24.0. None on older runners.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub files_tombstoned: Option<u32>,
    /// hq-cloud ≥5.24.0. None on older runners.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub files_refused_stale: Option<u32>,
}

/// `{type: "plan", company, filesToDownload, bytesToDownload, filesToUpload, bytesToUpload, filesToSkip, filesToConflict, filesToDelete?}`
/// Stage-1 result from `hq-sync-runner` (≥5.5.0). Emitted once per company
/// per direction — i.e. for `--direction both` a company emits one plan
/// event for the push phase and another for the pull phase. The menubar
/// uses these to compute an accurate progress denominator before
/// transfers start, replacing the Rust-side `count_files_to_transfer`
/// pre-pass that lived inline in `start_sync`. When connected to an
/// older runner that doesn't emit `plan`, the pre-pass remains as a
/// fallback (it computed the upload count only and never the
/// pull-side count, so plan events strictly improve accuracy).
///
/// `files_to_delete` (hq-cloud ≥5.24.0) is the push-only count of
/// journal entries scheduled for remote `DeleteObject`. Optional so
/// older runners (which don't emit the field) deserialize cleanly with
/// `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlanEvent {
    pub company: String,
    pub files_to_download: u32,
    pub bytes_to_download: u64,
    pub files_to_upload: u32,
    pub bytes_to_upload: u64,
    pub files_to_skip: u32,
    pub files_to_conflict: u32,
    /// hq-cloud ≥5.24.0. None on older runners.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub files_to_delete: Option<u32>,
}

/// `{type: "delete-refused-stale-etag", company, path, journalEtag, remoteEtag, reason}`
/// Emitted by hq-cloud ≥5.24.0 when the `currency-gated` delete policy
/// refuses to propagate a local deletion to S3 because:
///   - `reason: "stale-etag"` — the remote object's current ETag no longer
///     matches the journal's recorded one. Some other device modified the
///     file since this machine last synced; the pull leg re-pulls
///     naturally via `hasRemoteChanged`. `journalEtag` and `remoteEtag`
///     are real values.
///   - `reason: "legacy-no-etag"` — the journal entry predates remoteEtag
///     tracking (no etag to compare against). Refused in the safe
///     direction; a future sync that picks up an etag can re-evaluate.
///     `journalEtag` and `remoteEtag` are sentinel strings — do not
///     display them as ETags. Branch on `reason`.
///
/// Frontend treatment: this is operationally informative (peer drift /
/// migration), not an error. Renderers should surface it as a warning
/// row ("kept on remote: <path>") rather than a hard failure. Older
/// runners (<5.24.0) never emit this; absence is the silent default.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncDeleteRefusedStaleEtagEvent {
    pub company: String,
    pub path: String,
    pub journal_etag: String,
    pub remote_etag: String,
    pub reason: String,
}

/// `{type: "new-files", company, files: [{path, bytes, addedBy?}]}`
/// Emitted once per company after sync completes, listing files that were
/// newly added to the shared drive since the last sync. `addedBy` is the
/// email of the person who uploaded the file (null when attribution is
/// unavailable).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncNewFileEntry {
    pub path: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub added_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncNewFilesEvent {
    pub company: String,
    pub files: Vec<SyncNewFileEntry>,
}

/// `{type: "all-complete", companiesAttempted, filesDownloaded, bytesDownloaded, errors}`
/// Terminal event. Emitted exactly once after the fanout loop finishes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncAllCompleteEvent {
    pub companies_attempted: u32,
    pub files_downloaded: u32,
    pub bytes_downloaded: u64,
    pub errors: Vec<SyncCompanyError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompanyError {
    pub company: String,
    pub message: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Discriminated union for ndjson parsing
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SyncEvent {
    /// Caller is signed in but has no person entity yet — menubar should
    /// surface the onboarding flow.
    SetupNeeded,
    AuthError(SyncAuthErrorEvent),
    FanoutPlan(SyncFanoutPlanEvent),
    /// Stage-1 result for a single company / direction. Optional in the
    /// protocol — older runners (hq-cloud <5.5.0) skip it. When present,
    /// arrives before any `Progress` events for that company.
    Plan(SyncPlanEvent),
    Progress(SyncProgressEvent),
    Error(SyncErrorEvent),
    Complete(SyncCompleteEvent),
    /// hq-cloud ≥5.24.0. Emitted only under the `currency-gated` delete
    /// policy (opt-in via `HQ_SYNC_DELETE_POLICY=currency-gated` in 5.24;
    /// default in 5.25+). Older runners never emit this variant.
    DeleteRefusedStaleEtag(SyncDeleteRefusedStaleEtagEvent),
    NewFiles(SyncNewFilesEvent),
    AllComplete(SyncAllCompleteEvent),
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri event channel names
// ─────────────────────────────────────────────────────────────────────────────

pub const EVENT_SYNC_SETUP_NEEDED: &str = "sync:setup-needed";
pub const EVENT_SYNC_AUTH_ERROR: &str = "sync:auth-error";
pub const EVENT_SYNC_FANOUT_PLAN: &str = "sync:fanout-plan";
/// Per-company / per-direction Stage-1 result from the runner (≥hq-cloud@5.5.0).
/// Frontend uses these to refine the progress denominator established
/// by the upstream `EVENT_SYNC_TOTALS` pre-pass.
pub const EVENT_SYNC_PLAN: &str = "sync:plan";
pub const EVENT_SYNC_PROGRESS: &str = "sync:progress";
pub const EVENT_SYNC_ERROR: &str = "sync:error";
pub const EVENT_SYNC_COMPLETE: &str = "sync:complete";
/// hq-cloud ≥5.24.0. Emitted only under `currency-gated` delete policy
/// (opt-in 5.24, default 5.25+). Renderers should surface as an
/// informational warning ("kept on remote: <path>") rather than an
/// error — the file is intentionally preserved on S3 because peer drift
/// or a missing journal etag made the delete unsafe to propagate.
pub const EVENT_SYNC_DELETE_REFUSED_STALE_ETAG: &str = "sync:delete-refused-stale-etag";
pub const EVENT_SYNC_NEW_FILES: &str = "sync:new-files";
pub const EVENT_SYNC_ALL_COMPLETE: &str = "sync:all-complete";
/// Deprecated — kept for frontend shape-compat. Not emitted by the runner.
pub const EVENT_SYNC_CONFLICT: &str = "sync:conflict";
/// Emitted once per newly-provisioned company after `provision_missing_companies` succeeds.
pub const EVENT_SYNC_COMPANY_PROVISIONED: &str = "sync:company-provisioned";

/// Payload for `EVENT_SYNC_COMPANY_PROVISIONED`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompanyProvisionedEvent {
    pub company_uid: String,
    pub company_slug: String,
    pub bucket_name: String,
}

/// Emitted once per file during the first-push upload phase.
pub const EVENT_SYNC_COMPANY_FIRST_PUSH_PROGRESS: &str = "sync:company-first-push-progress";

/// Emitted once per company after its first-push upload completes.
pub const EVENT_SYNC_COMPANY_FIRST_PUSH_COMPLETE: &str = "sync:company-first-push-complete";

/// Emitted when `first_push_company` returns an error. Release-mode callers
/// receive no other signal on failure, so this is the only error surface.
pub const EVENT_SYNC_COMPANY_FIRST_PUSH_FAILED: &str = "sync:company-first-push-failed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompanyFirstPushProgressEvent {
    pub company_uid: String,
    pub company_slug: String,
    pub files_done: usize,
    pub files_total: usize,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompanyFirstPushCompleteEvent {
    pub company_uid: String,
    pub company_slug: String,
    pub files_uploaded: usize,
    pub files_skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompanyFirstPushFailedEvent {
    pub company_uid: String,
    pub company_slug: String,
    pub error: String,
}

// ── Personal first-push events ────────────────────────────────────────────────

pub const EVENT_SYNC_PERSONAL_PROVISIONED: &str = "sync:personal-provisioned";
pub const EVENT_SYNC_PERSONAL_FIRST_PUSH_PROGRESS: &str = "sync:personal-first-push-progress";
pub const EVENT_SYNC_PERSONAL_FIRST_PUSH_COMPLETE: &str = "sync:personal-first-push-complete";
pub const EVENT_SYNC_PERSONAL_SKIPPED_OWNERSHIP_MISMATCH: &str =
    "sync:personal-skipped-ownership-mismatch";
pub const EVENT_SYNC_PERSONAL_FIRST_PUSH_SKIPPED: &str = "sync:personal-first-push-skipped";

/// Pre-walk total — emitted once after the Rust pre-walk and before the
/// runner spawns. Carries the count of files we expect to process across
/// the entire sync (personal allowlist + every company folder, after
/// applying the .hqignore + DEFAULT_IGNORES filter). The UI uses this as
/// the denominator for a real per-file progress bar.
pub const EVENT_SYNC_TOTALS: &str = "sync:totals";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPersonalProvisionedEvent {
    pub person_uid: String,
    pub bucket_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPersonalFirstPushProgressEvent {
    pub person_uid: String,
    pub files_done: usize,
    pub files_total: usize,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPersonalFirstPushCompleteEvent {
    pub person_uid: String,
    pub files_uploaded: usize,
    pub files_skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPersonalSkippedOwnershipMismatchEvent {
    pub person_uid: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPersonalFirstPushSkippedEvent {
    pub person_uid: String,
    pub path: String,
    pub reason: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Meeting detection events (Recall Desktop SDK → Svelte)
// ─────────────────────────────────────────────────────────────────────────────

/// Platform discriminator for `MeetingDetectedEvent`.
///
/// Matches the platforms the Recall Desktop SDK can detect. `Other` is a
/// catch-all for any platform string the SDK emits that we have not
/// explicitly enumerated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MeetingPlatform {
    Zoom,
    Meet,
    Teams,
    Slack,
    Webex,
    Other,
}

/// Detection source: SDK detected via a calendar-imminent event, or via the
/// active-app detector (the user launched a meeting client).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum DetectionSource {
    SdkCalendar,
    SdkActiveApp,
}

/// Payload for the `meeting:detected` Tauri event forwarded from the
/// Recall Desktop SDK.
///
/// The SDK emits this on its stdout as ndjson. Recall's server-side SDK
/// documentation and our hq-sync sidecar contract agree on this shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetectedEvent {
    /// Recall-assigned stable ID for this detection (used for dedup).
    pub detection_id: String,
    /// The meeting URL (Zoom, Meet, Teams, etc.) that was detected.
    pub meeting_url: String,
    /// SDK window id for the detected meeting. Canonical handle for
    /// `RecallAiSdk.startRecording({ windowId })` and `meeting-closed`
    /// matching. Optional for back-compat with older bridge versions
    /// that only emitted `meetingUrl`; current bridge always populates.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub window_id: Option<String>,
    /// The video-conferencing platform.
    pub platform: MeetingPlatform,
    /// ISO 8601 timestamp when the detection fired.
    pub detected_at: String,
    /// Whether the detection came from a calendar event or the active app.
    pub source: DetectionSource,
    /// Optional: the calendar event id this detection was derived from
    /// (present for `sdk-calendar` detections; absent for `sdk-active-app`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_event_id: Option<String>,
}

/// Tauri event channel name for `MeetingDetectedEvent`.
pub const EVENT_MEETING_DETECTED: &str = "meeting:detected";

/// Payload for `meeting:closed` — emitted by the SDK when the meeting
/// window goes away (user quit the app, Zoom call ended, Slack huddle
/// closed, etc.). Used by the UI to clear an active-meeting row that
/// was never recorded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingClosedEvent {
    pub window_id: String,
    pub platform: MeetingPlatform,
    pub closed_at: String,
}

/// Tauri event channel name for `MeetingClosedEvent`.
pub const EVENT_MEETING_CLOSED: &str = "meeting:closed";

/// Payload for `notification:meeting-action` — emitted by the Rust side
/// when the user interacts with a "Meeting detected" macOS notification.
///
/// Fired in two cases:
/// - User clicked the notification body itself (`action: "open"`)
/// - User clicked the "Record" action button (`action: "record"`)
///
/// The Svelte side routes these to the popover or
/// `invoke('start_recording', { windowId })` respectively. `windowId`
/// is the SDK window the notification was emitted for; the renderer
/// uses it to look up the corresponding `ActiveMeeting` row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationMeetingActionEvent {
    /// `"open"` — user clicked the notification body; the renderer should
    /// open/focus the popover so the row is visible.
    /// `"record"` — user clicked the Record action button; the renderer
    /// should call `start_recording(windowId)` directly.
    pub action: String,
    /// SDK window id for the meeting the notification was emitted for.
    /// Empty when the notification predates per-meeting bookkeeping
    /// (defensive — shouldn't happen in practice).
    pub window_id: String,
    /// Platform discriminator from the original detection (lowercase).
    pub platform: String,
}

/// Tauri event channel name for `NotificationMeetingActionEvent`.
pub const EVENT_NOTIFICATION_MEETING_ACTION: &str = "notification:meeting-action";

// ─────────────────────────────────────────────────────────────────────────────
// Recording lifecycle events
// ─────────────────────────────────────────────────────────────────────────────

/// Payload for `recording:started` — emitted by the SDK bridge after
/// `RecallAiSdk.startRecording({ windowId, uploadToken })` resolves
/// successfully. The `windowId` keys back to the detection that
/// triggered the recording so the Svelte side can flip the UI to
/// "Recording…" for the matching row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStartedEvent {
    /// SDK window id for the recording.
    pub window_id: String,
    /// Platform discriminator (`zoom`, `meet`, …). Same enum as detections.
    pub platform: MeetingPlatform,
    /// ISO 8601 timestamp when the bridge confirmed the start.
    pub started_at: String,
}

/// Payload for `recording:ended` — emitted when the SDK ends the recording.
/// Normally triggered by an explicit `stopRecording`: either a user Stop, or
/// the bridge auto-stopping on `meeting-closed` when the call ends (host ends
/// it / everyone leaves). The SDK does NOT reliably auto-stop on its own when
/// the meeting window closes — its CHANGELOG documents per-platform auto-stop
/// as unreliable — so the bridge issues the stop explicitly (see the
/// `meeting-closed` handler in sidecar/recall-sdk-bridge/bridge.mjs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEndedEvent {
    /// SDK window id for the recording that ended.
    pub window_id: String,
    /// Platform discriminator (`zoom`, `meet`, …).
    pub platform: MeetingPlatform,
    /// ISO 8601 timestamp when the bridge confirmed the end.
    pub ended_at: String,
}

/// Payload for `recording:media-capture` — emitted by the SDK as it
/// latches onto audio / video sources after `startRecording`. Useful
/// for distinguishing "recording started but no audio yet" (likely a
/// TCC issue) from "recording fully live".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingMediaCaptureEvent {
    pub window_id: String,
    /// One of `audio`, `video`, `screenshare` from the SDK.
    pub capture_type: String,
    /// True when the source is actively streaming; false when it stops.
    pub capturing: bool,
}

/// Payload for `recording:error` — emitted by the bridge when a
/// start/stop command throws inside the SDK. Carries the command
/// name + windowId + a human-readable message so the UI can surface a
/// specific failure instead of just spinning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingErrorEvent {
    /// The bridge command that failed (`start-recording`, `stop-recording`, …).
    pub cmd: String,
    pub window_id: String,
    /// Human-readable error message from the SDK.
    pub message: String,
}

/// Tauri event channel name for `RecordingStartedEvent`.
pub const EVENT_RECORDING_STARTED: &str = "recording:started";
/// Tauri event channel name for `RecordingEndedEvent`.
pub const EVENT_RECORDING_ENDED: &str = "recording:ended";
/// Tauri event channel name for `RecordingMediaCaptureEvent`.
pub const EVENT_RECORDING_MEDIA_CAPTURE: &str = "recording:media-capture";
/// Tauri event channel name for `RecordingErrorEvent`.
pub const EVENT_RECORDING_ERROR: &str = "recording:error";

// ─────────────────────────────────────────────────────────────────────────────
// Permission events (US-011)
// ─────────────────────────────────────────────────────────────────────────────

/// One of the five macOS permissions the Recall Desktop SDK can request.
/// Mirrors `Permission` in @recallai/desktop-sdk's index.d.ts (kebab-case).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RecallPermission {
    Accessibility,
    ScreenCapture,
    Microphone,
    SystemAudio,
    FullDiskAccess,
}

/// Per-permission status emitted by the SDK on init and whenever the user
/// changes it in System Settings. Status strings the SDK uses:
/// `granted`, `denied`, `not-determined`, and (rarely) `restricted`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatusEvent {
    pub permission: RecallPermission,
    /// Free-form so we don't break on a new SDK status value; the UI treats
    /// anything other than "granted" as missing.
    pub status: String,
}

/// Tauri event channel name for `PermissionStatusEvent`.
pub const EVENT_PERMISSION_STATUS: &str = "permission:status";

/// Convenience signal — all required permissions have been granted; the UI
/// can collapse the missing-permissions banner without checking individual
/// rows. No payload.
pub const EVENT_PERMISSIONS_ALL_GRANTED: &str = "permissions:all-granted";

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_setup_needed_event() {
        let json = r#"{"type":"setup-needed"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event, SyncEvent::SetupNeeded);
    }

    #[test]
    fn test_parse_auth_error_event() {
        let json = r#"{"type":"auth-error","message":"Token expired"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::AuthError(SyncAuthErrorEvent {
                message: "Token expired".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_fanout_plan_event() {
        let json = r#"{"type":"fanout-plan","companies":[{"uid":"cmp_1","slug":"indigo"},{"uid":"cmp_2","slug":"voyage"}]}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::FanoutPlan(SyncFanoutPlanEvent {
                companies: vec![
                    SyncCompanyRef {
                        uid: "cmp_1".to_string(),
                        slug: "indigo".to_string(),
                        name: None,
                    },
                    SyncCompanyRef {
                        uid: "cmp_2".to_string(),
                        slug: "voyage".to_string(),
                        name: None,
                    },
                ],
            })
        );
    }

    #[test]
    fn test_parse_fanout_plan_event_with_names() {
        let json = r#"{"type":"fanout-plan","companies":[{"uid":"cmp_1","slug":"indigo","name":"Indigo"},{"uid":"cmp_2","slug":"voyage"}]}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::FanoutPlan(SyncFanoutPlanEvent {
                companies: vec![
                    SyncCompanyRef {
                        uid: "cmp_1".to_string(),
                        slug: "indigo".to_string(),
                        name: Some("Indigo".to_string()),
                    },
                    SyncCompanyRef {
                        uid: "cmp_2".to_string(),
                        slug: "voyage".to_string(),
                        name: None,
                    },
                ],
            })
        );
    }

    #[test]
    fn test_company_ref_skips_none_name() {
        let c = SyncCompanyRef {
            uid: "cmp_1".to_string(),
            slug: "indigo".to_string(),
            name: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("\"name\""));
    }

    #[test]
    fn test_parse_progress_event_with_message() {
        let json = r#"{"type":"progress","company":"indigo","path":"docs/a.md","bytes":42,"message":"shared by M1"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Progress(SyncProgressEvent {
                company: "indigo".to_string(),
                path: "docs/a.md".to_string(),
                bytes: 42,
                message: Some("shared by M1".to_string()),
                direction: None,
                deleted: None,
                author: None,
            })
        );
    }

    #[test]
    fn test_parse_progress_event_without_message() {
        let json = r#"{"type":"progress","company":"indigo","path":"docs/a.md","bytes":42}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Progress(SyncProgressEvent {
                company: "indigo".to_string(),
                path: "docs/a.md".to_string(),
                bytes: 42,
                message: None,
                direction: None,
                deleted: None,
                author: None,
            })
        );
    }

    #[test]
    fn test_parse_progress_event_with_author() {
        // hq-cloud ≥5.31.0 stamps `author` (from S3 `created-by`) on download
        // (pull) progress events so the activity log can attribute the file.
        let json = r#"{"type":"progress","company":"indigo","path":"docs/a.md","bytes":42,"direction":"down","author":"alice@example.com"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::Progress(p) => {
                assert_eq!(p.author, Some("alice@example.com".to_string()));
                assert_eq!(p.direction, Some("down".to_string()));
            }
            _ => panic!("expected Progress"),
        }
    }

    #[test]
    fn test_parse_progress_event_with_direction() {
        // hq-cloud ≥5.29.0 stamps direction (and may carry deleted).
        let json = r#"{"type":"progress","company":"indigo","path":"docs/a.md","bytes":42,"direction":"up"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::Progress(p) => {
                assert_eq!(p.direction, Some("up".to_string()));
                assert_eq!(p.deleted, None);
            }
            _ => panic!("expected Progress"),
        }
    }

    #[test]
    fn test_parse_error_event_with_company() {
        let json = r#"{"type":"error","company":"indigo","path":"docs/x.md","message":"Access denied"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Error(SyncErrorEvent {
                company: Some("indigo".to_string()),
                path: "docs/x.md".to_string(),
                message: "Access denied".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_error_event_without_company() {
        // Discovery-phase errors (before fanout-plan) have no company.
        let json = r#"{"type":"error","path":"(discovery)","message":"Vault unreachable"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Error(SyncErrorEvent {
                company: None,
                path: "(discovery)".to_string(),
                message: "Vault unreachable".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_complete_event() {
        let json = r#"{"type":"complete","company":"indigo","filesDownloaded":5,"bytesDownloaded":102400,"filesSkipped":2,"conflicts":0,"aborted":false}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Complete(SyncCompleteEvent {
                company: "indigo".to_string(),
                files_downloaded: 5,
                bytes_downloaded: 102400,
                files_skipped: 2,
                conflicts: 0,
                aborted: false,
                // Pre-5.24 runners don't emit these — None mirrors the
                // wire absence and keeps the assert checking that the
                // optional defaults take effect for older payloads.
                files_tombstoned: None,
                files_refused_stale: None,
            })
        );
    }

    #[test]
    fn test_parse_complete_event_aborted_on_conflict() {
        let json = r#"{"type":"complete","company":"indigo","filesDownloaded":0,"bytesDownloaded":0,"filesSkipped":0,"conflicts":1,"aborted":true}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::Complete(c) => {
                assert!(c.aborted);
                assert_eq!(c.conflicts, 1);
            }
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn test_parse_all_complete_event() {
        let json = r#"{"type":"all-complete","companiesAttempted":2,"filesDownloaded":7,"bytesDownloaded":204800,"errors":[{"company":"voyage","message":"timeout"}]}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::AllComplete(SyncAllCompleteEvent {
                companies_attempted: 2,
                files_downloaded: 7,
                bytes_downloaded: 204800,
                errors: vec![SyncCompanyError {
                    company: "voyage".to_string(),
                    message: "timeout".to_string(),
                }],
            })
        );
    }

    #[test]
    fn test_parse_all_complete_event_empty_errors() {
        let json = r#"{"type":"all-complete","companiesAttempted":1,"filesDownloaded":0,"bytesDownloaded":0,"errors":[]}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::AllComplete(a) => assert!(a.errors.is_empty()),
            _ => panic!("expected AllComplete"),
        }
    }

    #[test]
    fn test_parse_new_files_event() {
        let json = r#"{"type":"new-files","company":"indigo","files":[{"path":"docs/new.md","bytes":1024,"addedBy":"stefan@example.com"},{"path":"docs/other.md","bytes":512,"addedBy":null}]}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::NewFiles(SyncNewFilesEvent {
                company: "indigo".to_string(),
                files: vec![
                    SyncNewFileEntry {
                        path: "docs/new.md".to_string(),
                        bytes: 1024,
                        added_by: Some("stefan@example.com".to_string()),
                    },
                    SyncNewFileEntry {
                        path: "docs/other.md".to_string(),
                        bytes: 512,
                        added_by: None,
                    },
                ],
            })
        );
    }

    #[test]
    fn test_parse_new_files_event_empty_files() {
        let json = r#"{"type":"new-files","company":"voyage","files":[]}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::NewFiles(nf) => {
                assert_eq!(nf.company, "voyage");
                assert!(nf.files.is_empty());
            }
            _ => panic!("expected NewFiles"),
        }
    }

    #[test]
    fn test_parse_new_files_event_without_added_by_key() {
        // addedBy omitted entirely (not just null) — must default to None.
        let json = r#"{"type":"new-files","company":"indigo","files":[{"path":"a.txt","bytes":100}]}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::NewFiles(nf) => {
                assert_eq!(nf.files.len(), 1);
                assert_eq!(nf.files[0].added_by, None);
            }
            _ => panic!("expected NewFiles"),
        }
    }

    #[test]
    fn test_new_files_event_roundtrip() {
        let event = SyncEvent::NewFiles(SyncNewFilesEvent {
            company: "indigo".to_string(),
            files: vec![SyncNewFileEntry {
                path: "docs/a.md".to_string(),
                bytes: 42,
                added_by: Some("user@example.com".to_string()),
            }],
        });
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SyncEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn test_new_file_entry_skips_none_added_by() {
        let entry = SyncNewFileEntry {
            path: "a.txt".to_string(),
            bytes: 10,
            added_by: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("\"addedBy\""));
    }

    #[test]
    fn test_unknown_event_type_fails_gracefully() {
        let json = r#"{"type":"unknown","foo":"bar"}"#;
        let result: Result<SyncEvent, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_json_fails() {
        let json = r#"not valid json"#;
        let result: Result<SyncEvent, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // ── MeetingDetectedEvent ──────────────────────────────────────────────────

    #[test]
    fn test_meeting_detected_event_round_trips() {
        let payload = MeetingDetectedEvent {
            detection_id: "det_123".to_string(),
            meeting_url: "https://zoom.us/j/12345".to_string(),
            window_id: Some("win-abc".to_string()),
            platform: MeetingPlatform::Zoom,
            detected_at: "2026-05-20T10:00:00Z".to_string(),
            source: DetectionSource::SdkCalendar,
            source_event_id: Some("evt_abc".to_string()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: MeetingDetectedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, parsed);
    }

    #[test]
    fn test_meeting_detected_event_omits_none_source_event_id() {
        let payload = MeetingDetectedEvent {
            detection_id: "det_456".to_string(),
            meeting_url: "https://meet.google.com/abc-def-ghi".to_string(),
            window_id: None,
            platform: MeetingPlatform::Meet,
            detected_at: "2026-05-20T10:00:00Z".to_string(),
            source: DetectionSource::SdkActiveApp,
            source_event_id: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(!json.contains("\"sourceEventId\""));
        assert!(json.contains("\"platform\":\"meet\""));
        assert!(json.contains("\"source\":\"sdk-active-app\""));
    }

    #[test]
    fn test_meeting_detected_event_parses_camel_case_from_sdk() {
        let json = r#"{
            "detectionId": "det_789",
            "meetingUrl": "https://teams.microsoft.com/l/meetup-join/abc",
            "platform": "teams",
            "detectedAt": "2026-05-20T11:00:00Z",
            "source": "sdk-calendar",
            "sourceEventId": "cal_evt_99"
        }"#;
        let payload: MeetingDetectedEvent = serde_json::from_str(json).unwrap();
        assert_eq!(payload.detection_id, "det_789");
        assert_eq!(payload.platform, MeetingPlatform::Teams);
        assert_eq!(payload.source, DetectionSource::SdkCalendar);
        assert_eq!(payload.source_event_id.as_deref(), Some("cal_evt_99"));
    }

    #[test]
    fn test_progress_event_serializes_camel_case() {
        let event = SyncProgressEvent {
            company: "indigo".to_string(),
            path: "docs/a.md".to_string(),
            bytes: 42,
            message: None,
            direction: None,
            deleted: None,
            author: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        // `message: None` / `direction: None` / `deleted: None` / `author: None` must not serialize.
        assert!(!json.contains("\"message\""));
        assert!(!json.contains("\"direction\""));
        assert!(!json.contains("\"deleted\""));
        assert!(!json.contains("\"author\""));
        assert!(json.contains("\"company\""));
        assert!(json.contains("\"path\""));
        assert!(json.contains("\"bytes\""));
    }

    #[test]
    fn test_all_complete_event_roundtrip() {
        let event = SyncEvent::AllComplete(SyncAllCompleteEvent {
            companies_attempted: 3,
            files_downloaded: 10,
            bytes_downloaded: 999999,
            errors: vec![],
        });
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SyncEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn test_setup_needed_serializes_as_bare_type() {
        let event = SyncEvent::SetupNeeded;
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"type":"setup-needed"}"#);
    }

    // ── hq-cloud 5.24.0 — new event variant + optional fields ──────────────

    #[test]
    fn test_parse_delete_refused_stale_etag_event_with_real_etags() {
        let json = r#"{"type":"delete-refused-stale-etag","company":"indigo","path":"shared.md","journalEtag":"abc123","remoteEtag":"def456","reason":"stale-etag"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::DeleteRefusedStaleEtag(SyncDeleteRefusedStaleEtagEvent {
                company: "indigo".to_string(),
                path: "shared.md".to_string(),
                journal_etag: "abc123".to_string(),
                remote_etag: "def456".to_string(),
                reason: "stale-etag".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_delete_refused_stale_etag_legacy_no_etag_reason() {
        // hq-cloud emits sentinel strings for the legacy-no-etag branch.
        // Renderers must branch on `reason`, not on the etag values.
        let json = r#"{"type":"delete-refused-stale-etag","company":"personal","path":"legacy.md","journalEtag":"<legacy-no-etag>","remoteEtag":"<unknown>","reason":"legacy-no-etag"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::DeleteRefusedStaleEtag(e) => {
                assert_eq!(e.reason, "legacy-no-etag");
                assert_eq!(e.journal_etag, "<legacy-no-etag>");
            }
            _ => panic!("expected DeleteRefusedStaleEtag"),
        }
    }

    #[test]
    fn test_parse_plan_event_with_files_to_delete() {
        // hq-cloud ≥5.24.0 emits filesToDelete in the plan event.
        let json = r#"{"type":"plan","company":"indigo","filesToDownload":0,"bytesToDownload":0,"filesToUpload":3,"bytesToUpload":1024,"filesToSkip":5,"filesToConflict":0,"filesToDelete":2}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::Plan(p) => {
                assert_eq!(p.files_to_delete, Some(2));
                assert_eq!(p.files_to_upload, 3);
            }
            _ => panic!("expected Plan"),
        }
    }

    #[test]
    fn test_parse_plan_event_without_files_to_delete_back_compat() {
        // Pre-5.24 runners omit filesToDelete entirely. Must default to None.
        let json = r#"{"type":"plan","company":"indigo","filesToDownload":1,"bytesToDownload":100,"filesToUpload":0,"bytesToUpload":0,"filesToSkip":2,"filesToConflict":0}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::Plan(p) => assert_eq!(p.files_to_delete, None),
            _ => panic!("expected Plan"),
        }
    }

    #[test]
    fn test_parse_complete_event_with_5_24_counters() {
        let json = r#"{"type":"complete","company":"indigo","filesDownloaded":0,"bytesDownloaded":0,"filesSkipped":1,"conflicts":0,"aborted":false,"filesTombstoned":3,"filesRefusedStale":1}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::Complete(c) => {
                assert_eq!(c.files_tombstoned, Some(3));
                assert_eq!(c.files_refused_stale, Some(1));
            }
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn test_parse_complete_event_pre_5_24_back_compat() {
        // Pre-5.24 runners omit the counters; both must default to None.
        let json = r#"{"type":"complete","company":"indigo","filesDownloaded":1,"bytesDownloaded":10,"filesSkipped":0,"conflicts":0,"aborted":false}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        match event {
            SyncEvent::Complete(c) => {
                assert_eq!(c.files_tombstoned, None);
                assert_eq!(c.files_refused_stale, None);
            }
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn test_delete_refused_stale_etag_roundtrip() {
        let event = SyncEvent::DeleteRefusedStaleEtag(SyncDeleteRefusedStaleEtagEvent {
            company: "indigo".to_string(),
            path: "shared.md".to_string(),
            journal_etag: "old".to_string(),
            remote_etag: "new".to_string(),
            reason: "stale-etag".to_string(),
        });
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SyncEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }

    #[test]
    fn test_plan_event_skips_none_files_to_delete() {
        // `files_to_delete: None` must NOT serialize — keeps the on-wire
        // shape identical to what a pre-5.24 runner emits, so any external
        // consumer that handles both runner versions sees consistent input.
        let plan = SyncPlanEvent {
            company: "indigo".to_string(),
            files_to_download: 0,
            bytes_to_download: 0,
            files_to_upload: 0,
            bytes_to_upload: 0,
            files_to_skip: 0,
            files_to_conflict: 0,
            files_to_delete: None,
        };
        let json = serde_json::to_string(&plan).unwrap();
        assert!(!json.contains("filesToDelete"));
    }

    #[test]
    fn test_complete_event_skips_none_counters() {
        let c = SyncCompleteEvent {
            company: "indigo".to_string(),
            files_downloaded: 0,
            bytes_downloaded: 0,
            files_skipped: 0,
            conflicts: 0,
            aborted: false,
            files_tombstoned: None,
            files_refused_stale: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("filesTombstoned"));
        assert!(!json.contains("filesRefusedStale"));
    }
}
