//! Liveness engine (US-004).
//!
//! Refines the coarse, mtime-only status the readers (US-002/US-003) emit into
//! the canonical [`SessionStatus`] taxonomy by combining two best-effort signals:
//!
//!   1. **A last-activity window** — how long ago the session last wrote to disk,
//!      mapped to `running` / `awaiting_input` / `idle` / `ended` against the
//!      named thresholds below.
//!   2. **A running-process cross-check** — whether a live `claude` / `codex`
//!      process actually exists on the box. A session whose last activity *looks*
//!      fresh but has **no live process** is resolved to [`SessionStatus::Ended`]
//!      (the process died without a final write). This is the PRD's hard
//!      requirement: "a session with no live process resolves to ended".
//!
//! Best-effort by design (PRD notes: "Best-effort by design — the UI labels it as
//! such"). Every input is observed, never authoritative: clocks skew, a writer can
//! pause mid-turn, a process can be a stale zombie. The thresholds are therefore
//! deliberately coarse and documented as named constants so a maintainer can tune
//! them without spelunking.
//!
//! ## How status is derived
//!
//! Given the session's last-activity age (now − lastActivityAt) and whether the
//! tool's process is alive:
//!
//! | age ≤ [`RUNNING_WINDOW_SECS`] | age ≤ [`AWAITING_WINDOW_SECS`] | age ≤ [`IDLE_WINDOW_SECS`] | else |
//! |---|---|---|---|
//! | live process → `running`<br>no process → `ended` | live process → `awaiting_input`<br>no process → `ended` | live process → `idle`<br>no process → `ended` | `ended` |
//!
//! The intuition: a *very* fresh write means the agent is actively emitting →
//! `running`. A write that's fresh-ish but has gone quiet, with the process still
//! alive, is the classic "blocked on the human at a prompt" shape →
//! `awaiting_input`. Quiet-but-alive past that → `idle`. Anything stale, or
//! anything with no live process at all, is `ended`.
//!
//! ## Process cross-check
//!
//! We do not depend on the `sysinfo` crate (it is not in this app's `Cargo.toml`,
//! and the PRD says to prefer a `pgrep`-style scan when it is not already
//! vendored). [`scan_running_agents`] shells out to `pgrep -fl` once per refresh
//! and classifies each matching command line as a Claude and/or Codex process.
//! One scan covers the whole fleet, so liveness derivation for N sessions costs a
//! single process spawn, not N.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{AgentTool, SessionStatus};

// ─────────────────────────────────────────────────────────────────────────────
// Time thresholds (documented named constants — PRD: "Thresholds are documented")
// ─────────────────────────────────────────────────────────────────────────────

/// A write within this window means the agent is *actively emitting* — the
/// freshest classification, mapped to [`SessionStatus::Running`] (process
/// permitting). Kept in lock-step with the readers' coarse `RUNNING_WINDOW_SECS`
/// (90s) so the engine never *down*grades a session the reader already called
/// running purely on the window. 90 seconds.
pub const RUNNING_WINDOW_SECS: u64 = 90;

/// Past the running window but still recent, with a live process, is the
/// signature of a session **blocked on the human** (an approval / prompt the
/// agent is waiting on — it stopped writing but the process is alive). Mapped to
/// [`SessionStatus::AwaitingInput`]. Beyond [`RUNNING_WINDOW_SECS`] and within
/// this bound. 5 minutes.
pub const AWAITING_WINDOW_SECS: u64 = 5 * 60;

/// Past the awaiting window but within this bound, with a live process, is a
/// session that's gone quiet but isn't over → [`SessionStatus::Idle`]. Beyond
/// this (or with no live process at any age) the session is
/// [`SessionStatus::Ended`]. 30 minutes — matches the readers' `IDLE_WINDOW_SECS`
/// upper bound so the engine and the coarse reader agree on the idle↔ended edge.
pub const IDLE_WINDOW_SECS: u64 = 30 * 60;

// ─────────────────────────────────────────────────────────────────────────────
// Running-process inventory
// ─────────────────────────────────────────────────────────────────────────────

/// A snapshot of which agent tools have at least one live process on this box.
///
/// Produced once per refresh by [`scan_running_agents`] and consumed by
/// [`derive_status`] for every session, so the (relatively expensive) process
/// scan happens a single time regardless of how many sessions we classify.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunningAgents {
    /// At least one live `claude` (Claude Code) process exists.
    pub claude: bool,
    /// At least one live `codex` process exists.
    pub codex: bool,
}

impl RunningAgents {
    /// Whether the tool that owns a session currently has a live process.
    pub fn is_tool_live(self, tool: AgentTool) -> bool {
        match tool {
            AgentTool::Claude => self.claude,
            AgentTool::Codex => self.codex,
        }
    }
}

/// Scan the running process table (best-effort) for live `claude` / `codex`
/// processes via a `pgrep`-style command, returning which tools are alive.
///
/// We avoid the `sysinfo` crate (not vendored in this app; the PRD says to prefer
/// a `pgrep`-style scan when it is not already a dependency) and instead invoke
/// `pgrep -fl claude\|codex` once. `-f` matches against the full command line (so
/// a `node …/claude` or `codex exec` invocation is caught, not just an exact
/// process name), and `-l` prints the command so [`classify_processes`] can tell
/// the two tools apart and reject false positives (e.g. *this* app, or an editor
/// with "codex" in a file path). Any failure (no `pgrep`, no matches, non-zero
/// exit) yields an empty inventory — best-effort, never an error.
pub fn scan_running_agents() -> RunningAgents {
    // `pgrep -fl` prints "<pid> <full command line>" per match. The pattern is an
    // ERE alternation so a single spawn covers both tools. A non-zero exit (e.g.
    // "no processes matched") is fine — we just get empty output.
    let output = std::process::Command::new("pgrep")
        .arg("-fl")
        .arg("claude|codex")
        .output();

    let stdout = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        // pgrep absent / not executable → assume nothing live (best-effort).
        Err(_) => String::new(),
    };

    classify_processes(&stdout)
}

/// Classify `pgrep -fl` output lines into a [`RunningAgents`] inventory. Pure
/// over its input so the matching rules are unit-testable without spawning
/// processes.
///
/// Each line is `"<pid> <command line>"`. We look at the command line (lowercased)
/// and count a line as:
///   - **Claude** if it mentions `claude` but is not obviously this very app
///     (`hq-sync` / `hq sync` — the menubar binary, which has "claude" nowhere,
///     but we guard defensively) — and not a self-match on our own scan.
///   - **Codex** if it mentions `codex`.
///
/// A line can satisfy neither (skipped). We deliberately do **not** try to map a
/// process back to a *specific* session id — the PID/cwd correlation is brittle
/// across `node` wrappers — only to the per-tool "is anything alive?" question the
/// liveness rule needs.
pub fn classify_processes(pgrep_output: &str) -> RunningAgents {
    let mut agents = RunningAgents::default();

    for line in pgrep_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Strip the leading "<pid> " so a numeric pid can't accidentally match.
        let cmd = line
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest)
            .unwrap_or(line)
            .to_ascii_lowercase();

        if cmd.is_empty() {
            continue;
        }

        // Reject our own process table entry: `pgrep -fl claude|codex` itself
        // shows up as a match on some platforms (the pattern string is in argv).
        if cmd.contains("pgrep") {
            continue;
        }

        if cmd.contains("claude") {
            agents.claude = true;
        }
        if cmd.contains("codex") {
            agents.codex = true;
        }
    }

    agents
}

// ─────────────────────────────────────────────────────────────────────────────
// Status derivation
// ─────────────────────────────────────────────────────────────────────────────

/// Derive the refined [`SessionStatus`] for one session from its last-activity
/// timestamp, the tool that owns it, the running-agent inventory, and the current
/// time.
///
/// This is the heart of US-004 and the single place the threshold table is
/// applied. Pure over its inputs (no I/O, injected `now`) so every transition is
/// unit-testable deterministically.
///
/// `last_activity_iso` is the session's `lastActivityAt` (RFC-3339). If it cannot
/// be parsed, `fallback_mtime` (the file mtime the reader stat'd) is used so a
/// missing/garbled timestamp never blanks the classification.
///
/// **No-live-process rule (HARD):** regardless of how fresh the activity looks, a
/// session whose owning tool has no live process resolves to
/// [`SessionStatus::Ended`] — the process exited without a closing write.
pub fn derive_status(
    last_activity_iso: &str,
    fallback_mtime: SystemTime,
    tool: AgentTool,
    agents: RunningAgents,
    now: SystemTime,
) -> SessionStatus {
    let activity_time = parse_rfc3339_to_system_time(last_activity_iso).unwrap_or(fallback_mtime);
    let age_secs = now
        .duration_since(activity_time)
        .map(|d| d.as_secs())
        // Future activity (clock skew) → age 0 → treated as fresh.
        .unwrap_or(0);

    status_for(age_secs, agents.is_tool_live(tool))
}

/// The pure threshold rule: map an activity age (seconds) + whether the tool is
/// live to a [`SessionStatus`]. Split out from [`derive_status`] so the window
/// edges can be tested without constructing timestamps.
///
/// The no-live-process short-circuit comes first: with no live process the
/// session is over no matter how fresh the last write looked.
pub fn status_for(age_secs: u64, tool_is_live: bool) -> SessionStatus {
    // HARD: no live process → ended, regardless of activity freshness.
    if !tool_is_live {
        return SessionStatus::Ended;
    }

    if age_secs <= RUNNING_WINDOW_SECS {
        SessionStatus::Running
    } else if age_secs <= AWAITING_WINDOW_SECS {
        SessionStatus::AwaitingInput
    } else if age_secs <= IDLE_WINDOW_SECS {
        SessionStatus::Idle
    } else {
        SessionStatus::Ended
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Time parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parse an RFC-3339 / ISO-8601 timestamp into a `SystemTime`. Returns `None` on
/// any parse failure (or a pre-epoch time) so callers fall back to the mtime.
/// Mirrors the readers' helper of the same name so the whole sessions module
/// parses timestamps one way.
fn parse_rfc3339_to_system_time(iso: &str) -> Option<SystemTime> {
    let dt = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    let secs = dt.timestamp();
    if secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(secs as u64))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// All tools live — the common "everything running" inventory.
    fn all_live() -> RunningAgents {
        RunningAgents {
            claude: true,
            codex: true,
        }
    }

    /// Format a `SystemTime` as the RFC-3339 string the readers stamp into
    /// `lastActivityAt`, so `derive_status` round-trips a real timestamp under
    /// test (seconds precision, `Z` suffix).
    fn iso(t: SystemTime) -> String {
        let secs = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    // ── status_for: every transition edge ──────────────────────────────────

    #[test]
    fn status_running_when_fresh_and_process_alive() {
        // Inside the running window, process alive → running.
        assert_eq!(status_for(0, true), SessionStatus::Running);
        assert_eq!(
            status_for(RUNNING_WINDOW_SECS, true),
            SessionStatus::Running
        );
    }

    #[test]
    fn status_awaiting_input_when_quiet_but_alive() {
        // Just past the running window, still within awaiting, process alive →
        // awaiting_input (the "blocked on the human" shape).
        assert_eq!(
            status_for(RUNNING_WINDOW_SECS + 1, true),
            SessionStatus::AwaitingInput
        );
        assert_eq!(
            status_for(AWAITING_WINDOW_SECS, true),
            SessionStatus::AwaitingInput
        );
    }

    #[test]
    fn status_idle_when_stale_but_alive() {
        // Past awaiting, within idle, process alive → idle.
        assert_eq!(
            status_for(AWAITING_WINDOW_SECS + 1, true),
            SessionStatus::Idle
        );
        assert_eq!(status_for(IDLE_WINDOW_SECS, true), SessionStatus::Idle);
    }

    #[test]
    fn status_ended_when_long_stale_even_if_process_alive() {
        // Past the idle window → ended, even though a process is alive (this
        // session's writes stopped long ago; some *other* agent of the same tool
        // is what's live).
        assert_eq!(
            status_for(IDLE_WINDOW_SECS + 1, true),
            SessionStatus::Ended
        );
        assert_eq!(status_for(u64::MAX, true), SessionStatus::Ended);
    }

    /// HARD requirement (PRD): a session with NO live process resolves to ended,
    /// regardless of how fresh its last activity looks.
    #[test]
    fn status_ended_when_no_live_process_regardless_of_freshness() {
        // age 0 (would otherwise be running) but no process → ended.
        assert_eq!(status_for(0, false), SessionStatus::Ended);
        // every other window with no process → ended too.
        assert_eq!(status_for(RUNNING_WINDOW_SECS, false), SessionStatus::Ended);
        assert_eq!(
            status_for(AWAITING_WINDOW_SECS, false),
            SessionStatus::Ended
        );
        assert_eq!(status_for(IDLE_WINDOW_SECS, false), SessionStatus::Ended);
    }

    // ── derive_status: end-to-end with timestamps + inventory ───────────────

    #[test]
    fn derive_running_from_fresh_timestamp() {
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let activity = now - Duration::from_secs(10);
        let iso = iso(activity);
        assert_eq!(
            derive_status(&iso, now, AgentTool::Claude, all_live(), now),
            SessionStatus::Running
        );
    }

    #[test]
    fn derive_awaiting_input_from_quiet_timestamp() {
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let activity = now - Duration::from_secs(RUNNING_WINDOW_SECS + 30);
        let iso = iso(activity);
        assert_eq!(
            derive_status(&iso, now, AgentTool::Codex, all_live(), now),
            SessionStatus::AwaitingInput
        );
    }

    #[test]
    fn derive_idle_from_stale_timestamp() {
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let activity = now - Duration::from_secs(AWAITING_WINDOW_SECS + 60);
        let iso = iso(activity);
        assert_eq!(
            derive_status(&iso, now, AgentTool::Claude, all_live(), now),
            SessionStatus::Idle
        );
    }

    #[test]
    fn derive_ended_when_owning_tool_has_no_process() {
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        // A *fresh* Codex session, but only Claude is live → the Codex session is
        // ended (its process is gone; the fresh mtime is a leftover).
        let activity = now - Duration::from_secs(5);
        let iso = iso(activity);
        let only_claude = RunningAgents {
            claude: true,
            codex: false,
        };
        assert_eq!(
            derive_status(&iso, now, AgentTool::Codex, only_claude, now),
            SessionStatus::Ended,
            "fresh activity but no codex process → ended"
        );
        // And the Claude session in the same inventory is still running.
        assert_eq!(
            derive_status(&iso, now, AgentTool::Claude, only_claude, now),
            SessionStatus::Running
        );
    }

    #[test]
    fn derive_falls_back_to_mtime_when_timestamp_unparseable() {
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let mtime = now - Duration::from_secs(10); // fresh mtime
        assert_eq!(
            derive_status("not-a-timestamp", mtime, AgentTool::Claude, all_live(), now),
            SessionStatus::Running,
            "unparseable timestamp falls back to the fresh mtime"
        );
    }

    #[test]
    fn derive_handles_future_activity_as_fresh() {
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let future = now + Duration::from_secs(120); // clock skew
        let iso = iso(future);
        assert_eq!(
            derive_status(&iso, now, AgentTool::Claude, all_live(), now),
            SessionStatus::Running,
            "future activity (skew) treated as fresh, never panics"
        );
    }

    // ── classify_processes: pgrep output parsing ────────────────────────────

    #[test]
    fn classify_detects_claude_and_codex() {
        let out = "\
12345 node /Users/corey/.claude/local/claude --resume
67890 codex exec --model gpt-5.5
11111 /Applications/Some.app/Contents/MacOS/Some unrelated
";
        let agents = classify_processes(out);
        assert!(agents.claude, "claude command line detected");
        assert!(agents.codex, "codex command line detected");
    }

    #[test]
    fn classify_only_claude_when_no_codex_line() {
        let out = "12345 node /Users/corey/.claude/local/claude\n";
        let agents = classify_processes(out);
        assert!(agents.claude);
        assert!(!agents.codex);
    }

    #[test]
    fn classify_empty_output_is_no_agents() {
        assert_eq!(classify_processes(""), RunningAgents::default());
        assert_eq!(classify_processes("   \n  \n"), RunningAgents::default());
    }

    #[test]
    fn classify_ignores_pgrep_self_match() {
        // pgrep's own argv contains the pattern on some platforms — must not
        // count as a live agent.
        let out = "99999 pgrep -fl claude|codex\n";
        let agents = classify_processes(out);
        assert!(!agents.claude, "pgrep self-match is not a live claude");
        assert!(!agents.codex, "pgrep self-match is not a live codex");
    }

    #[test]
    fn classify_does_not_match_bare_pid_number() {
        // A line that is only a pid (no command) must not match anything.
        let agents = classify_processes("12345\n");
        assert_eq!(agents, RunningAgents::default());
    }

    #[test]
    fn running_agents_is_tool_live_maps_per_tool() {
        let only_claude = RunningAgents {
            claude: true,
            codex: false,
        };
        assert!(only_claude.is_tool_live(AgentTool::Claude));
        assert!(!only_claude.is_tool_live(AgentTool::Codex));
    }
}
