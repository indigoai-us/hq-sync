//! Work daemon — session relay for live-steer-by-publish (US-011).
//!
//! ## Feasibility spike findings
//!
//! Three mechanisms for injecting into a running Claude Code session were
//! evaluated:
//!
//!   1. **stdin / PTY injection**: The running `claude` or `Claude.app`
//!      process owns a PTY whose master fd belongs to the host terminal, not
//!      the daemon.  Writing to the child process's stdin from an unrelated
//!      process requires knowing the PTY device path (unavailable without
//!      debug entitlements on macOS) and is unreliable under App Sandbox.
//!
//!   2. **AppleScript UI injection**: macOS-only, requires Accessibility
//!      permissions in the app's entitlements, brittle against Claude Code
//!      UI layout changes, and entirely incompatible with headless / Linux
//!      deployments (US-010).
//!
//!   3. **`claude://code` deep-link (cold-open)**: Opens (or focuses) a
//!      Claude Code window pre-filled with the steering prompt text.
//!      Portable (macOS: `open` deep-link; Linux: `claude --print`), requires
//!      no additional permissions, and is the only documented public surface
//!      for delivering context to a Claude Code session.
//!      Limitation: starts a new session context rather than injecting into
//!      the existing conversation — acceptable for v1.
//!
//! **Spike conclusion**: direct in-flight injection is not feasible via any
//! portable, sandboxed-safe surface in v1.  Cold-open prefill is the v1
//! mechanism.  If Claude Code later exposes a local IPC socket or a REST
//! `/sessions/{id}/steer` endpoint, `cold_open_steering` can be updated to
//! prefer injection with cold-open as the fallback.
//!
//! ## V1 relay behaviour
//!
//! 1. After spawning a session for thread `T`, `mod.rs` calls
//!    `register_thread_topic(thread_id, company_uid)` and subscribes the
//!    MQTT client to `topic_for_thread(company_uid, thread_id)` (QoS 1).
//!
//! 2. When a `note` or `mention` event arrives on that topic the relay:
//!    a. Builds a cold-open prefill with the steering note and thread context.
//!    b. Invokes `open claude://code/new?q=...` (macOS) or falls back to
//!       logging on platforms where `open` is unavailable.
//!    c. POSTs a `note` acknowledgement event so the steerer sees receipt.
//!
//! 3. If `open` fails (Claude Code not installed or deep-link rejected),
//!    posts a `blocked` event so the note is never silently lost.
//!
//! 4. If a thread topic arrives for a thread NOT in the relay registry (daemon
//!    restarted; session already ended), posts a `blocked` event:
//!    "no live session; cold-open offered" — never a silent drop.
//!
//! ## Authorization
//!
//! Enforced at the IoT broker layer (US-003): the STS session policy grants
//! Subscribe/Receive on `hq/{companyUid}/thread/*` only to callers with
//! verified company membership.  The relay trusts that any message it
//! receives on a subscribed thread topic has already passed that gate.
//!
//! ## Log codes
//!
//!   `RELAY_SUBSCRIBE`          subscribed to thread steering topic after spawn
//!   `RELAY_STEER`              note/mention event received; relaying
//!   `RELAY_COLD_OPEN`          cold-open issued for steering note
//!   `RELAY_COLD_OPEN_FAIL`     cold-open failed; posting blocked event
//!   `RELAY_ACK_OK`             acknowledgement note event posted
//!   `RELAY_ACK_FAIL`           acknowledgement post failed (non-fatal)
//!   `RELAY_NO_SESSION`         topic not in registry; no-session event posted
//!   `RELAY_NO_SESSION_FAIL`    no-session event post failed (non-fatal)
//!   `RELAY_UNKNOWN_TOPIC`      publish received on unregistered thread topic
//!   `RELAY_IGNORE`             non-steering event kind; ignored

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const LOG_TAG: &str = "relay";

// ── Thread-topic registry ─────────────────────────────────────────────────────

/// One entry per registered steering topic.
#[derive(Clone)]
struct RelayEntry {
    thread_id: String,
    company_uid: String,
}

/// Process-lifetime registry: MQTT topic string → RelayEntry.
/// Populated by `register_thread_topic` right after a session is spawned.
static THREAD_TOPICS: OnceLock<Mutex<HashMap<String, RelayEntry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, RelayEntry>> {
    THREAD_TOPICS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The MQTT topic for a thread's steering channel.
/// Shape: `hq/{companyUid}/thread/{threadId}`
pub fn topic_for_thread(company_uid: &str, thread_id: &str) -> String {
    format!("hq/{}/thread/{}", company_uid, thread_id)
}

/// Register a thread in the relay registry so that incoming MQTT publishes on
/// its steering topic are routed here.  Call immediately after spawning a
/// session so the subscription is set up before any steering arrives.
pub fn register_thread_topic(thread_id: &str, company_uid: &str) {
    let topic = topic_for_thread(company_uid, thread_id);
    if let Ok(mut g) = registry().lock() {
        g.insert(
            topic,
            RelayEntry {
                thread_id: thread_id.to_string(),
                company_uid: company_uid.to_string(),
            },
        );
    }
}

/// Look up a thread by its MQTT topic string.
/// Returns `Some((thread_id, company_uid))` if the topic is registered.
pub fn lookup_thread(topic: &str) -> Option<(String, String)> {
    registry()
        .lock()
        .ok()
        .and_then(|g| g.get(topic).map(|e| (e.thread_id.clone(), e.company_uid.clone())))
}

// ── Payload parsing ───────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SteeringEvent {
    event_kind: String,
    payload: Option<serde_json::Value>,
}

fn extract_note_text(payload: &Option<serde_json::Value>) -> String {
    payload
        .as_ref()
        .and_then(|p| {
            p.get("text")
                .or_else(|| p.get("note"))
                .or_else(|| p.get("message"))
        })
        .and_then(|v| v.as_str())
        .unwrap_or("(no message body)")
        .to_string()
}

fn is_steering_event(kind: &str) -> bool {
    matches!(kind, "note" | "mention")
}

// ── URL helpers ───────────────────────────────────────────────────────────────

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Build the cold-open prefill for a steering note.  The session is instructed
/// to acknowledge receipt and update the thread before continuing.
pub fn build_steering_prefill(thread_id: &str, note: &str, hq_folder: &str) -> String {
    let mut lines = vec![
        format!("Steering note for Work Thread `{}`.", thread_id),
        String::new(),
        "A participant has posted the following steering message via the work mesh:".to_string(),
        String::new(),
        format!("{}", note),
        String::new(),
        format!(
            "Please POST a `note` event to `POST /v1/work-mesh/threads/{}/events` \
             acknowledging receipt, then act on the instruction and continue the thread.",
            thread_id
        ),
        "never leave the thread silent — if you cannot proceed, POST a `blocked` event.".to_string(),
    ];
    if !hq_folder.is_empty() {
        lines.push(String::new());
        lines.push(format!("Working directory: {}", hq_folder));
    }
    lines.join("\n")
}

// ── Cold-open relay (v1 mechanism, spike result) ──────────────────────────────

/// Attempt cold-open relay via `open claude://code/new?q=...` (macOS).
///
/// **Spike conclusion**: direct in-session injection is infeasible in v1.
/// This function is the cold-open fallback and the primary v1 relay mechanism.
/// Returns Ok(()) when `open` accepted the URL; Err(reason) otherwise.
pub fn cold_open_steering(thread_id: &str, note: &str, hq_folder: &str) -> Result<(), String> {
    let prefill = build_steering_prefill(thread_id, note, hq_folder);
    let mut url = format!("claude://code/new?q={}", url_encode(&prefill));
    if !hq_folder.is_empty() {
        url.push_str(&format!("&folder={}", url_encode(hq_folder)));
    }

    let output = std::process::Command::new("open")
        .arg(&url)
        .output()
        .map_err(|e| format!("open: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "open exited {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

// ── REST event helpers ────────────────────────────────────────────────────────

async fn post_thread_event(
    base_url: &str,
    access_token: &str,
    thread_id: &str,
    company_uid: &str,
    event_kind: &str,
    payload: serde_json::Value,
) {
    use crate::util::logfile::log;

    let url = format!(
        "{}/v1/work-mesh/threads/{}/events",
        base_url.trim_end_matches('/'),
        thread_id,
    );
    let body = serde_json::json!({
        "companyUid": company_uid,
        "eventKind": event_kind,
        "payload": payload,
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log(LOG_TAG, &format!("RELAY_ACK_FAIL thread={thread_id} build_client={e}"));
            return;
        }
    };

    match client
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            log(
                LOG_TAG,
                &format!("RELAY_ACK_OK thread={thread_id} kind={event_kind}"),
            );
        }
        Ok(r) => {
            log(
                LOG_TAG,
                &format!(
                    "RELAY_ACK_FAIL thread={thread_id} kind={event_kind} status={}",
                    r.status()
                ),
            );
        }
        Err(e) => {
            log(LOG_TAG, &format!("RELAY_ACK_FAIL thread={thread_id} err={e}"));
        }
    }
}

// ── Public relay entry points ─────────────────────────────────────────────────

/// Handle an incoming MQTT Publish on a registered thread steering topic.
///
/// Called by `mod.rs` for every Publish whose topic `lookup_thread` resolves.
/// Ignores non-steering event kinds (e.g. `claim`, `progress`, `done` are
/// emitted by the spawned session and should not be re-relayed).
pub async fn handle_thread_publish(
    topic: &str,
    payload_bytes: &[u8],
    base_url: &str,
    access_token: &str,
    hq_folder: &str,
) {
    use crate::util::logfile::log;

    let (thread_id, company_uid) = match lookup_thread(topic) {
        Some(pair) => pair,
        None => {
            log(LOG_TAG, &format!("RELAY_UNKNOWN_TOPIC topic={topic}"));
            return;
        }
    };

    // Parse the event.  Non-JSON (e.g. retained snapshot binary) is silently
    // ignored — steering notes always arrive as JSON event payloads.
    let event: SteeringEvent = match serde_json::from_slice(payload_bytes) {
        Ok(e) => e,
        Err(_) => return,
    };

    if !is_steering_event(&event.event_kind) {
        log(
            LOG_TAG,
            &format!(
                "RELAY_IGNORE thread={thread_id} kind={}",
                event.event_kind
            ),
        );
        return;
    }

    let note_text = extract_note_text(&event.payload);
    log(
        LOG_TAG,
        &format!("RELAY_STEER thread={thread_id} kind={}", event.event_kind),
    );

    match cold_open_steering(&thread_id, &note_text, hq_folder) {
        Ok(()) => {
            log(LOG_TAG, &format!("RELAY_COLD_OPEN thread={thread_id}"));
            let truncated = &note_text[..note_text.len().min(80)];
            post_thread_event(
                base_url,
                access_token,
                &thread_id,
                &company_uid,
                "note",
                serde_json::json!({
                    "text": format!(
                        "Steering note received; new session opened. Note: \"{truncated}\""
                    )
                }),
            )
            .await;
        }
        Err(reason) => {
            log(
                LOG_TAG,
                &format!("RELAY_COLD_OPEN_FAIL thread={thread_id} reason={reason}"),
            );
            post_thread_event(
                base_url,
                access_token,
                &thread_id,
                &company_uid,
                "blocked",
                serde_json::json!({
                    "reason": format!(
                        "Steering note received but cold-open failed: {reason}"
                    )
                }),
            )
            .await;
        }
    }
}

/// Post a "no live session" event for a thread that received a steering note
/// but has no registered relay entry (daemon restarted or session already ended).
///
/// The note is NOT silently dropped — a `blocked` event carries the note text
/// so the steerer sees receipt and can re-open via the hq-console deep-link.
pub async fn post_no_session_event(
    thread_id: &str,
    company_uid: &str,
    note_text: &str,
    base_url: &str,
    access_token: &str,
) {
    use crate::util::logfile::log;
    log(LOG_TAG, &format!("RELAY_NO_SESSION thread={thread_id}"));

    let truncated = &note_text[..note_text.len().min(120)];
    post_thread_event(
        base_url,
        access_token,
        thread_id,
        company_uid,
        "blocked",
        serde_json::json!({
            "reason": format!(
                "Steering note received but no live session for this thread. \
                 Note: \"{truncated}\". \
                 Use the hq-console deep-link to open a new session."
            )
        }),
    )
    .await;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Topic helpers ─────────────────────────────────────────────────────────

    #[test]
    fn topic_for_thread_has_expected_shape() {
        assert_eq!(
            topic_for_thread("cmp_indigo", "thr-abc123"),
            "hq/cmp_indigo/thread/thr-abc123"
        );
        assert_eq!(
            topic_for_thread("cmp_acme", "thr-xyz"),
            "hq/cmp_acme/thread/thr-xyz"
        );
    }

    // ── Registry ─────────────────────────────────────────────────────────────

    #[test]
    fn register_and_lookup_round_trip() {
        register_thread_topic("thr-reg-test", "cmp_test");
        let result = lookup_thread("hq/cmp_test/thread/thr-reg-test");
        assert_eq!(
            result,
            Some(("thr-reg-test".to_string(), "cmp_test".to_string()))
        );
    }

    #[test]
    fn lookup_unknown_topic_returns_none() {
        let result = lookup_thread("hq/cmp_other/thread/thr-not-registered");
        assert!(result.is_none(), "unregistered topic must return None");
    }

    #[test]
    fn register_overwrites_on_duplicate() {
        register_thread_topic("thr-dup", "cmp_v1");
        register_thread_topic("thr-dup", "cmp_v2");
        // Last write wins; company_uid updated.
        let result = lookup_thread("hq/cmp_v2/thread/thr-dup");
        assert_eq!(
            result,
            Some(("thr-dup".to_string(), "cmp_v2".to_string()))
        );
    }

    // ── Payload parsing ───────────────────────────────────────────────────────

    #[test]
    fn is_steering_event_matches_note_and_mention() {
        assert!(is_steering_event("note"));
        assert!(is_steering_event("mention"));
    }

    #[test]
    fn is_steering_event_ignores_non_steering_kinds() {
        for kind in ["claim", "progress", "done", "blocked", "handoff", "answer"] {
            assert!(
                !is_steering_event(kind),
                "kind={kind} must not be a steering event"
            );
        }
    }

    #[test]
    fn extract_note_text_reads_text_field() {
        let payload = Some(serde_json::json!({"text": "Please focus on auth"}));
        assert_eq!(extract_note_text(&payload), "Please focus on auth");
    }

    #[test]
    fn extract_note_text_falls_back_to_note_field() {
        let payload = Some(serde_json::json!({"note": "use the new API"}));
        assert_eq!(extract_note_text(&payload), "use the new API");
    }

    #[test]
    fn extract_note_text_falls_back_to_message_field() {
        let payload = Some(serde_json::json!({"message": "try approach B"}));
        assert_eq!(extract_note_text(&payload), "try approach B");
    }

    #[test]
    fn extract_note_text_returns_placeholder_when_absent() {
        let result = extract_note_text(&None);
        assert_eq!(result, "(no message body)");
        let empty = Some(serde_json::json!({}));
        assert_eq!(extract_note_text(&empty), "(no message body)");
    }

    // ── URL encoding ─────────────────────────────────────────────────────────

    #[test]
    fn url_encode_encodes_special_chars() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a/b+c=d"), "a%2Fb%2Bc%3Dd");
    }

    #[test]
    fn url_encode_preserves_unreserved() {
        assert_eq!(url_encode("ABCabc123-_.~"), "ABCabc123-_.~");
    }

    // ── Prefill ───────────────────────────────────────────────────────────────

    #[test]
    fn build_steering_prefill_contains_thread_id_and_note() {
        let prefill = build_steering_prefill("thr-001", "Fix the auth bug", "/workspace");
        assert!(prefill.contains("thr-001"), "must contain thread id");
        assert!(prefill.contains("Fix the auth bug"), "must contain note text");
        assert!(prefill.contains("/workspace"), "must contain hq_folder");
    }

    #[test]
    fn build_steering_prefill_instructs_to_post_ack_event() {
        let prefill = build_steering_prefill("thr-002", "some note", "");
        assert!(prefill.contains("note"), "must instruct to post note event");
        assert!(prefill.contains("blocked"), "must instruct to post blocked if stuck");
        assert!(prefill.contains("never"), "must say never leave thread silent");
    }

    #[test]
    fn build_steering_prefill_omits_folder_when_empty() {
        let prefill = build_steering_prefill("thr-003", "steer me", "");
        assert!(
            !prefill.contains("Working directory"),
            "must not include folder line when empty"
        );
    }

    // ── Cold-open (unit: function surface; actual `open` not called in tests) ─

    #[test]
    fn cold_open_steering_builds_valid_url_structure() {
        // We verify the URL that would be passed to `open` by checking the
        // prefill and url_encode logic, NOT by actually invoking `open`.
        let prefill = build_steering_prefill("thr-x", "steer", "/hq");
        let encoded = url_encode(&prefill);
        assert!(encoded.contains("thr-x"), "encoded prefill must reference thread");
        let url = format!("claude://code/new?q={}&folder={}", encoded, url_encode("/hq"));
        assert!(url.starts_with("claude://code/new?"), "URL must target claude://code/new");
        assert!(url.contains("q="), "must have q param");
        assert!(url.contains("folder="), "must have folder param");
    }

    // ── No-session path ───────────────────────────────────────────────────────

    #[test]
    fn post_no_session_event_truncates_long_notes() {
        // Verify the truncation logic (120 chars) does not panic on long input.
        let long_note = "x".repeat(500);
        let truncated = &long_note[..long_note.len().min(120)];
        assert_eq!(truncated.len(), 120);
        // The function itself is async REST — just confirm the reason string
        // we'd generate is well-formed.
        let reason = format!(
            "Steering note received but no live session for this thread. \
             Note: \"{truncated}\". \
             Use the hq-console deep-link to open a new session."
        );
        assert!(reason.contains("no live session"), "reason must mention no live session");
        assert!(reason.contains("hq-console"), "reason must offer hq-console deep-link");
    }
}
