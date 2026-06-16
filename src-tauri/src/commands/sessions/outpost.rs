//! Desktop outpost subscriber + box-level status + merge (US-011).
//!
//! This is the desktop half of the per-session outpost path. The outpost VM
//! publishes a compact `AgentSession[]` (origin=outpost) to the realtime topic
//! `hq/{personUid}/sessions` on the desktop's poll cadence (US-009), and the
//! realtime session policy scopes publish/subscribe to that topic per person
//! (US-010). This module:
//!
//!   1. **Subscribes** to `hq/{personUid}/sessions` over MQTT-over-WSS — reusing
//!      the exact credential/presign/subscribe pattern from `dm_mqtt.rs` — and,
//!      unlike DM (where the message is only a wake signal), **parses the
//!      payload** into `AgentSession[]`, stamps `origin=outpost`, and stores it.
//!   2. **Merges** those outpost sessions into the SAME Mission Control snapshot
//!      as the local readers, so local + outpost agents appear in one fleet
//!      (`outpost_sessions_snapshot` is folded into `collect_snapshot`).
//!   3. **Sources the box-level status** from `GET /outpost/status` (up/down,
//!      runtime, relay, last-seen) so the UI can head the outpost group with a
//!      status card (design.md "Outpost status card (US-011)").
//!   4. **Falls back to S3** when MQTT is unavailable — the same heartbeat payload
//!      is polled from the vault — and a **stale-after timeout** drops outpost
//!      sessions that stop reporting, so a dead box doesn't leave ghost rows.
//!
//! ## Shared state
//!
//! [`OutpostStore`] is a process-global, mutex-guarded cache of the latest
//! heartbeat (`AgentSession[]` + the instant it landed) plus the latest box
//! status. The MQTT receiver and the S3-fallback poller both write it; the
//! Mission Control snapshot assembly reads it. Reads apply the stale-after
//! timeout so a heartbeat that stopped arriving evaporates without any writer
//! having to actively clear it.
//!
//! ## Failure posture
//!
//! Every path here is **best-effort and non-fatal**, exactly like `dm_mqtt.rs`:
//! a creds/presign/connect failure logs and retries with backoff while the S3
//! poll keeps the store warm; a `/outpost/status` failure leaves the last known
//! box status (aged by the stale window) rather than blanking it. The local
//! fleet is never affected by an outpost-path failure.
//!
//! ## Log codes (`outpost` tag)
//!
//!   `OUTPOST_MQTT_CREDS_FAIL` / `OUTPOST_MQTT_PRESIGN_FAIL` /
//!   `OUTPOST_MQTT_CONNECT_OK` / `OUTPOST_MQTT_SUBSCRIBED` /
//!   `OUTPOST_MQTT_HEARTBEAT` / `OUTPOST_MQTT_DISCONNECT` / `OUTPOST_MQTT_FALLBACK`
//!   / `OUTPOST_S3_POLL_OK` / `OUTPOST_STATUS_OK` / `OUTPOST_STATUS_FAIL`. No
//!   secrets are ever logged (never the presigned URL, never the creds).

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::logfile::log;

use super::{AgentOrigin, AgentSession};

const LOG_TAG: &str = "outpost";

/// AWS service name for the IoT MQTT broker SigV4 signature. Fixed by AWS.
/// Same value `dm_mqtt.rs` uses — kept local so the two modules don't couple.
const IOT_SERVICE: &str = "iotdevicegateway";

/// Reconnect backoff floor (mirrors `dm_mqtt::BACKOFF_MIN`).
const BACKOFF_MIN: Duration = Duration::from_secs(5);
/// Reconnect backoff ceiling (mirrors `dm_mqtt::BACKOFF_MAX`).
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Presigned-WSS validity (mirrors `dm_mqtt::PRESIGN_EXPIRES`).
const PRESIGN_EXPIRES: Duration = Duration::from_secs(3600);

/// How long an outpost heartbeat is trusted before its sessions are dropped.
///
/// The outpost emitter (US-009) publishes on the desktop poll cadence (~5s), so
/// missing several beats means the box stopped reporting. 90 seconds matches the
/// stale-timeout note in design.md ("N outpost sessions dropped after the 90s
/// stale timeout") and the codex-relay staleness posture on the server. Past
/// this age, [`OutpostStore::current`] returns no sessions and flags the status
/// stale, so a dead box leaves no ghost rows.
pub const HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(90);

/// S3-fallback poll cadence. Only the fallback path uses a timer — the MQTT path
/// is push. Kept slightly longer than the live cadence since it's the backstop.
const S3_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// `/outpost/status` refresh cadence (the box card is control-plane state, not
/// per-session, so it doesn't need the live heartbeat cadence).
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(20);

// ─────────────────────────────────────────────────────────────────────────────
// Box-level status (sourced from GET /outpost/status)
// ─────────────────────────────────────────────────────────────────────────────

/// The desktop-facing outpost box status that heads the outpost group
/// (design.md "Outpost status card (US-011)").
///
/// A normalised projection of the hq-pro `StatusResponse` — the card only needs
/// up/down, runtime, relay, and an `ip · region` line, plus a last-seen relative
/// (driven by the heartbeat freshness, not a server field). camelCase on the
/// wire so the Svelte card reads it without remapping, matching the rest of the
/// sessions contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutpostStatus {
    /// Whether the box reads as up (provisioned, instance running). Drives the
    /// green-vs-red card. Best-effort: see [`derive_box_up`].
    pub up: bool,
    /// The agent runtime the box runs (`claude` / `codex`), surfaced as RUNTIME.
    pub runtime: String,
    /// Whether the relay reads as connected (the RELAY stat: green connected /
    /// red disconnected). For codex boxes this tracks the codex relay state; for
    /// claude boxes a ready+running box is treated as connected.
    pub relay_connected: bool,
    /// `ip · region` meta line. Either half may be empty when the row omits it.
    pub ip: String,
    pub region: String,
    /// ISO-8601 of the most recent heartbeat we received (the LAST SEEN stat).
    /// Empty when we have never received a beat. Set by the store, not the
    /// status fetch, so "last seen" reflects actual per-session reporting.
    pub last_seen_at: String,
    /// `true` once the heartbeat has gone stale past [`HEARTBEAT_STALE_AFTER`] —
    /// the card then renders its down/last-seen treatment and the stale-sessions
    /// note even if `/outpost/status` still says the box exists.
    pub stale: bool,
}

impl OutpostStatus {
    /// A status for "no outpost configured / never seen" — the card renders a
    /// neutral down state. Used as the default when `/outpost/status` 404s (no
    /// box) or has never succeeded.
    fn unknown() -> Self {
        OutpostStatus {
            up: false,
            runtime: String::new(),
            relay_connected: false,
            ip: String::new(),
            region: String::new(),
            last_seen_at: String::new(),
            stale: false,
        }
    }
}

/// The raw `GET /outpost/status` response — only the fields the box card needs.
/// Extra fields on the wire are ignored (serde default). Mirrors hq-pro's
/// `StatusResponse` (`src/outpost/types.ts`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOutpostStatus {
    #[serde(default)]
    state: String,
    #[serde(default)]
    instance_state: String,
    #[serde(default)]
    agent_runtime: String,
    #[serde(default)]
    static_ip: String,
    #[serde(default)]
    region: String,
    /// Derived codex sub-state (`connected` / `awaiting-login` / `disabled` / …).
    #[serde(default)]
    codex_state: String,
    /// Raw relay-state passthrough (`connected` / `error` / …), codex rows only.
    #[serde(default)]
    codex_relay_status: String,
}

/// Whether the box reads as UP from a raw status (pure, testable).
///
/// The brainstorm caveat (companies/indigo/.../outpost-ready-state-is-stale-auth-flag.md)
/// is that `state: "ready"` is a stale boot flag, not live health — so we require
/// BOTH the provisioning `state` to be `ready` AND the live `instanceState` to be
/// `running`. A box that is provisioning/bootstrapping/failed, or whose instance
/// isn't running, reads down.
fn derive_box_up(raw: &RawOutpostStatus) -> bool {
    raw.state == "ready" && raw.instance_state == "running"
}

/// Whether the relay reads as connected from a raw status (pure, testable).
///
/// For a codex runtime, the relay is the codex remote-control channel — connected
/// iff the derived `codexState` (or the raw relay status) is `connected`. For a
/// claude runtime there is no separate relay, so an up box is treated as
/// connected and a down box as disconnected.
fn derive_relay_connected(raw: &RawOutpostStatus) -> bool {
    let is_codex = raw.agent_runtime == "codex"
        || !raw.codex_state.is_empty()
        || !raw.codex_relay_status.is_empty();
    if is_codex {
        raw.codex_state == "connected" || raw.codex_relay_status == "connected"
    } else {
        derive_box_up(raw)
    }
}

/// Project a raw `/outpost/status` response onto the desktop [`OutpostStatus`]
/// card shape (pure over its input; `last_seen_at`/`stale` are filled in by the
/// store from heartbeat freshness, not here).
fn project_status(raw: &RawOutpostStatus) -> OutpostStatus {
    OutpostStatus {
        up: derive_box_up(raw),
        runtime: if raw.agent_runtime.is_empty() {
            "claude".to_string()
        } else {
            raw.agent_runtime.clone()
        },
        relay_connected: derive_relay_connected(raw),
        ip: raw.static_ip.clone(),
        region: raw.region.clone(),
        last_seen_at: String::new(),
        stale: false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared outpost store (heartbeat + status, stale-aware reads)
// ─────────────────────────────────────────────────────────────────────────────

/// The mutable inner state behind [`OutpostStore`].
#[derive(Debug, Default)]
struct OutpostState {
    /// The most recent heartbeat's sessions (already stamped origin=outpost).
    sessions: Vec<AgentSession>,
    /// When that heartbeat landed. `None` until the first beat.
    last_heartbeat: Option<SystemTime>,
    /// The most recent box status from `/outpost/status`. `None` until first fetch.
    status: Option<OutpostStatus>,
}

/// Process-global outpost cache shared by the MQTT receiver, the S3 fallback
/// poller, the status poller, and the snapshot assembly. A single `Mutex` is
/// ample — writes are a handful per poll cycle and reads happen once per
/// snapshot. Lazily initialised so tests and non-outpost builds pay nothing.
pub struct OutpostStore;

fn state() -> &'static Mutex<OutpostState> {
    static STORE: OnceLock<Mutex<OutpostState>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(OutpostState::default()))
}

impl OutpostStore {
    /// Record a fresh heartbeat: replace the cached outpost sessions (each
    /// re-stamped `origin=outpost` defensively) and mark the landing time. The
    /// `at` instant is injected so the stale logic is deterministic under test.
    pub fn record_heartbeat(sessions: Vec<AgentSession>, at: SystemTime) {
        let stamped = sessions
            .into_iter()
            .map(|mut s| {
                s.origin = AgentOrigin::Outpost;
                s
            })
            .collect();
        let mut guard = state().lock().expect("outpost store mutex poisoned");
        guard.sessions = stamped;
        guard.last_heartbeat = Some(at);
    }

    /// Record the latest box status from `/outpost/status`.
    pub fn record_status(status: OutpostStatus) {
        let mut guard = state().lock().expect("outpost store mutex poisoned");
        guard.status = Some(status);
    }

    /// The current outpost view at instant `now`, with the stale-after timeout
    /// applied (pure read; injected `now` for tests):
    ///
    ///   - sessions: the cached heartbeat's sessions IF the beat is within
    ///     [`HEARTBEAT_STALE_AFTER`], else empty (stale → dropped).
    ///   - status: the last box status, with `last_seen_at` set from the most
    ///     recent heartbeat and `stale` set when that beat is past the window
    ///     (or never happened). When no `/outpost/status` has succeeded but we
    ///     HAVE seen a heartbeat, a minimal up status is synthesised so the card
    ///     still reflects last-seen.
    pub fn current(now: SystemTime) -> OutpostView {
        let guard = state().lock().expect("outpost store mutex poisoned");
        Self::view_from(&guard, now)
    }

    /// Pure projection of an [`OutpostState`] at `now` — split out so the
    /// stale-timeout rule is unit-testable without the global mutex.
    fn view_from(st: &OutpostState, now: SystemTime) -> OutpostView {
        let fresh = st
            .last_heartbeat
            .map(|t| now.duration_since(t).map(|d| d < HEARTBEAT_STALE_AFTER).unwrap_or(true))
            .unwrap_or(false);

        let sessions = if fresh { st.sessions.clone() } else { Vec::new() };

        let last_seen_at = st
            .last_heartbeat
            .map(system_time_to_iso)
            .unwrap_or_default();

        let status = match &st.status {
            Some(base) => {
                let mut s = base.clone();
                s.last_seen_at = last_seen_at.clone();
                // A box that stopped beating reads stale regardless of the
                // last control-plane state — the card flips to its down/last-seen
                // treatment and shows the stale-sessions note.
                s.stale = !fresh;
                if !fresh {
                    s.up = false;
                    s.relay_connected = false;
                }
                Some(s)
            }
            // No /outpost/status yet but we have heartbeats → synthesise a
            // minimal card so the operator still sees up + last-seen.
            None if st.last_heartbeat.is_some() => Some(OutpostStatus {
                up: fresh,
                runtime: String::new(),
                relay_connected: fresh,
                ip: String::new(),
                region: String::new(),
                last_seen_at,
                stale: !fresh,
            }),
            // Nothing known at all → no card (the UI omits the outpost group).
            None => None,
        };

        OutpostView { sessions, status }
    }

    /// Test-only reset of the global store between cases.
    #[cfg(test)]
    fn reset() {
        let mut guard = state().lock().expect("outpost store mutex poisoned");
        *guard = OutpostState::default();
    }
}

/// The stale-aware outpost view folded into a Mission Control snapshot.
#[derive(Debug, Clone, Default)]
pub struct OutpostView {
    /// Fresh outpost sessions (empty when the heartbeat is stale).
    pub sessions: Vec<AgentSession>,
    /// The box status card, or `None` when no outpost is known.
    pub status: Option<OutpostStatus>,
}

/// Snapshot accessor used by `sessions::collect_snapshot` — returns the current
/// stale-aware outpost view from the global store.
pub fn outpost_view(now: SystemTime) -> OutpostView {
    OutpostStore::current(now)
}

// ─────────────────────────────────────────────────────────────────────────────
// Heartbeat payload parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parse an MQTT/S3 heartbeat payload into outpost sessions (pure, testable).
///
/// The emitter publishes the compact `AgentSession[]` directly (US-009), but we
/// tolerate a `{ "sessions": [...] }` envelope too, so the wire format can gain
/// metadata later without breaking the desktop. Each session is re-stamped
/// `origin=outpost` (the emitter already sets it, but we never trust the wire).
/// A malformed payload yields `Err` so the caller can log + ignore rather than
/// poisoning the store.
pub fn parse_heartbeat(payload: &[u8]) -> Result<Vec<AgentSession>, String> {
    // Try the bare array first (the documented shape), then the envelope.
    if let Ok(sessions) = serde_json::from_slice::<Vec<AgentSession>>(payload) {
        return Ok(stamp_outpost(sessions));
    }
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(default)]
        sessions: Vec<AgentSession>,
    }
    let env: Envelope =
        serde_json::from_slice(payload).map_err(|e| format!("heartbeat parse: {e}"))?;
    Ok(stamp_outpost(env.sessions))
}

fn stamp_outpost(sessions: Vec<AgentSession>) -> Vec<AgentSession> {
    sessions
        .into_iter()
        .map(|mut s| {
            s.origin = AgentOrigin::Outpost;
            s
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Realtime credentials + sessions topic (reuses the dm_mqtt pattern)
// ─────────────────────────────────────────────────────────────────────────────

/// STS credentials block inside the realtime-credentials response (same shape as
/// `dm_mqtt::RealtimeCredentials`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
    #[allow(dead_code)]
    expiration: String,
}

/// Response of `POST /v1/realtime/credentials`. The server returns the caller's
/// DM topic (`hq/<personUid>/dm`); the per-person STS policy scopes the whole
/// `hq/<personUid>/*` subtree (US-010), so the sessions topic is reachable by
/// swapping the leaf — see [`sessions_topic_for`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeCredsResponse {
    credentials: RealtimeCredentials,
    iot_endpoint: String,
    region: String,
    /// The caller's own topic, e.g. `hq/<personUid>/dm`.
    topic: String,
}

/// Derive the sessions topic from the realtime-credentials `topic`
/// (`hq/<personUid>/dm` → `hq/<personUid>/sessions`). Pure + testable. Falls back
/// to swapping just the trailing segment so an unexpected leaf still lands under
/// the same person prefix the STS policy scopes.
pub fn sessions_topic_for(creds_topic: &str) -> String {
    match creds_topic.rsplit_once('/') {
        Some((prefix, _leaf)) => format!("{prefix}/sessions"),
        None => "sessions".to_string(),
    }
}

/// Fetch short-lived realtime credentials from the vault (identical to
/// `dm_mqtt::fetch_realtime_credentials`, kept local so the two background tasks
/// don't couple). Returns `Err` with a short reason on any failure.
async fn fetch_realtime_credentials() -> Result<RealtimeCredsResponse, String> {
    let access_token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| format!("vault url: {e}"))?;

    let url = format!("{}/v1/realtime/credentials", base_url);

    let resp = build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("status={}", status.as_u16()));
    }

    resp.json::<RealtimeCredsResponse>()
        .await
        .map_err(|e| format!("parse: {e}"))
}

// ── SigV4 presign (mirrors dm_mqtt::build_signed_wss_url) ────────────────────

/// Presign an AWS IoT MQTT-over-WSS URL. Identical algorithm to
/// `dm_mqtt::build_signed_wss_url` — the session token is appended AFTER signing
/// (AWS IoT 403s if it's in the signed query). Pure + deterministic given `now`.
fn build_signed_wss_url(
    access_key_id: &str,
    secret_access_key: &str,
    session_token: &str,
    endpoint: &str,
    region: &str,
    now: SystemTime,
) -> Result<String, String> {
    use aws_credential_types::Credentials;
    use aws_sigv4::http_request::{
        sign, SignableBody, SignableRequest, SignatureLocation, SigningSettings,
    };
    use aws_sigv4::sign::v4;
    use aws_smithy_runtime_api::client::identity::Identity;

    let creds = Credentials::new(
        access_key_id,
        secret_access_key,
        None,
        None,
        "hq-sync-realtime-iot",
    );
    let identity: Identity = creds.into();

    let mut settings = SigningSettings::default();
    settings.signature_location = SignatureLocation::QueryParams;
    settings.expires_in = Some(PRESIGN_EXPIRES);

    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(IOT_SERVICE)
        .time(now)
        .settings(settings)
        .build()
        .map_err(|e| format!("signing params: {e}"))?
        .into();

    let signable_uri = format!("wss://{}/mqtt", endpoint);
    let host_header = endpoint.to_string();
    let headers = [("host", host_header.as_str())];

    let signable = SignableRequest::new(
        "GET",
        &signable_uri,
        headers.iter().copied(),
        SignableBody::Bytes(&[]),
    )
    .map_err(|e| format!("signable request: {e}"))?;

    let signing_output = sign(signable, &params).map_err(|e| format!("sign: {e}"))?;
    let (instructions, _signature) = signing_output.into_parts();

    let query_pairs = instructions.params();
    if query_pairs.is_empty() {
        return Err("presign produced no query params".to_string());
    }
    let query = query_pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    Ok(format!(
        "wss://{}/mqtt?{}&X-Amz-Security-Token={}",
        endpoint,
        query,
        aws_uri_encode(session_token),
    ))
}

/// RFC3986 / AWS-canonical percent-encoding (mirrors `dm_mqtt::aws_uri_encode`).
fn aws_uri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Stable MQTT client id (mirrors `dm_mqtt::client_id`, distinct prefix so the
/// two connections from one machine never collide on IoT's client-id reaping).
fn client_id() -> String {
    let machine = crate::commands::config::ensure_machine_id().unwrap_or_else(|_| "unknown".into());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!(
        "hqsync-sess-{}-{}",
        &machine.chars().take(34).collect::<String>(),
        &suffix[..8]
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MQTT receive loop (subscribe + parse, unlike dm_mqtt's wake-only path)
// ─────────────────────────────────────────────────────────────────────────────

/// One connect→subscribe→receive cycle for the sessions topic. On each inbound
/// Publish the payload is parsed into `AgentSession[]` and recorded in the store
/// (unlike DM, where the message is only a wake signal). Returns `Err` on any
/// connection failure so the caller backs off + retries.
async fn run_once(creds: &RealtimeCredsResponse) -> Result<(), String> {
    use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};

    let now = SystemTime::now();
    let url = build_signed_wss_url(
        &creds.credentials.access_key_id,
        &creds.credentials.secret_access_key,
        &creds.credentials.session_token,
        &creds.iot_endpoint,
        &creds.region,
        now,
    )
    .map_err(|e| {
        log(LOG_TAG, &format!("OUTPOST_MQTT_PRESIGN_FAIL {e}"));
        e
    })?;

    let topic = sessions_topic_for(&creds.topic);

    let mut opts = MqttOptions::new(client_id(), url, 443);
    opts.set_transport(Transport::wss_with_default_config());
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_clean_session(true);

    let (client, mut eventloop) = AsyncClient::new(opts, 10);

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                log(LOG_TAG, "OUTPOST_MQTT_CONNECT_OK");
                if let Err(e) = client.subscribe(topic.clone(), QoS::AtMostOnce).await {
                    return Err(format!("subscribe: {e}"));
                }
                log(LOG_TAG, &format!("OUTPOST_MQTT_SUBSCRIBED topic={topic}"));
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                // The payload IS the heartbeat — parse it into outpost sessions.
                match parse_heartbeat(&p.payload) {
                    Ok(sessions) => {
                        let n = sessions.len();
                        OutpostStore::record_heartbeat(sessions, SystemTime::now());
                        log(LOG_TAG, &format!("OUTPOST_MQTT_HEARTBEAT sessions={n}"));
                    }
                    Err(e) => log(LOG_TAG, &format!("OUTPOST_MQTT_HEARTBEAT_BAD {e}")),
                }
            }
            Ok(_) => { /* SubAck, PingResp, Outgoing, etc. — ignore. */ }
            Err(e) => return Err(format!("eventloop: {e}")),
        }
    }
}

/// Spawn the outpost sessions MQTT receiver. Called from `main.rs` `.setup()`,
/// macOS-gated like the other realtime tasks. On any failure it logs and retries
/// with capped exponential backoff; the S3 fallback poller keeps the store warm
/// meanwhile, so an MQTT outage never blanks the outpost group — it just degrades
/// to the slower poll cadence (PRD US-011 resilience criterion).
pub fn setup_outpost_mqtt_receiver(_app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;

        let mut backoff = BACKOFF_MIN;
        loop {
            match fetch_realtime_credentials().await {
                Ok(creds) => {
                    let started = SystemTime::now();
                    if let Err(e) = run_once(&creds).await {
                        log(LOG_TAG, &format!("OUTPOST_MQTT_DISCONNECT {e}"));
                    }
                    if started.elapsed().map(|d| d > Duration::from_secs(30)).unwrap_or(false) {
                        backoff = BACKOFF_MIN;
                    }
                }
                Err(e) => log(LOG_TAG, &format!("OUTPOST_MQTT_CREDS_FAIL {e}")),
            }

            log(
                LOG_TAG,
                &format!("OUTPOST_MQTT_FALLBACK reconnect in {}s (S3 poll active)", backoff.as_secs()),
            );
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// S3 heartbeat fallback + box-status pollers
// ─────────────────────────────────────────────────────────────────────────────

/// Fetch the S3-vault heartbeat (the MQTT-unavailable fallback) and record it.
///
/// The emitter writes the same `AgentSession[]` payload to a vault prefix when it
/// can't reach the realtime fabric; the desktop reads it via the vault API's
/// signed-read endpoint. Best-effort: a 404 (no heartbeat object) or any error
/// just leaves the store untouched so the stale-timeout takes over.
async fn poll_s3_heartbeat_once() {
    let payload = match fetch_s3_heartbeat().await {
        Ok(bytes) => bytes,
        Err(e) => {
            log(LOG_TAG, &format!("OUTPOST_S3_POLL_MISS {e}"));
            return;
        }
    };
    match parse_heartbeat(&payload) {
        Ok(sessions) => {
            let n = sessions.len();
            OutpostStore::record_heartbeat(sessions, SystemTime::now());
            log(LOG_TAG, &format!("OUTPOST_S3_POLL_OK sessions={n}"));
        }
        Err(e) => log(LOG_TAG, &format!("OUTPOST_S3_POLL_BAD {e}")),
    }
}

/// GET the S3-vault heartbeat object via the vault API (`/v1/outpost/heartbeat`,
/// served as the signed-read fallback for the sessions payload). Returns the raw
/// body bytes or a short error.
async fn fetch_s3_heartbeat() -> Result<Vec<u8>, String> {
    let access_token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;
    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| format!("vault url: {e}"))?;
    let url = format!("{}/v1/outpost/heartbeat", base_url);

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("status={}", resp.status().as_u16()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("body: {e}"))
}

/// Fetch `GET /outpost/status` and record the projected box card. Best-effort: a
/// 404 (no outpost) records the neutral `unknown` status so the card reads down
/// rather than disappearing; any other error leaves the last known status.
async fn poll_outpost_status_once() {
    match fetch_outpost_status().await {
        Ok(Some(raw)) => {
            OutpostStore::record_status(project_status(&raw));
            log(LOG_TAG, "OUTPOST_STATUS_OK");
        }
        Ok(None) => {
            // 404 → no box provisioned for this user. Record the neutral down
            // card only if we have never seen a heartbeat (don't stomp a live one).
            if OutpostStore::current(SystemTime::now()).status.is_none() {
                OutpostStore::record_status(OutpostStatus::unknown());
            }
            log(LOG_TAG, "OUTPOST_STATUS_NONE");
        }
        Err(e) => log(LOG_TAG, &format!("OUTPOST_STATUS_FAIL {e}")),
    }
}

/// GET `/outpost/status`. Returns `Ok(None)` on a 404 (no outpost for this user),
/// `Ok(Some)` on success, `Err` on any other failure.
async fn fetch_outpost_status() -> Result<Option<RawOutpostStatus>, String> {
    let access_token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;
    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| format!("vault url: {e}"))?;
    let url = format!("{}/outpost/status", base_url);

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;

    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("status={}", resp.status().as_u16()));
    }
    resp.json::<RawOutpostStatus>()
        .await
        .map(Some)
        .map_err(|e| format!("parse: {e}"))
}

/// Spawn the S3-heartbeat fallback poller and the box-status poller. Called from
/// `main.rs` `.setup()` alongside the MQTT receiver. Both run on independent
/// interval timers (mirrors the share/dm poller pattern) and are best-effort.
pub fn setup_outpost_pollers(_app: AppHandle) {
    // S3 heartbeat fallback — keeps the store warm when MQTT is down.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(7)).await;
        let mut ticker = tokio::time::interval(S3_POLL_INTERVAL);
        loop {
            ticker.tick().await;
            poll_s3_heartbeat_once().await;
        }
    });

    // Box-level status — heads the outpost group.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        let mut ticker = tokio::time::interval(STATUS_POLL_INTERVAL);
        loop {
            ticker.tick().await;
            poll_outpost_status_once().await;
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Time helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Format a `SystemTime` as an RFC-3339 / `Z` string (seconds precision),
/// matching how the readers stamp `lastActivityAt`.
fn system_time_to_iso(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::sessions::{AgentTool, SessionStatus};

    fn outpost_session(id: &str) -> AgentSession {
        AgentSession {
            id: id.to_string(),
            tool: AgentTool::Claude,
            // Deliberately LOCAL on the wire so we prove the store re-stamps it.
            origin: AgentOrigin::Local,
            cwd: "/home/outpost/repos/thing".to_string(),
            project: "thing".to_string(),
            company: "indigo".to_string(),
            model: "claude-opus-4-8".to_string(),
            status: SessionStatus::Running,
            started_at: "2026-06-15T18:00:00Z".to_string(),
            last_activity_at: "2026-06-15T18:43:20Z".to_string(),
            source: "outpost-heartbeat".to_string(),
        }
    }

    // ── heartbeat parsing ───────────────────────────────────────────────────

    #[test]
    fn parse_heartbeat_accepts_bare_array_and_stamps_outpost() {
        let payload = serde_json::to_vec(&vec![outpost_session("a"), outpost_session("b")]).unwrap();
        let sessions = parse_heartbeat(&payload).expect("bare array parses");
        assert_eq!(sessions.len(), 2);
        // Wire said local; the parser MUST force origin=outpost.
        assert!(sessions.iter().all(|s| s.origin == AgentOrigin::Outpost));
    }

    #[test]
    fn parse_heartbeat_accepts_envelope_shape() {
        let payload = serde_json::json!({ "sessions": [outpost_session("a")] });
        let bytes = serde_json::to_vec(&payload).unwrap();
        let sessions = parse_heartbeat(&bytes).expect("envelope parses");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].origin, AgentOrigin::Outpost);
    }

    #[test]
    fn parse_heartbeat_rejects_garbage() {
        assert!(parse_heartbeat(b"not json").is_err());
    }

    // ── sessions topic derivation ───────────────────────────────────────────

    #[test]
    fn sessions_topic_swaps_the_dm_leaf() {
        assert_eq!(sessions_topic_for("hq/prs_abc/dm"), "hq/prs_abc/sessions");
        // Any leaf under the same person prefix lands on /sessions.
        assert_eq!(sessions_topic_for("hq/prs_abc/share"), "hq/prs_abc/sessions");
    }

    // ── box-status derivation (up/down + relay) ─────────────────────────────

    #[test]
    fn box_up_requires_ready_state_and_running_instance() {
        // Ready + running → up (the only up combination — guards the stale-flag caveat).
        let up = RawOutpostStatus {
            state: "ready".into(),
            instance_state: "running".into(),
            ..Default::default()
        };
        assert!(derive_box_up(&up));

        // Ready flag but instance not running → DOWN (state:ready is a stale boot flag).
        let stale_flag = RawOutpostStatus {
            state: "ready".into(),
            instance_state: "stopped".into(),
            ..Default::default()
        };
        assert!(!derive_box_up(&stale_flag));

        // Still bootstrapping → down.
        let booting = RawOutpostStatus {
            state: "bootstrapping".into(),
            instance_state: "running".into(),
            ..Default::default()
        };
        assert!(!derive_box_up(&booting));
    }

    #[test]
    fn relay_connected_tracks_codex_state_for_codex_boxes() {
        let codex_connected = RawOutpostStatus {
            state: "ready".into(),
            instance_state: "running".into(),
            agent_runtime: "codex".into(),
            codex_state: "connected".into(),
            ..Default::default()
        };
        assert!(derive_relay_connected(&codex_connected));

        let codex_down = RawOutpostStatus {
            state: "ready".into(),
            instance_state: "running".into(),
            agent_runtime: "codex".into(),
            codex_state: "awaiting-login".into(),
            ..Default::default()
        };
        assert!(!derive_relay_connected(&codex_down), "codex relay not connected → disconnected");

        // Claude box: relay == up.
        let claude_up = RawOutpostStatus {
            state: "ready".into(),
            instance_state: "running".into(),
            agent_runtime: "claude".into(),
            ..Default::default()
        };
        assert!(derive_relay_connected(&claude_up));
    }

    #[test]
    fn project_status_fills_card_fields() {
        let raw = RawOutpostStatus {
            state: "ready".into(),
            instance_state: "running".into(),
            agent_runtime: "claude".into(),
            static_ip: "203.0.113.7".into(),
            region: "us-east-1".into(),
            ..Default::default()
        };
        let card = project_status(&raw);
        assert!(card.up);
        assert_eq!(card.runtime, "claude");
        assert!(card.relay_connected);
        assert_eq!(card.ip, "203.0.113.7");
        assert_eq!(card.region, "us-east-1");
    }

    // ── store: merge + stale-timeout (the US-011 e2e back-pressure) ──────────

    #[test]
    fn fresh_heartbeat_surfaces_outpost_sessions() {
        OutpostStore::reset();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        OutpostStore::record_heartbeat(vec![outpost_session("o1")], now);

        // Read 10s later — within the stale window → sessions present, marked outpost.
        let view = OutpostStore::current(now + Duration::from_secs(10));
        assert_eq!(view.sessions.len(), 1);
        assert_eq!(view.sessions[0].origin, AgentOrigin::Outpost);
        // A synthesised card (no /outpost/status yet) reads up + carries last-seen.
        let status = view.status.expect("synthesised card present");
        assert!(status.up);
        assert!(!status.stale);
        assert!(!status.last_seen_at.is_empty());
        OutpostStore::reset();
    }

    #[test]
    fn stale_heartbeat_drops_outpost_sessions_and_marks_card() {
        OutpostStore::reset();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        OutpostStore::record_status(OutpostStatus {
            up: true,
            runtime: "claude".into(),
            relay_connected: true,
            ip: "203.0.113.7".into(),
            region: "us-east-1".into(),
            last_seen_at: String::new(),
            stale: false,
        });
        OutpostStore::record_heartbeat(vec![outpost_session("o1")], now);

        // Read PAST the stale window → sessions dropped, card flips to stale/down.
        let after = now + HEARTBEAT_STALE_AFTER + Duration::from_secs(1);
        let view = OutpostStore::current(after);
        assert!(view.sessions.is_empty(), "stale heartbeat drops outpost sessions");
        let status = view.status.expect("card still present (last-seen)");
        assert!(status.stale, "card marked stale past the timeout");
        assert!(!status.up, "stale box reads down");
        assert!(!status.relay_connected, "stale box relay reads disconnected");
        // last-seen reflects the last heartbeat instant, not 'now'.
        assert!(!status.last_seen_at.is_empty());
        OutpostStore::reset();
    }

    #[test]
    fn no_outpost_known_yields_no_card() {
        OutpostStore::reset();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let view = OutpostStore::current(now);
        assert!(view.sessions.is_empty());
        assert!(view.status.is_none(), "no heartbeat + no status → no outpost card");
        OutpostStore::reset();
    }

    #[test]
    fn status_only_box_renders_card_without_sessions() {
        OutpostStore::reset();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        // /outpost/status succeeded but no per-session heartbeat yet (box up,
        // emitter not reporting): card present + up, no sessions, last-seen empty.
        OutpostStore::record_status(OutpostStatus {
            up: true,
            runtime: "claude".into(),
            relay_connected: true,
            ip: "203.0.113.7".into(),
            region: "us-east-1".into(),
            last_seen_at: String::new(),
            stale: false,
        });
        let view = OutpostStore::current(now);
        assert!(view.sessions.is_empty());
        let status = view.status.expect("status card present");
        // No heartbeat ever → stale (we have never seen a per-session beat).
        assert!(status.stale, "a box with status but no heartbeat reads stale");
        assert!(status.last_seen_at.is_empty());
        OutpostStore::reset();
    }

    // ── presign shape (regression: token appended AFTER signing) ────────────

    #[test]
    fn signed_wss_url_appends_session_token_after_signature() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_780_012_800);
        let url = build_signed_wss_url(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "FAKE/SESSION+TOKEN=",
            "abc123.iot.us-east-1.amazonaws.com",
            "us-east-1",
            now,
        )
        .expect("presign succeeds");
        assert!(url.starts_with("wss://abc123.iot.us-east-1.amazonaws.com/mqtt?"));
        let sig_at = url.find("X-Amz-Signature").unwrap();
        let tok_at = url.find("X-Amz-Security-Token").unwrap();
        assert!(tok_at > sig_at, "token must be appended after the signature: {url}");
        assert!(url.contains("FAKE%2FSESSION%2BTOKEN%3D"), "token must be AWS-encoded: {url}");
    }

    #[test]
    fn realtime_creds_response_deserializes() {
        let json = r#"{
            "credentials": {
                "accessKeyId": "ASIA_FAKE",
                "secretAccessKey": "secret",
                "sessionToken": "token",
                "expiration": "2026-05-29T01:00:00Z"
            },
            "iotEndpoint": "abc123.iot.us-east-1.amazonaws.com",
            "region": "us-east-1",
            "topic": "hq/prs_abc/dm"
        }"#;
        let parsed: RealtimeCredsResponse = serde_json::from_str(json).expect("parses");
        assert_eq!(parsed.credentials.access_key_id, "ASIA_FAKE");
        assert_eq!(sessions_topic_for(&parsed.topic), "hq/prs_abc/sessions");
    }

    #[test]
    fn raw_outpost_status_tolerates_extra_fields() {
        // The real StatusResponse carries many more fields; we must ignore them.
        let json = r#"{
            "userId": "u", "instanceName": "i", "staticIp": "203.0.113.7",
            "region": "us-east-1", "state": "ready", "agentRuntime": "codex",
            "rcPort": 7799, "instanceState": "running", "codexState": "connected",
            "codexRelayStatus": "connected", "platform": "ec2", "extra": {"nested": 1}
        }"#;
        let raw: RawOutpostStatus = serde_json::from_str(json).expect("parses despite extras");
        let card = project_status(&raw);
        assert!(card.up);
        assert_eq!(card.runtime, "codex");
        assert!(card.relay_connected);
    }
}
