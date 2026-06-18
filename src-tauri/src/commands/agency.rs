//! Local reader/writer for the **Agency** surface in Mission Control.
//!
//! The desktop-alt window lists every hq-pack-agency TEAM the user is running —
//! its workers and their status — and surfaces the team-manager's pending
//! QUESTIONS so the operator can answer them directly in the app instead of
//! through the liaison worker's AskUserQuestion. All three commands read/write
//! the agency chat files under the resolved HQ folder
//! (`workspace/agency/<company>/<team>/<worker>/<instance>/chat.jsonl` + the
//! team's `status.json`); no vault round-trip.
//!
//! Answering writes an `ANSWER: <text> [ans:<id>]` line to the team-manager
//! inbox, where `<id>` is the POSIX `cksum` of the question text — the SAME id
//! the liaison's `liaison.sh answer` computes — so the liaison worker and
//! Mission Control dedup against each other and never double-answer (the
//! "mirror alongside the liaison" model). The cksum impl is unit-tested against
//! the real `cksum(1)` output so the two id schemes can never silently drift.
//!
//! Like the other desktop-alt readers (`commands/library_local.rs`), every
//! command is gated by `feature_gate::desktop_features_enabled()`, resolves the
//! HQ folder with the standard 4-tier resolver, and guards writes with the
//! lexical `is_within` path-traversal check. Parsing is lenient: a malformed
//! line or a missing file is skipped, never a panic.
//!
//! App-registered Tauri commands authorized by `core:default` (custom commands
//! are not gated by per-command permission identifiers), so no allow-* tokens
//! are added to `capabilities/desktop-alt.json`.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::paths;

// ---- wire types (camelCase) ------------------------------------------------

/// One worker (a `(worker, instance)`) in a team.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgencyWorker {
    pub worker: String,
    pub instance: String,
    /// From the team's `status.json` (`running` | `stopped` | …); `unknown` when absent.
    pub status: String,
    /// True once the worker has posted its `type:"ready"` handshake to its inbox.
    pub ready: bool,
    /// ISO `started_at` from `status.json` — when the worker was last spawned;
    /// empty when absent. Drives the "up 12m" uptime label.
    pub started_at: String,
    /// ISO `updated_at` from `status.json` — last status write for this worker;
    /// empty when absent. Drives the "seen 30s ago" freshness label.
    pub updated_at: String,
}

/// One running agency team.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgencyTeam {
    pub company: String,
    pub team: String,
    pub workers: Vec<AgencyWorker>,
}

/// A pending question the team-manager routed to the liaison and that has not
/// yet been answered into the manager inbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgencyQuestion {
    pub company: String,
    pub team: String,
    /// Stable dedup id — the POSIX cksum of the question text (matches the liaison).
    pub id: String,
    pub question: String,
    pub ts: String,
    /// Bounded answer choices the manager attached to the ASK (`"options"` on the
    /// chat line); empty for a free-text question. Rendered as one-tap answer chips.
    pub options: Vec<String>,
}

/// One line of the Manager ⇄ Liaison conversation, normalised for display so the
/// operator can read the full context behind a pending question.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgencyMessage {
    /// Sender — `manager` | `liaison` | `operator` | a worker name.
    pub from: String,
    /// Classified kind — `ask` | `fyi` | `answer` | `learn` | `ready` | `reply` | `close` | `msg`.
    pub kind: String,
    /// Display text with the known prefix and any trailing `[ans:<id>]` tag stripped.
    pub text: String,
    pub ts: String,
    /// Which inbox the line lives in — `team-manager` | `team-liaison`.
    pub inbox: String,
}

// ---- POSIX cksum (the [ans:<id>] dedup key the liaison uses) ----------------

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

/// POSIX `cksum` CRC-32 of `bytes` — byte-for-byte identical to `cksum(1)`.
fn cksum(bytes: &[u8]) -> u32 {
    let t = crc_table();
    let mut crc: u32 = 0;
    for &b in bytes {
        crc = (crc << 8) ^ t[(((crc >> 24) as u8) ^ b) as usize];
    }
    let mut len = bytes.len();
    while len != 0 {
        let b = (len & 0xFF) as u8;
        crc = (crc << 8) ^ t[(((crc >> 24) as u8) ^ b) as usize];
        len >>= 8;
    }
    !crc
}

// ---- helpers ---------------------------------------------------------------

/// Bounded answer choices the manager attached to an ASK line (`"options"` — an
/// array of strings). Empty when the field is absent, not an array, or carries
/// no non-blank string entries — i.e. a free-text question. Mirrors how the
/// producer (`agency.sh ask --option …`) writes them, and is forward-compatible:
/// older ASK lines with no `options` key simply yield `[]`.
fn parse_options(line: &serde_json::Value) -> Vec<String> {
    line.get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Normalise one chat.jsonl line from `inbox_owner`'s inbox into a display
/// message: resolve the sender, classify the kind, and strip the `ASK:`/`FYI:`/
/// `ANSWER:`/`LEARN:` prefix + the trailing `[ans:<id>]` dedup tag so the chat
/// reads cleanly. Pure + lenient — unit-tested.
fn classify_message(inbox_owner: &str, line: &serde_json::Value) -> AgencyMessage {
    let role = line.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let typ = line.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let from_field = line.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let raw = line
        .get("text")
        .and_then(|v| v.as_str())
        .or_else(|| line.get("reason").and_then(|v| v.as_str()))
        .unwrap_or("");

    let owner_short = match inbox_owner {
        "team-manager" => "manager",
        "team-liaison" => "liaison",
        other => other,
    };
    // Assistant lines are authored by the inbox owner; `user` lines carry a
    // `from` tag (an empty `from` on a user line is an operator-posted message).
    let from = match role {
        "assistant" => owner_short.to_string(),
        "manager" => "manager".to_string(),
        "user" if from_field.is_empty() => "operator".to_string(),
        _ => from_field.to_string(),
    };

    let (kind, text) = if !typ.is_empty() {
        (typ.to_string(), raw.to_string())
    } else if let Some(r) = raw.strip_prefix("ASK: ") {
        ("ask".to_string(), r.to_string())
    } else if let Some(r) = raw.strip_prefix("FYI: ") {
        ("fyi".to_string(), r.to_string())
    } else if let Some(r) = raw.strip_prefix("ANSWER: ") {
        let clean = r.rfind(" [ans:").map(|i| &r[..i]).unwrap_or(r);
        ("answer".to_string(), clean.to_string())
    } else if let Some(r) = raw.strip_prefix("LEARN: ") {
        ("learn".to_string(), r.to_string())
    } else {
        ("msg".to_string(), raw.to_string())
    };

    AgencyMessage {
        from,
        kind,
        text,
        ts: line.get("ts").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        inbox: inbox_owner.to_string(),
    }
}

fn resolve_hq_folder() -> PathBuf {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    )
}

fn agency_root(hq: &Path) -> PathBuf {
    hq.join("workspace").join("agency")
}

/// Immediate subdirectory names of `dir`, sorted. Empty when `dir` is missing.
fn child_dirs(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

/// Parse a JSONL file into `serde_json::Value` rows; malformed lines skipped.
fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect()
}

/// `true` if the inbox carries a `type:"ready"` handshake line.
fn inbox_ready(inbox: &Path) -> bool {
    read_jsonl(inbox)
        .iter()
        .any(|o| o.get("type").and_then(|v| v.as_str()) == Some("ready"))
}

/// Read the team's `status.json` → the `workers` map (lenient; `{}` on any error).
fn read_status_map(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("workers").cloned())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn now_iso() -> String {
    // Matches chat.sh's `date -u +%FT%TZ` (e.g. 2026-06-18T20:30:00Z).
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn is_within(root: &Path, candidate: &Path) -> bool {
    lexically_normalize(candidate).starts_with(lexically_normalize(root))
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                _ => stack.push(component),
            },
            other => stack.push(other),
        }
    }
    let mut out = PathBuf::new();
    for c in stack {
        out.push(c.as_os_str());
    }
    out
}

// ---- commands --------------------------------------------------------------

/// List every agency team under `workspace/agency/<company>/<team>/`, each with
/// its `(worker, instance)` rows + status + ready flag. Empty (not an error)
/// when nothing is provisioned.
#[tauri::command]
pub async fn list_agency_teams() -> Result<Vec<AgencyTeam>, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("agency reader requires a signed-in user".to_string());
    }
    let root = agency_root(&resolve_hq_folder());
    let mut teams = Vec::new();
    for company in child_dirs(&root) {
        let cdir = root.join(&company);
        for team in child_dirs(&cdir) {
            let tdir = cdir.join(&team);
            let status = read_status_map(&tdir.join("status.json"));
            let mut workers = Vec::new();
            for worker in child_dirs(&tdir) {
                let wdir = tdir.join(&worker);
                for instance in child_dirs(&wdir) {
                    let inbox = wdir.join(&instance).join("chat.jsonl");
                    let wstat = status.get(&worker).and_then(|w| w.get(&instance));
                    let field = |key: &str, default: &str| {
                        wstat
                            .and_then(|i| i.get(key))
                            .and_then(|v| v.as_str())
                            .unwrap_or(default)
                            .to_string()
                    };
                    workers.push(AgencyWorker {
                        worker: worker.clone(),
                        ready: inbox_ready(&inbox),
                        status: field("status", "unknown"),
                        started_at: field("started_at", ""),
                        updated_at: field("updated_at", ""),
                        instance,
                    });
                }
            }
            teams.push(AgencyTeam { company: company.clone(), team, workers });
        }
    }
    Ok(teams)
}

/// List the team-manager's PENDING questions across all teams — `ASK:` lines in
/// each team-liaison inbox whose `[ans:<cksum>]` has not yet been written into
/// the matching team-manager inbox.
#[tauri::command]
pub async fn list_agency_questions() -> Result<Vec<AgencyQuestion>, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("agency reader requires a signed-in user".to_string());
    }
    let root = agency_root(&resolve_hq_folder());
    let mut out = Vec::new();
    for company in child_dirs(&root) {
        let cdir = root.join(&company);
        for team in child_dirs(&cdir) {
            let tdir = cdir.join(&team);
            let liaison = tdir.join("team-liaison").join("main").join("chat.jsonl");
            let manager_text =
                std::fs::read_to_string(tdir.join("team-manager").join("main").join("chat.jsonl"))
                    .unwrap_or_default();
            for o in read_jsonl(&liaison) {
                let role = o.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let from = o.get("from").and_then(|v| v.as_str()).unwrap_or("");
                let text = o.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if role == "user" && from == "manager" {
                    if let Some(q) = text.strip_prefix("ASK: ") {
                        let id = cksum(q.as_bytes()).to_string();
                        if !manager_text.contains(&format!("[ans:{id}]")) {
                            out.push(AgencyQuestion {
                                company: company.clone(),
                                team: team.clone(),
                                id,
                                question: q.to_string(),
                                ts: o.get("ts").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                options: parse_options(&o),
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Answer a pending question — append `ANSWER: <answer> [ans:<id>]` to the
/// team-manager inbox (the same line `liaison.sh answer` writes), idempotently.
/// `id` is the dedup id from `list_agency_questions`. Path-guarded so a crafted
/// company/team can't escape the agency root.
#[tauri::command]
pub async fn answer_agency_question(
    company: String,
    team: String,
    id: String,
    answer: String,
) -> Result<String, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("answering requires a signed-in user".to_string());
    }
    let root = agency_root(&resolve_hq_folder());
    let manager = root
        .join(&company)
        .join(&team)
        .join("team-manager")
        .join("main")
        .join("chat.jsonl");
    if !is_within(&root, &manager) {
        return Err("invalid team path".to_string());
    }
    let existing = std::fs::read_to_string(&manager).unwrap_or_default();
    if existing.contains(&format!("[ans:{id}]")) {
        return Ok("already-answered".to_string());
    }
    if let Some(parent) = manager.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Append-only, matching chat.sh: one compact JSON line to the manager inbox.
    let line = serde_json::json!({
        "role": "user",
        "from": "liaison",
        "text": format!("ANSWER: {answer} [ans:{id}]"),
        "ts": now_iso(),
    });
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&manager)
        .map_err(|e| e.to_string())?;
    writeln!(f, "{}", serde_json::to_string(&line).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok("delivered".to_string())
}

/// The Manager ⇄ Liaison conversation for one team — both inboxes merged and
/// sorted chronologically — so the operator can read the full context behind a
/// pending question. Path-guarded; empty (not an error) when nothing exists.
#[tauri::command]
pub async fn list_agency_chat(company: String, team: String) -> Result<Vec<AgencyMessage>, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("agency chat requires a signed-in user".to_string());
    }
    let root = agency_root(&resolve_hq_folder());
    let tdir = root.join(&company).join(&team);
    if !is_within(&root, &tdir) {
        return Err("invalid team path".to_string());
    }
    let mut msgs = Vec::new();
    for owner in ["team-manager", "team-liaison"] {
        let inbox = tdir.join(owner).join("main").join("chat.jsonl");
        for line in read_jsonl(&inbox) {
            let m = classify_message(owner, &line);
            if !m.text.trim().is_empty() {
                msgs.push(m);
            }
        }
    }
    // Chronological by ISO ts (lexical sort is correct for the `…Z` format);
    // stable so same-timestamp lines keep their per-inbox order.
    msgs.sort_by(|a, b| a.ts.cmp(&b.ts));
    Ok(msgs)
}

/// Post an operator message straight into the team-manager inbox — the same
/// chat.jsonl transport the manager's `listen` loop consumes — so the operator
/// can talk to the team directly. Path-guarded; rejects an empty body.
#[tauri::command]
pub async fn send_agency_message(
    company: String,
    team: String,
    text: String,
) -> Result<String, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("sending requires a signed-in user".to_string());
    }
    let body = text.trim();
    if body.is_empty() {
        return Err("empty message".to_string());
    }
    let root = agency_root(&resolve_hq_folder());
    let manager = root
        .join(&company)
        .join(&team)
        .join("team-manager")
        .join("main")
        .join("chat.jsonl");
    if !is_within(&root, &manager) {
        return Err("invalid team path".to_string());
    }
    if let Some(parent) = manager.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Append-only, matching chat.sh `user`/`say`: one compact JSON line.
    let line = serde_json::json!({
        "role": "user",
        "from": "operator",
        "text": body,
        "ts": now_iso(),
    });
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&manager)
        .map_err(|e| e.to_string())?;
    writeln!(f, "{}", serde_json::to_string(&line).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok("sent".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pinned against real `cksum(1)` output (printf '%s' "<s>" | cksum). If the
    // CRC drifts from POSIX cksum the liaison and Mission Control would compute
    // different [ans:<id>]s and double-answer — so this is a hard contract.
    #[test]
    fn cksum_matches_posix() {
        assert_eq!(cksum(b"Ship it?"), 780_494_884);
        assert_eq!(cksum(b"test"), 3_076_352_578);
        assert_eq!(cksum(b"Approve deploy to prod?"), 1_221_633_456);
        assert_eq!(cksum(b"Deploy nick to prod & resolve #1234?"), 2_811_623_944);
    }

    #[test]
    fn parse_options_reads_string_array() {
        // Trims, drops blanks + non-strings, preserves order.
        let line = serde_json::json!({
            "text": "ASK: Deploy?",
            "options": ["Ship it", "  Hold  ", "", "   ", 42, true]
        });
        assert_eq!(parse_options(&line), vec!["Ship it".to_string(), "Hold".to_string()]);
    }

    #[test]
    fn parse_options_absent_or_malformed_is_empty() {
        // No key, wrong type, and all-blank all collapse to a free-text question.
        assert!(parse_options(&serde_json::json!({"text": "ASK: x"})).is_empty());
        assert!(parse_options(&serde_json::json!({"options": "yes/no"})).is_empty());
        assert!(parse_options(&serde_json::json!({"options": ["", "  "]})).is_empty());
    }

    #[test]
    fn classify_message_resolves_sender_kind_and_clean_text() {
        // manager's ASK lives in the liaison inbox; prefix stripped, kind=ask.
        let ask = classify_message(
            "team-liaison",
            &serde_json::json!({"role":"user","from":"manager","text":"ASK: Deploy?","ts":"t1"}),
        );
        assert_eq!((ask.from.as_str(), ask.kind.as_str(), ask.text.as_str()), ("manager", "ask", "Deploy?"));

        // liaison's ANSWER lives in the manager inbox; prefix + [ans:] tag stripped.
        let ans = classify_message(
            "team-manager",
            &serde_json::json!({"role":"user","from":"liaison","text":"ANSWER: Ship it [ans:123]","ts":"t2"}),
        );
        assert_eq!((ans.from.as_str(), ans.kind.as_str(), ans.text.as_str()), ("liaison", "answer", "Ship it"));

        // user line with no `from` is an operator-posted message.
        let op = classify_message(
            "team-manager",
            &serde_json::json!({"role":"user","text":"hold off until QA signs off","ts":"t3"}),
        );
        assert_eq!((op.from.as_str(), op.kind.as_str()), ("operator", "msg"));

        // assistant `ready` line is authored by the inbox owner.
        let rdy = classify_message(
            "team-manager",
            &serde_json::json!({"role":"assistant","type":"ready","text":"online","ts":"t0"}),
        );
        assert_eq!((rdy.from.as_str(), rdy.kind.as_str()), ("manager", "ready"));
    }
}
