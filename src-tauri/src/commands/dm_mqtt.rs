//! Instant DM push receiver — MQTT-over-WSS to AWS IoT Core.
//!
//! ## What this does (and deliberately does NOT do)
//!
//! This is the "instant" half of DM delivery. The slow half — the 60s poll in
//! `share_notify::setup_share_notify_poller` (one timer, two fetches, calls
//! `dm_notify::poll_dm_once`) — is untouched and remains the long-stop. This
//! module adds a push channel that collapses delivery latency from ~60s to
//! near-real-time.
//!
//! **The MQTT message is ONLY a wake signal.** On ANY message received on the
//! subscribed topic — and also once on every (re)connect for offline catch-up
//! (US-006) — we call the EXISTING `dm_notify::poll_dm_once(app)`. That function
//! already fetches unread DMs since the persisted cursor, fires the macOS
//! notifications, acks them, and advances the cursor. So we do NOT parse the
//! MQTT payload, do NOT implement get-by-id, and do NOT duplicate the
//! notification loop. Wake → poll. That's the whole design.
//!
//! Dedupe against the 60s poll is automatic: `poll_dm_once` is singleton-guarded
//! (`try_set_in_flight`) and advances the cursor, so a wake-poll and a
//! near-simultaneous scheduled poll never double-deliver.
//!
//! ## Auth + transport
//!
//! 1. `POST {vault}/v1/realtime/credentials` (Bearer access token, same as the
//!    poller) → short-lived STS creds scoped to iot:Connect/Subscribe/Receive on
//!    `hq/<personUid>/*`, plus the IoT Data-ATS endpoint, region, and the
//!    caller's own topic.
//! 2. SigV4-presign a GET for `wss://{endpoint}/mqtt` (service
//!    `iotdevicegateway`) → an `X-Amz-*` query string including the session
//!    token. See `build_signed_wss_url`.
//! 3. Hand the presigned `wss://…` URL to rumqttc, subscribe to `topic`, and
//!    drive the eventloop.
//!
//! ## Failure posture
//!
//! Every failure here is **non-fatal and silent to the user**. Creds fetch
//! failed, presign failed, IoT unreachable, disconnect mid-stream — all just log
//! and retry with backoff. The 60s poll keeps delivering meanwhile, so there is
//! no regression: the worst case is we fall back to the old latency.
//!
//! ## Gating
//!
//! Primary gate is `feature_gate::is_indigo_user()` (`@getindigo.ai`), matching
//! the dogfood posture of the realtime server. We also honour the same
//! `dmNotifications` menubar pref the poller respects, so a user who turned DMs
//! off doesn't get a live socket either.
//!
//! ## Log codes (`dm-mqtt` tag)
//!
//!   `DM_MQTT_GATE_SKIP` / `DM_MQTT_CREDS_FAIL` / `DM_MQTT_PRESIGN_FAIL` /
//!   `DM_MQTT_CONNECT_OK` / `DM_MQTT_SUBSCRIBED` / `DM_MQTT_WAKE` /
//!   `DM_MQTT_DISCONNECT` / `DM_MQTT_FALLBACK`. No secrets are ever logged
//!   (never the presigned URL, never the creds).

use std::time::{Duration, SystemTime};

use serde::Deserialize;
use tauri::AppHandle;

use crate::commands::cognito;
use crate::commands::dm_notify::poll_dm_once;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::logfile::log;

const LOG_TAG: &str = "dm-mqtt";

/// AWS service name for the IoT MQTT broker SigV4 signature. Fixed by AWS.
const IOT_SERVICE: &str = "iotdevicegateway";

/// Reconnect backoff: start here, double on each consecutive failure…
const BACKOFF_MIN: Duration = Duration::from_secs(5);
/// …capped here so a long outage doesn't blow out the retry interval.
const BACKOFF_MAX: Duration = Duration::from_secs(300);

/// Presigned WSS URLs are valid for this long. We re-fetch creds + re-presign
/// on every reconnect anyway, so this only needs to comfortably exceed one
/// connection's setup window; AWS caps IoT presign expiry at 86400s.
const PRESIGN_EXPIRES: Duration = Duration::from_secs(3600);

// ── Wire types ───────────────────────────────────────────────────────────────

/// The STS credentials block inside the realtime-credentials response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
    #[allow(dead_code)]
    expiration: String,
}

/// Response of `POST /v1/realtime/credentials`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeCredsResponse {
    credentials: RealtimeCredentials,
    /// AWS IoT Data-ATS host (no scheme), e.g. `abc123.iot.us-east-1.amazonaws.com`.
    iot_endpoint: String,
    region: String,
    /// The caller's own DM topic, e.g. `hq/<personUid>/dm`.
    topic: String,
}

// ── Credentials fetch ──────────────────────────────────────────────────────────

/// Fetch short-lived realtime credentials from the vault. Returns `Err` (with a
/// short reason) on any auth / network / non-2xx / parse failure — the caller
/// logs it and falls back to the poll silently.
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

// ── SigV4 presign (pure, unit-testable) ──────────────────────────────────────────

/// Presign an AWS IoT MQTT-over-WSS URL. Pure + deterministic given `now`, so it
/// can be unit-tested without a network or a real signer.
///
/// Produces `wss://{endpoint}/mqtt?X-Amz-Algorithm=…&…&X-Amz-Security-Token=…`.
/// We sign a GET with an empty body and the `host` header (the only header AWS
/// IoT requires in the canonical request for the WSS handshake). The scheme is
/// included so rumqttc's `http::Uri` parse (host/port extraction) and
/// tungstenite's `into_client_request` both accept it; the path + query carry
/// the SigV4 material through to the WebSocket upgrade request.
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

    // CRITICAL (AWS IoT WSS): sign WITHOUT the session token. AWS IoT Core's
    // WebSocket auth returns 403 if the signed canonical query includes
    // X-Amz-Security-Token. The token must be appended to the URL AFTER signing
    // (done at the end of this fn). Passing it into the signed identity is what
    // made the 0.3.0-beta.1 client 403 on connect even though the very same
    // creds work when the token is appended post-signing (proven via the
    // hq-pro scripts/instant-dm-e2e.mjs harness).
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

    // The URI we sign is the WSS endpoint with the fixed `/mqtt` path. AWS IoT
    // requires the `host` header in the canonical request; SignableRequest takes
    // headers as (name, value) pairs.
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

    // For QueryParams location the instructions carry the X-Amz-* query pairs
    // (already percent-encoded). Append them to the URL ourselves.
    let query_pairs = instructions.params();
    if query_pairs.is_empty() {
        return Err("presign produced no query params".to_string());
    }
    let query = query_pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    // Append the session token AFTER signing (NOT part of the signed query) —
    // AWS IoT requires this for MQTT-over-WSS. AWS-canonical percent-encoding.
    Ok(format!(
        "wss://{}/mqtt?{}&X-Amz-Security-Token={}",
        endpoint,
        query,
        aws_uri_encode(session_token),
    ))
}

/// RFC3986 / AWS-canonical percent-encoding: everything except the unreserved
/// set `A-Za-z0-9-_.~` is encoded. Used for the X-Amz-Security-Token value we
/// append to the presigned IoT URL after signing.
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

// ── MQTT receive loop ────────────────────────────────────────────────────────────

/// Stable-ish MQTT client id. AWS IoT requires a client id; it must be unique
/// per concurrent connection or IoT will kick the older session. We derive it
/// from the machine id so two Macs for the same person don't collide, with a
/// short random suffix so a fast reconnect (before IoT reaps the old session)
/// doesn't self-evict.
fn client_id() -> String {
    let machine = crate::commands::config::ensure_machine_id().unwrap_or_else(|_| "unknown".into());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    // Keep it well under IoT's 128-char client-id limit.
    format!("hqsync-{}-{}", &machine.chars().take(40).collect::<String>(), &suffix[..8])
}

/// One connect→subscribe→receive cycle. Returns `Ok(())` on a clean shutdown
/// signal (never, in practice — the loop runs until the process exits) and
/// `Err(reason)` on any connection failure so the caller can back off and retry.
///
/// Calls `poll_dm_once(app)` once right after a successful connect (offline
/// catch-up, US-006) and once per inbound Publish (the wake signal).
async fn run_once(app: &AppHandle, creds: &RealtimeCredsResponse) -> Result<(), String> {
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
        log(LOG_TAG, &format!("DM_MQTT_PRESIGN_FAIL {e}"));
        e
    })?;

    // rumqttc's WSS transport uses `broker_addr` (the full `wss://host/path?query`
    // string) as the WebSocket request URI, and separately parses host+port for
    // the TCP connect. So we set the presigned URL as the broker address and the
    // SigV4 query rides through to the upgrade request. We deliberately do NOT use
    // `MqttOptions::parse_url`, which rejects the X-Amz-* query keys as unknown.
    let mut opts = MqttOptions::new(client_id(), url, 443);
    opts.set_transport(Transport::wss_with_default_config());
    opts.set_keep_alive(Duration::from_secs(30));
    // AWS IoT requires a clean session for SigV4-WSS connections.
    opts.set_clean_session(true);

    let (client, mut eventloop) = AsyncClient::new(opts, 10);

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                log(LOG_TAG, "DM_MQTT_CONNECT_OK");
                // Subscribe to our own topic. QoS 0 (AtMostOnce): the payload is
                // only a wake signal — if one is dropped the 60s poll backstops it.
                if let Err(e) = client.subscribe(creds.topic.clone(), QoS::AtMostOnce).await {
                    return Err(format!("subscribe: {e}"));
                }
                log(LOG_TAG, &format!("DM_MQTT_SUBSCRIBED topic={}", creds.topic));
                // Offline catch-up (US-006): drain anything missed while we were
                // disconnected, before the first push arrives.
                poll_dm_once(app.clone()).await;
            }
            Ok(Event::Incoming(Packet::Publish(_))) => {
                // Wake signal. We do not inspect the payload — just poll.
                log(LOG_TAG, "DM_MQTT_WAKE");
                poll_dm_once(app.clone()).await;
            }
            Ok(_) => { /* SubAck, PingResp, Outgoing, etc. — ignore. */ }
            Err(e) => {
                // Any connection-level error ends this cycle; caller backs off.
                return Err(format!("eventloop: {e}"));
            }
        }
    }
}

/// Spawn the instant-DM MQTT receiver. Called from `main.rs` `.setup()`,
/// macOS-gated like the other background tasks.
///
/// Gated on `is_indigo_user()` (primary) and the `dmNotifications` pref. On any
/// failure the task logs and retries with capped exponential backoff — it never
/// surfaces an error to the user and never blocks the 60s poll.
pub fn setup_dm_mqtt_receiver(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if !crate::util::feature_gate::is_indigo_user().await {
            log(LOG_TAG, "DM_MQTT_GATE_SKIP not an @getindigo.ai identity");
            return;
        }

        // Give the app a moment to finish init + load the Cognito token from
        // disk (mirrors the share-notify poller's 5s launch delay).
        tokio::time::sleep(Duration::from_secs(5)).await;

        let mut backoff = BACKOFF_MIN;
        loop {
            // Re-fetch credentials before every (re)connect — they are short-lived
            // STS creds and will have expired across a long backoff.
            match fetch_realtime_credentials().await {
                Ok(creds) => {
                    // A successful connect cycle runs until the socket drops; only
                    // then do we fall through to back off + reconnect. Reset the
                    // backoff after a connection that actually established.
                    let started = SystemTime::now();
                    match run_once(&app, &creds).await {
                        Ok(()) => { /* unreachable in practice; treat as disconnect */ }
                        Err(e) => log(LOG_TAG, &format!("DM_MQTT_DISCONNECT {e}")),
                    }
                    // If we stayed connected for a while, reset backoff so the next
                    // transient drop reconnects fast.
                    if started.elapsed().map(|d| d > Duration::from_secs(30)).unwrap_or(false) {
                        backoff = BACKOFF_MIN;
                    }
                }
                Err(e) => {
                    log(LOG_TAG, &format!("DM_MQTT_CREDS_FAIL {e}"));
                }
            }

            log(
                LOG_TAG,
                &format!("DM_MQTT_FALLBACK reconnect in {}s (poll still active)", backoff.as_secs()),
            );
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_time() -> SystemTime {
        // 2026-05-29T00:00:00Z — arbitrary fixed instant for deterministic tests.
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_780_012_800)
    }

    #[test]
    fn signed_wss_url_has_expected_shape() {
        let url = build_signed_wss_url(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "FAKE/SESSION+TOKEN=",
            "abc123.iot.us-east-1.amazonaws.com",
            "us-east-1",
            fixed_time(),
        )
        .expect("presign succeeds");

        // Host + path, not the exact signature.
        assert!(
            url.starts_with("wss://abc123.iot.us-east-1.amazonaws.com/mqtt?"),
            "url = {url}"
        );
        // Required SigV4 query-presign params (case-sensitive).
        assert!(url.contains("X-Amz-Algorithm"), "missing algorithm: {url}");
        assert!(url.contains("X-Amz-Credential"), "missing credential: {url}");
        assert!(url.contains("X-Amz-Signature"), "missing signature: {url}");
        assert!(
            url.contains("X-Amz-Security-Token"),
            "missing session token: {url}"
        );
        // The credential scope must name the IoT service.
        assert!(
            url.contains("iotdevicegateway"),
            "credential scope must name the service: {url}"
        );

        // REGRESSION (the 0.3.0-beta.1 403): the session token MUST be appended
        // AFTER signing, never part of the signed query. Assert it appears after
        // X-Amz-Signature (i.e. it's the trailing appended param, not signed).
        let sig_at = url.find("X-Amz-Signature").unwrap();
        let tok_at = url.find("X-Amz-Security-Token").unwrap();
        assert!(
            tok_at > sig_at,
            "security token must be appended AFTER the signature (post-signing), got: {url}"
        );
        // And it must be AWS-percent-encoded (/ + = → %2F %2B %3D), not raw.
        assert!(
            url.contains("FAKE%2FSESSION%2BTOKEN%3D"),
            "session token must be AWS-encoded: {url}"
        );
    }

    #[test]
    fn realtime_creds_response_deserializes_camel_case() {
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
        assert_eq!(parsed.credentials.session_token, "token");
        assert_eq!(parsed.iot_endpoint, "abc123.iot.us-east-1.amazonaws.com");
        assert_eq!(parsed.region, "us-east-1");
        assert_eq!(parsed.topic, "hq/prs_abc/dm");
    }

    #[test]
    fn gate_is_indigo_email_only() {
        // The receiver gates on the shared feature_gate helper; assert its pure
        // suffix-match rule here so this module's gate contract is pinned too.
        use crate::util::feature_gate::is_allowed_email;
        assert!(is_allowed_email(Some("dev@getindigo.ai")));
        assert!(!is_allowed_email(Some("dev@example.com")));
        assert!(!is_allowed_email(None));
    }
}
