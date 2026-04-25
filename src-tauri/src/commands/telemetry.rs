//! Telemetry collector — scan ~/.claude/projects/**/*.jsonl on each sync,
//! apply the KEEP/REMOVE allowlist, batch up to 1 MB, POST to /v1/usage.
//!
//! Dispatched from the `AllComplete` arm of `handle_sync_line` via
//! `tauri::async_runtime::spawn`. Does NOT block the sync loop.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::commands::vault_client::{UsageBatch, VaultClient};
use crate::commands::sync::resolve_vault_api_url;

// ── Cursor schema ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CursorEntry {
    offset: u64,
    mtime: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelemetryCursor {
    version: String,
    files: HashMap<String, CursorEntry>,
}

impl Default for TelemetryCursor {
    fn default() -> Self {
        Self {
            version: "1".to_string(),
            files: HashMap::new(),
        }
    }
}

fn cursor_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".hq/telemetry-cursor.json"))
}

fn load_cursor() -> TelemetryCursor {
    cursor_path()
        .and_then(|p| fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cursor(cursor: &TelemetryCursor) -> Result<(), String> {
    use std::io::Write;
    let path = cursor_path().ok_or("home dir unavailable")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(cursor).map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

// ── Sanitizer ─────────────────────────────────────────────────────────────────

fn cwd_hash(cwd: &str) -> String {
    let digest = Sha256::digest(cwd.as_bytes());
    let hex = format!("{:x}", digest);
    hex[..12.min(hex.len())].to_string()
}

/// Build an outgoing event row from the explicit KEEP allowlist.
/// Unknown fields are dropped by default. Fields in the REMOVE list are absent
/// because they are not in the KEEP list.
fn sanitize_row(row: &Value) -> Option<Value> {
    let obj = row.as_object()?;
    let mut out = serde_json::Map::new();

    macro_rules! copy_opt {
        ($key:expr) => {
            if let Some(v) = obj.get($key) {
                out.insert($key.to_string(), v.clone());
            }
        };
    }

    copy_opt!("type");
    copy_opt!("timestamp");
    copy_opt!("sessionId");
    copy_opt!("uuid");
    // parentUuid is explicitly kept (can be null)
    copy_opt!("parentUuid");
    copy_opt!("userType");
    copy_opt!("entrypoint");

    // cwdHash: sha256 of cwd, first 12 hex chars — never expose raw cwd
    if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
        out.insert("cwdHash".to_string(), Value::String(cwd_hash(cwd)));
    }

    copy_opt!("gitBranch");
    copy_opt!("version");
    copy_opt!("requestId");

    // Extract message sub-fields into the top level
    if let Some(msg) = obj.get("message").and_then(|v| v.as_object()) {
        if let Some(v) = msg.get("model") {
            out.insert("model".to_string(), v.clone());
        }
        if let Some(v) = msg.get("role") {
            out.insert("role".to_string(), v.clone());
        }
        if let Some(v) = msg.get("usage") {
            out.insert("usage".to_string(), v.clone());
        }
    }

    Some(Value::Object(out))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_local_telemetry_enabled() -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    let path = home.join(".hq/menubar.json");
    if let Ok(contents) = fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<Value>(&contents) {
            return v.get("telemetryEnabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        }
    }
    false
}

fn read_machine_id() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    let path = home.join(".hq/menubar.json");
    if let Ok(contents) = fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<Value>(&contents) {
            if let Some(id) = v.get("machineId").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    return id.to_string();
                }
            }
        }
    }
    // Bootstrap via ensure_machine_id
    crate::commands::config::ensure_machine_id().unwrap_or_default()
}

fn mtime_secs(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Per-row tracking: which file + byte-end-offset contributed this row.
struct RowSource {
    file_path: String,
    end_offset: u64,
    mtime: u64,
}

const MAX_BATCH_BYTES: usize = 1_000_000;

// ── Main entry point ──────────────────────────────────────────────────────────

/// Scan ~/.claude/projects/**/*.jsonl, sanitize, and POST new events.
///
/// Dispatched from `handle_sync_line`'s AllComplete arm via
/// `tauri::async_runtime::spawn`. Errors are logged and swallowed — telemetry
/// must never abort or delay sync.
pub async fn send_telemetry_if_opted_in<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
    _hq_folder: &str,
    jwt: &str,
) -> Result<(), String> {
    // 1. Build VaultClient
    let api_url = resolve_vault_api_url()?;
    let vault = VaultClient::new(&api_url, jwt);

    // 2. Opt-in check
    let enabled = match vault.get_telemetry_opt_in().await {
        Ok(resp) => resp.enabled,
        Err(_) => {
            eprintln!("[telemetry] telemetry-opt-in-fallback-local");
            read_local_telemetry_enabled()
        }
    };
    if !enabled {
        return Ok(());
    }

    // 3. Load cursor
    let cursor = load_cursor();
    let loaded_files = cursor.files.clone();
    let mut newly_committed: HashMap<String, CursorEntry> = HashMap::new();
    let mut rotation_resets: HashMap<String, CursorEntry> = HashMap::new();

    // 4. Enumerate ~/.claude/projects/**/*.jsonl
    let home = dirs::home_dir().ok_or("home dir unavailable")?;
    let pattern = format!("{}/.claude/projects/**/*.jsonl", home.display());
    let file_paths: Vec<_> = match glob::glob(&pattern) {
        Ok(g) => g.flatten().filter(|p| p.is_file()).collect(),
        Err(_) => return Ok(()),
    };

    let machine_id = read_machine_id();
    let installer_version = env!("CARGO_PKG_VERSION").to_string();

    let mut batch_events: Vec<Value> = Vec::new();
    let mut batch_sources: Vec<RowSource> = Vec::new();

    for file_path in &file_paths {
        let path_str = file_path.to_string_lossy().to_string();

        let metadata = match fs::metadata(file_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let current_size = metadata.len();
        let current_mtime = mtime_secs(&metadata);

        let stored = cursor.files.get(&path_str).cloned().unwrap_or_default();
        let mut offset = stored.offset;

        // File-rotation safety: if file shrank or mtime went backwards
        let rotated = current_size < offset
            || (stored.mtime > 0 && current_mtime < stored.mtime);
        if rotated {
            offset = 0;
            // Mark the reset so we persist it even if there are 0 rows
            rotation_resets.insert(
                path_str.clone(),
                CursorEntry { offset: 0, mtime: current_mtime },
            );
        }

        if offset >= current_size && !rotated {
            // Nothing new to read
            continue;
        }

        // Open and seek
        let mut file = match fs::File::open(file_path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if offset > 0 {
            if file.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }
        }
        let mut content = String::new();
        if file.read_to_string(&mut content).is_err() {
            continue;
        }

        if content.is_empty() {
            continue;
        }

        // Compute line end-offsets within the file
        let segments: Vec<&str> = content.split('\n').collect();
        let n = segments.len();
        let mut cumulative: u64 = 0;
        let line_end_offsets: Vec<u64> = segments.iter().enumerate().map(|(i, seg)| {
            cumulative += seg.len() as u64;
            if i < n - 1 {
                cumulative += 1; // account for the '\n' separator
            }
            offset + cumulative
        }).collect();

        for (i, seg) in segments.iter().enumerate() {
            let trimmed = seg.trim();
            if trimmed.is_empty() {
                continue;
            }
            let parsed: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let sanitized = match sanitize_row(&parsed) {
                Some(v) => v,
                None => continue,
            };

            // Check if adding this row would exceed 1 MB
            if !batch_events.is_empty() {
                let candidate = build_wire_payload(&machine_id, &installer_version, &batch_events, &sanitized);
                if candidate.len() > MAX_BATCH_BYTES {
                    // Flush current batch
                    flush_batch(
                        &vault,
                        &machine_id,
                        &installer_version,
                        &mut batch_events,
                        &mut batch_sources,
                        &mut newly_committed,
                    ).await;
                }
            }

            batch_events.push(sanitized);
            batch_sources.push(RowSource {
                file_path: path_str.clone(),
                end_offset: line_end_offsets[i],
                mtime: current_mtime,
            });
        }
    }

    // Flush remaining batch
    if !batch_events.is_empty() {
        flush_batch(
            &vault,
            &machine_id,
            &installer_version,
            &mut batch_events,
            &mut batch_sources,
            &mut newly_committed,
        ).await;
    }

    // Build final cursor: loaded < rotation_resets < newly_committed
    let mut final_files = loaded_files;
    for (fp, entry) in rotation_resets {
        final_files.insert(fp, entry);
    }
    for (fp, entry) in newly_committed {
        final_files.insert(fp, entry);
    }

    // 7. Atomic cursor write
    let final_cursor = TelemetryCursor {
        version: "1".to_string(),
        files: final_files,
    };
    save_cursor(&final_cursor)?;

    Ok(())
}

/// Build the full wire payload JSON for size-checking.
fn build_wire_payload(
    machine_id: &str,
    installer_version: &str,
    existing: &[Value],
    candidate: &Value,
) -> Vec<u8> {
    let mut events = existing.to_vec();
    events.push(candidate.clone());
    let payload = serde_json::json!({
        "machineId": machine_id,
        "installerVersion": installer_version,
        "events": events,
    });
    serde_json::to_vec(&payload).unwrap_or_default()
}

async fn flush_batch(
    vault: &VaultClient,
    machine_id: &str,
    installer_version: &str,
    batch_events: &mut Vec<Value>,
    batch_sources: &mut Vec<RowSource>,
    newly_committed: &mut HashMap<String, CursorEntry>,
) {
    let batch = UsageBatch {
        machine_id: machine_id.to_string(),
        installer_version: installer_version.to_string(),
        events: std::mem::take(batch_events),
    };
    let sources = std::mem::take(batch_sources);

    if vault.post_usage(&batch).await.is_ok() {
        // Advance cursor to max end_offset per file in this batch
        let mut max_per_file: HashMap<String, (u64, u64)> = HashMap::new();
        for src in &sources {
            max_per_file
                .entry(src.file_path.clone())
                .and_modify(|(_, off)| *off = (*off).max(src.end_offset))
                .or_insert((src.mtime, src.end_offset));
        }
        for (fp, (mtime, offset)) in max_per_file {
            newly_committed.insert(fp, CursorEntry { offset, mtime });
        }
    }
    // On non-200: batch_events and batch_sources are already cleared (mem::take),
    // and we do NOT advance newly_committed for this batch's files.
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::ENV_MUTEX;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Test helpers ─────────────────────────────────────────────────────────

    /// Create a temp HOME with ~/.hq/ and ~/.claude/projects/ structure.
    fn setup_home() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".hq")).unwrap();
        fs::create_dir_all(tmp.path().join(".claude/projects")).unwrap();
        tmp
    }

    /// Write a JSONL file under ~/.claude/projects/<subdir>/<name>.jsonl.
    fn write_jsonl(home: &std::path::Path, subdir: &str, name: &str, lines: &[&str]) -> std::path::PathBuf {
        let dir = home.join(".claude/projects").join(subdir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let content: String = lines.iter().map(|l| format!("{}\n", l)).collect();
        fs::write(&p, &content).unwrap();
        p
    }

    fn write_menubar(home: &std::path::Path, content: &str) {
        fs::write(home.join(".hq/menubar.json"), content).unwrap();
    }

    fn read_cursor(home: &std::path::Path) -> TelemetryCursor {
        let body = fs::read_to_string(home.join(".hq/telemetry-cursor.json")).unwrap();
        serde_json::from_str(&body).unwrap()
    }

    const USER_ROW: &str = r#"{"type":"user","timestamp":"2026-04-25T10:00:00Z","sessionId":"s1","uuid":"u1","parentUuid":null,"userType":"human","entrypoint":"cli","cwd":"/Users/x/proj","gitBranch":"main","version":"1.0","message":{"role":"user","content":[{"type":"text","text":"hello world"}],"id":"msg_1"}}"#;
    const ASST_ROW: &str = r#"{"type":"assistant","timestamp":"2026-04-25T10:00:01Z","sessionId":"s1","uuid":"u2","parentUuid":"u1","message":{"role":"assistant","model":"claude-opus","content":[{"type":"text","text":"hi"},{"type":"thinking","thinking":"hmm"}],"stop_sequence":"</end>","usage":{"input_tokens":42,"output_tokens":7},"id":"msg_2"},"toolUseIds":["t1"],"toolResults":[{"id":"t1","output":"x"}],"requestId":"req_1"}"#;

    fn make_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.handle().clone()
    }

    // ── (a) opt-in=false → 0 bytes sent ──────────────────────────────────────

    #[tokio::test]
    async fn test_opt_in_false_sends_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": false})))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"test-id","hqPath":"/foo"}"#);
        write_jsonl(home.path(), "proj", "session.jsonl", &[USER_ROW, ASST_ROW]);
        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "test-jwt").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());
        let reqs = server.received_requests().await.unwrap();
        let posts: Vec<_> = reqs.iter().filter(|r| r.method == wiremock::http::Method::POST).collect();
        assert_eq!(posts.len(), 0, "no POST expected when opt-in is false");
    }

    // ── (b) Missing cursor file → all files at offset 0 ──────────────────────

    #[tokio::test]
    async fn test_missing_cursor_starts_at_offset_zero() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": true})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"mid-b","hqPath":"/foo"}"#);
        let jsonl_path = write_jsonl(home.path(), "proj", "s.jsonl", &[USER_ROW, ASST_ROW]);
        let file_size = fs::metadata(&jsonl_path).unwrap().len();

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "test-jwt").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());

        // Cursor file should exist with correct offset
        let cursor = read_cursor(home.path());
        let path_str = jsonl_path.to_string_lossy().to_string();
        let entry = cursor.files.get(&path_str).expect("cursor should have entry for the file");
        assert_eq!(entry.offset, file_size, "cursor offset should equal file size");

        // POST should have been made with 2 events
        let reqs = server.received_requests().await.unwrap();
        let posts: Vec<_> = reqs.iter().filter(|r| r.method == wiremock::http::Method::POST).collect();
        assert!(!posts.is_empty(), "at least 1 POST expected");
        let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
        let events = body["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
    }

    // ── (c) Strip-list removes every REMOVE field ─────────────────────────────

    #[tokio::test]
    async fn test_strip_list_removes_remove_fields() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": true})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"mid-c"}"#);
        // Row containing ALL REMOVE fields
        let full_row = r#"{"type":"user","timestamp":"2026-04-25T10:00:00Z","sessionId":"s1","uuid":"u1","parentUuid":null,"userType":"human","entrypoint":"cli","cwd":"/Users/x","gitBranch":"main","version":"1.0","content":[{"type":"text"}],"thinking":"internal","text":"raw","toolUseIds":["t1"],"toolResults":[{"id":"t1"}],"message":{"role":"user","content":[{"type":"text","text":"hi"}],"model":"claude","thinking":"x","text":"y","stop_sequence":"\n\nHuman:","id":"msg_1","usage":{"input_tokens":5,"output_tokens":2}}}"#;
        write_jsonl(home.path(), "proj", "full.jsonl", &[full_row]);

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "tok").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());

        let reqs = server.received_requests().await.unwrap();
        let posts: Vec<_> = reqs.iter().filter(|r| r.method == wiremock::http::Method::POST).collect();
        assert!(!posts.is_empty());
        let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
        for event in body["events"].as_array().unwrap() {
            let obj = event.as_object().unwrap();
            for removed in &["content", "thinking", "text", "toolUseIds", "toolResults"] {
                assert!(!obj.contains_key(*removed), "top-level `{}` must be absent", removed);
            }
            if let Some(msg) = obj.get("message") {
                let msg_obj = msg.as_object().unwrap();
                for removed in &["content", "thinking", "text", "stop_sequence", "id"] {
                    assert!(!msg_obj.contains_key(*removed), "message.`{}` must be absent", removed);
                }
            }
        }
    }

    // ── (d) 1 MB cap rollover ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_one_mb_cap_causes_rollover() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": true})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"mid-d"}"#);

        // Generate ~50 rows with a large gitBranch so sanitized rows are ~25 KB each
        // 50 * 25 KB ≈ 1.25 MB > 1 MB → should produce ≥2 batches
        let long_branch = "x".repeat(25_000);
        let mut lines = Vec::new();
        for i in 0..50usize {
            let row = json!({
                "type": "user",
                "timestamp": format!("2026-04-25T10:00:{:02}Z", i % 60),
                "sessionId": "s1",
                "uuid": format!("u{}", i),
                "parentUuid": null,
                "userType": "human",
                "entrypoint": "cli",
                "cwd": "/Users/x",
                "gitBranch": long_branch,
                "version": "1.0",
                "message": {"role": "user", "content": [{"type": "text", "text": "hi"}], "id": "m"}
            });
            lines.push(serde_json::to_string(&row).unwrap());
        }
        let lines_str: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        write_jsonl(home.path(), "proj", "large.jsonl", &lines_str);

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "tok").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());

        let reqs = server.received_requests().await.unwrap();
        let posts: Vec<_> = reqs.iter().filter(|r| r.method == wiremock::http::Method::POST).collect();
        assert!(posts.len() >= 2, "expected ≥2 POSTs due to 1 MB rollover, got {}", posts.len());

        // Last batch must be < 1 MB
        let last_post = posts.last().unwrap();
        assert!(last_post.body.len() < MAX_BATCH_BYTES,
            "last batch must be < 1 MB, got {} bytes", last_post.body.len());
    }

    // ── (e) Non-200 does NOT advance cursor ───────────────────────────────────

    #[tokio::test]
    async fn test_non_200_does_not_advance_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": true})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(500).set_body_string("error"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"mid-e"}"#);
        let jsonl_path = write_jsonl(home.path(), "proj", "s.jsonl", &[USER_ROW, ASST_ROW, USER_ROW]);

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "tok").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());

        let path_str = jsonl_path.to_string_lossy().to_string();
        // Cursor entry must be absent (or at 0), NOT at EOF
        let cursor_file = home.path().join(".hq/telemetry-cursor.json");
        if cursor_file.exists() {
            let cursor = read_cursor(home.path());
            if let Some(entry) = cursor.files.get(&path_str) {
                assert_eq!(entry.offset, 0, "cursor must not advance on 500");
            }
            // If absent, that's also acceptable
        }
        // Verify that no entry with non-zero offset exists
        if cursor_file.exists() {
            let cursor = read_cursor(home.path());
            let entry_offset = cursor.files.get(&path_str).map(|e| e.offset).unwrap_or(0);
            assert_eq!(entry_offset, 0, "cursor offset must be 0 (or absent) after failed POST");
        }
    }

    // ── (f) Atomic cursor write ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_atomic_cursor_write_no_tmp_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": true})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"mid-f"}"#);
        write_jsonl(home.path(), "proj", "s.jsonl", &[USER_ROW]);

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "tok").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());
        assert!(!home.path().join(".hq/telemetry-cursor.json.tmp").exists(),
            "no .tmp file should remain after atomic write");
        assert!(home.path().join(".hq/telemetry-cursor.json").exists(),
            "cursor file must exist after successful run");
    }

    // ── (g) New files discovered between runs start at offset 0 ──────────────

    #[tokio::test]
    async fn test_new_file_between_runs_starts_at_zero() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": true})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"mid-g"}"#);

        // Run 1: only fixture A
        let _path_a = write_jsonl(home.path(), "proj-a", "a.jsonl", &[USER_ROW]);
        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        send_telemetry_if_opted_in(&handle, "/hq", "tok").await.unwrap();

        let posts_run1 = server.received_requests().await.unwrap()
            .iter().filter(|r| r.method == wiremock::http::Method::POST).count();
        assert!(posts_run1 >= 1, "run 1 should POST fixture A");

        // Run 2: add fixture B
        let path_b = write_jsonl(home.path(), "proj-b", "b.jsonl", &[ASST_ROW]);
        send_telemetry_if_opted_in(&handle, "/hq", "tok").await.unwrap();

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        let cursor = read_cursor(home.path());
        let path_b_str = path_b.to_string_lossy().to_string();
        let b_size = fs::metadata(&path_b).unwrap().len();
        let b_entry = cursor.files.get(&path_b_str)
            .expect("cursor should have an entry for fixture B after run 2");
        assert_eq!(b_entry.offset, b_size, "fixture B should be fully consumed in run 2");
    }

    // ── (h) Truncated/rotated file resets cursor ──────────────────────────────

    #[tokio::test]
    async fn test_rotated_file_resets_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({"enabled": true})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        write_menubar(home.path(), r#"{"machineId":"mid-h"}"#);

        // Run 1: fixture A with 3 rows
        let path_a = write_jsonl(home.path(), "proj", "a.jsonl", &[USER_ROW, ASST_ROW, USER_ROW]);
        let original_size = fs::metadata(&path_a).unwrap().len();
        assert!(original_size > 0);

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        send_telemetry_if_opted_in(&handle, "/hq", "tok").await.unwrap();

        // Verify run 1 set cursor to EOF
        let cursor_after_run1 = read_cursor(home.path());
        let path_a_str = path_a.to_string_lossy().to_string();
        let entry1 = cursor_after_run1.files.get(&path_a_str).unwrap();
        assert_eq!(entry1.offset, original_size);

        // Truncate A to 0 bytes (size < stored_offset → rotation trigger)
        {
            let _f = fs::OpenOptions::new().write(true).truncate(true).open(&path_a).unwrap();
        }
        assert_eq!(fs::metadata(&path_a).unwrap().len(), 0);

        // Run 2: A is now empty after truncation
        send_telemetry_if_opted_in(&handle, "/hq", "tok").await.unwrap();

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        // Cursor for A should be reset to 0
        let cursor_after_run2 = read_cursor(home.path());
        let entry2_offset = cursor_after_run2.files.get(&path_a_str).map(|e| e.offset).unwrap_or(0);
        assert_eq!(entry2_offset, 0, "cursor must be reset to 0 after file rotation/truncation");
    }

    // ── (i) GET opt-in HTTP 500 → fallback reads menubar.json ─────────────────

    #[tokio::test]
    async fn test_opt_in_500_fallback_true_runs_telemetry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(500).set_body_string("error"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        // menubar.json has telemetryEnabled: true
        write_menubar(home.path(), r#"{"machineId":"mid-i1","telemetryEnabled":true}"#);
        write_jsonl(home.path(), "proj", "s.jsonl", &[USER_ROW]);

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "tok").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());
        let reqs = server.received_requests().await.unwrap();
        let posts: Vec<_> = reqs.iter().filter(|r| r.method == wiremock::http::Method::POST).collect();
        assert!(!posts.is_empty(), "telemetryEnabled=true in fallback → should POST ≥1");
    }

    #[tokio::test]
    async fn test_opt_in_500_fallback_false_skips_telemetry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage/opt-in"))
            .respond_with(ResponseTemplate::new(500).set_body_string("error"))
            .mount(&server)
            .await;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let home = setup_home();
        // menubar.json has telemetryEnabled: false
        write_menubar(home.path(), r#"{"machineId":"mid-i2","telemetryEnabled":false}"#);
        write_jsonl(home.path(), "proj", "s.jsonl", &[USER_ROW]);

        std::env::set_var("HOME", home.path());
        std::env::set_var("HQ_VAULT_API_URL", server.uri());

        let handle = make_app_handle();
        let result = send_telemetry_if_opted_in(&handle, "/hq", "tok").await;

        std::env::remove_var("HOME");
        std::env::remove_var("HQ_VAULT_API_URL");

        assert!(result.is_ok());
        let reqs = server.received_requests().await.unwrap();
        let posts: Vec<_> = reqs.iter().filter(|r| r.method == wiremock::http::Method::POST).collect();
        assert_eq!(posts.len(), 0, "telemetryEnabled=false in fallback → no POST");
    }

    // ── test_telemetry_strips_prompt_bodies (fixture-based) ───────────────────

    #[test]
    fn test_telemetry_strips_prompt_bodies() {
        let fixtures_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude-projects");

        let mut checked = 0usize;
        for entry in walkdir::WalkDir::new(&fixtures_dir)
            .into_iter()
            .flatten()
            .filter(|e| e.path().extension().map_or(false, |x| x == "jsonl"))
        {
            let content = fs::read_to_string(entry.path()).expect("read fixture");
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let parsed: Value = serde_json::from_str(trimmed).expect("parse fixture line");
                let sanitized = sanitize_row(&parsed)
                    .expect("sanitize_row must return Some for valid rows");
                let obj = sanitized.as_object().unwrap();

                // No REMOVE field at top level
                for removed in &["content", "thinking", "text", "toolUseIds", "toolResults"] {
                    assert!(
                        !obj.contains_key(*removed),
                        "fixture {:?}: top-level `{}` must not survive sanitization",
                        entry.path(),
                        removed,
                    );
                }
                // No REMOVE field inside `message` (there should be no `message` key after sanitization)
                assert!(!obj.contains_key("message"),
                    "fixture {:?}: `message` must be flattened — no sub-object should remain",
                    entry.path());

                checked += 1;
            }
        }
        assert!(checked > 0, "must have processed at least one fixture row");
    }
}
