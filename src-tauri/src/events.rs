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

/// `{type: "progress", company, path, bytes, message?}`
/// Per-file download event. One per file, per company.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressEvent {
    pub company: String,
    pub path: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
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
pub struct SyncConflictEvent {
    pub path: String,
    pub local_hash: String,
    pub remote_hash: String,
    pub can_auto_resolve: bool,
}

/// `{type: "complete", company, filesDownloaded, bytesDownloaded, filesSkipped, conflicts, aborted}`
/// Emitted once per company after that company's sync finishes (or aborts).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompleteEvent {
    pub company: String,
    pub files_downloaded: u32,
    pub bytes_downloaded: u64,
    pub files_skipped: u32,
    pub conflicts: u32,
    pub aborted: bool,
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
    Progress(SyncProgressEvent),
    Error(SyncErrorEvent),
    Complete(SyncCompleteEvent),
    AllComplete(SyncAllCompleteEvent),
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri event channel names
// ─────────────────────────────────────────────────────────────────────────────

pub const EVENT_SYNC_SETUP_NEEDED: &str = "sync:setup-needed";
pub const EVENT_SYNC_AUTH_ERROR: &str = "sync:auth-error";
pub const EVENT_SYNC_FANOUT_PLAN: &str = "sync:fanout-plan";
pub const EVENT_SYNC_PROGRESS: &str = "sync:progress";
pub const EVENT_SYNC_ERROR: &str = "sync:error";
pub const EVENT_SYNC_COMPLETE: &str = "sync:complete";
pub const EVENT_SYNC_ALL_COMPLETE: &str = "sync:all-complete";
/// Deprecated — kept for frontend shape-compat. Not emitted by the runner.
pub const EVENT_SYNC_CONFLICT: &str = "sync:conflict";

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
            })
        );
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
            company: "indigo".to_string(),
            path: "docs/a.md".to_string(),
            bytes: 42,
            message: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        // `message: None` must not serialize.
        assert!(!json.contains("\"message\""));
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
}
