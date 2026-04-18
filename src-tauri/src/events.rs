//! Sync event types emitted to the Svelte frontend.
//!
//! The `hq sync --json` subprocess emits ndjson lines with a `"type"` field.
//! We parse each line into a [`SyncEvent`] and re-emit typed Tauri events.

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Individual event payloads (frontend-facing)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressEvent {
    pub phase: String,
    pub files_complete: u32,
    pub files_total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflictEvent {
    pub path: String,
    pub local_hash: String,
    pub remote_hash: String,
    pub can_auto_resolve: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncErrorEvent {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompleteEvent {
    pub files_changed: u32,
    pub bytes_transferred: u64,
    pub journal_path: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Discriminated union for ndjson parsing
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SyncEvent {
    #[serde(rename = "progress")]
    Progress(SyncProgressEvent),
    #[serde(rename = "conflict")]
    Conflict(SyncConflictEvent),
    #[serde(rename = "error")]
    Error(SyncErrorEvent),
    #[serde(rename = "complete")]
    Complete(SyncCompleteEvent),
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri event channel names
// ─────────────────────────────────────────────────────────────────────────────

pub const EVENT_SYNC_PROGRESS: &str = "sync:progress";
pub const EVENT_SYNC_CONFLICT: &str = "sync:conflict";
pub const EVENT_SYNC_ERROR: &str = "sync:error";
pub const EVENT_SYNC_COMPLETE: &str = "sync:complete";

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_progress_event() {
        let json = r#"{"type":"progress","phase":"uploading","filesComplete":3,"filesTotal":10}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Progress(SyncProgressEvent {
                phase: "uploading".to_string(),
                files_complete: 3,
                files_total: 10,
            })
        );
    }

    #[test]
    fn test_parse_conflict_event() {
        let json = r#"{"type":"conflict","path":"docs/readme.md","localHash":"abc123","remoteHash":"def456","canAutoResolve":false}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Conflict(SyncConflictEvent {
                path: "docs/readme.md".to_string(),
                local_hash: "abc123".to_string(),
                remote_hash: "def456".to_string(),
                can_auto_resolve: false,
            })
        );
    }

    #[test]
    fn test_parse_error_event() {
        let json = r#"{"type":"error","code":"AUTH_EXPIRED","message":"Token expired"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Error(SyncErrorEvent {
                code: "AUTH_EXPIRED".to_string(),
                message: "Token expired".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_complete_event() {
        let json = r#"{"type":"complete","filesChanged":5,"bytesTransferred":102400,"journalPath":"/tmp/sync.journal"}"#;
        let event: SyncEvent = serde_json::from_str(json).unwrap();
        assert_eq!(
            event,
            SyncEvent::Complete(SyncCompleteEvent {
                files_changed: 5,
                bytes_transferred: 102400,
                journal_path: "/tmp/sync.journal".to_string(),
            })
        );
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

    #[test]
    fn test_progress_event_serializes_camel_case() {
        let event = SyncProgressEvent {
            phase: "downloading".to_string(),
            files_complete: 1,
            files_total: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"filesComplete\""));
        assert!(json.contains("\"filesTotal\""));
    }

    #[test]
    fn test_sync_event_roundtrip() {
        let event = SyncEvent::Complete(SyncCompleteEvent {
            files_changed: 10,
            bytes_transferred: 999999,
            journal_path: "/var/log/sync.journal".to_string(),
        });
        let json = serde_json::to_string(&event).unwrap();
        let parsed: SyncEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }
}
