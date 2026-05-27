//! Hard version-gate against the hq-pro `/v1/client-version/check` endpoint.
//!
//! Sibling to `hq_cli_update.rs` and `updater.rs`, but distinct:
//!
//!   - `updater.rs`           — Tauri auto-updater nag against the GitHub
//!                              `latest.json`. Soft + user-initiated.
//!   - `hq_cli_update.rs`     — npm-registry nag for the *separate* hq-cli
//!                              binary the user has on PATH.
//!   - `version_gate.rs` (us) — authoritative hq-pro check that can hard-yank
//!                              a known-bad hq-sync release without waiting
//!                              for the npm/GitHub `latest` channels to move.
//!                              On `updateRequired:true` we reuse the Tauri
//!                              updater's `download_and_install` directly,
//!                              which then restarts the app.
//!
//! Trust model: anonymous. The menubar may be running pre-sign-in (cold
//! launch, expired tokens) so we never send credentials. Endpoint identifies
//! us by `clientId="hq-sync"` + `currentVersion=APP_VERSION`.
//!
//! Failure mode: silent. Network down, hq-pro 5xx, malformed body — log via
//! `util::logfile::log` and return. The gate must never break the app for a
//! user who's otherwise fine.
//!
//! Background loop cadence:
//!   - First check 5s after launch — runs BEFORE the existing 10s soft
//!     updater check so we can hard-gate the install before the user touches
//!     anything sensitive.
//!   - Repeat every 6h (matches `updater::setup_update_checker`).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::util::client_info::{client_headers, CLIENT_VERSION};
use crate::util::logfile::log;

/// Client identifier we present to hq-pro. Must match an entry in the server's
/// `CLIENT_VERSIONS` table.
const CLIENT_ID: &str = "hq-sync";

/// Endpoint path appended to whatever `resolve_vault_api_url` returns.
const ENDPOINT_PATH: &str = "/v1/client-version/check";

/// HTTP timeout. Tight on purpose — a hung server can't be allowed to stall
/// the background loop or the app launch path.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// First check fires this long after launch. Smaller than the soft updater
/// (10s) so the hard gate is the *first* update prompt the user can see.
const INITIAL_DELAY: Duration = Duration::from_secs(5);

/// Re-check cadence. Matches `updater::setup_update_checker`.
const CHECK_INTERVAL: Duration = Duration::from_secs(21_600);

/// Wire shape returned by the hq-pro `/v1/client-version/check` endpoint.
/// Field names mirror `apps/hq-pro/src/vault-service/handlers/client-version-check.ts`
/// exactly — drift here = silent gate failure.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionCheckResponse {
    pub client_id: String,
    pub current_version: String,
    pub min_version: String,
    pub latest_version: String,
    pub update_required: bool,
    pub update_recommended: bool,
    #[serde(default)]
    pub update_command: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionGateEvent {
    pub current_version: String,
    pub latest_version: String,
    pub min_version: String,
    pub message: Option<String>,
}

/// Resolve the vault API URL the same way `commands::sync` does. Wrapped here
/// so the version gate doesn't depend on `sync` (which would create a cycle
/// at the lib-graph level).
fn vault_api_url() -> Result<String, String> {
    crate::commands::sync::resolve_vault_api_url()
}

/// Platform tag passed to hq-pro. Format mirrors what hq-cli sends so the
/// server can route a single `platform`-aware `downloadUrl` resolver across
/// both clients.
fn platform_tag() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// One-shot: POST to the gate endpoint and parse the response. Returns
/// `Ok(None)` on any non-2xx or malformed body (treat as "no decision") so
/// callers can match a single `Ok(Some(decision))` arm for the gate-fires
/// path.
///
/// The `base_url` argument is the resolved API base (e.g.
/// `https://hqapi.getindigo.ai`). Factored as an argument so the unit test
/// can point at a `wiremock::MockServer::uri()` without re-implementing the
/// vault URL precedence.
pub(crate) async fn fetch_decision(
    base_url: &str,
    current_version: &str,
) -> Result<Option<VersionCheckResponse>, String> {
    let client = reqwest::Client::builder()
        .default_headers(client_headers())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("build client: {e}"))?;

    let body = serde_json::json!({
        "clientId": CLIENT_ID,
        "currentVersion": current_version,
        "platform": platform_tag(),
    });

    let url = format!("{}{}", base_url.trim_end_matches('/'), ENDPOINT_PATH);
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        // 4xx — server rejected our request shape. Log + treat as no-gate so
        // a server-side regression doesn't brick all menubar instances.
        log(
            "version-gate",
            &format!("check returned HTTP {status} from {url}"),
        );
        return Ok(None);
    }

    let parsed = resp
        .json::<VersionCheckResponse>()
        .await
        .map_err(|e| format!("parse JSON: {e}"))?;
    Ok(Some(parsed))
}

/// Force-install the latest hq-sync via the Tauri updater. Mirrors
/// `updater::install_update` — calls `updater.check().await` again because
/// the `Update` value isn't `Clone`, then `download_and_install`, then
/// `app.restart()`. On macOS the process typically terminates inside
/// `download_and_install`; the explicit restart() is a cross-platform
/// safety net.
async fn force_install(app: &AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|e| e.to_string())?;
            app.restart();
        }
        Ok(None) => {
            // hq-pro says we're below min, but the Tauri updater can't see
            // any newer release. This means the GitHub `latest.json` hasn't
            // been published yet — surface it loudly so we can debug.
            return Err(
                "hq-pro hard-gate fired but tauri-updater sees no release; \
                 latest.json may be stale"
                    .to_string(),
            );
        }
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// React to a single decision. Side-effects:
///   - `update_required` -> emit `version-gate:update-required` (frontend
///      shows blocking modal), then attempt force-install. On force-install
///      error, log + leave the modal up so the user has manual escape hatch.
///   - `update_recommended` -> emit `version-gate:update-recommended` (banner).
///   - neither -> emit `version-gate:current` so the frontend can clear any
///      stale banner.
async fn react_to_decision(app: &AppHandle, decision: &VersionCheckResponse) {
    let event = VersionGateEvent {
        current_version: decision.current_version.clone(),
        latest_version: decision.latest_version.clone(),
        min_version: decision.min_version.clone(),
        message: decision.message.clone(),
    };

    if decision.update_required {
        let _ = app.emit("version-gate:update-required", &event);
        log(
            "version-gate",
            &format!(
                "REQUIRED: current={} min={} latest={} — forcing install",
                decision.current_version, decision.min_version, decision.latest_version
            ),
        );
        if let Err(e) = force_install(app).await {
            log("version-gate", &format!("force_install failed: {e}"));
        }
        return;
    }

    if decision.update_recommended {
        let _ = app.emit("version-gate:update-recommended", &event);
        log(
            "version-gate",
            &format!(
                "RECOMMENDED: current={} latest={}",
                decision.current_version, decision.latest_version
            ),
        );
        return;
    }

    let _ = app.emit("version-gate:current", &event);
}

/// Public: do one check + react. Used by `setup_version_gate` and (in
/// future) a Tauri command for an on-demand "Check Now" menu item.
pub async fn check_once(app: &AppHandle) -> Result<(), String> {
    let base_url = vault_api_url()?;
    match fetch_decision(&base_url, CLIENT_VERSION).await? {
        Some(decision) => {
            react_to_decision(app, &decision).await;
            Ok(())
        }
        None => Ok(()),
    }
}

/// Spawn the background loop. First check fires 5s after launch (before the
/// soft updater's 10s), then every 6h. Errors are logged but never propagate
/// — a flaky network must not break the loop.
pub fn setup_version_gate(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            if let Err(e) = check_once(&handle).await {
                log("version-gate", &format!("background check failed: {e}"));
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetch_decision_parses_update_required_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/client-version/check"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "clientId": "hq-sync",
                "currentVersion": "0.1.50",
                "minVersion": "0.1.100",
                "latestVersion": "0.1.110",
                "updateRequired": true,
                "updateRecommended": false,
                "downloadUrl": "https://example.com/sync/latest",
                "message": "Security fix"
            })))
            .mount(&server)
            .await;

        let result = fetch_decision(&server.uri(), "0.1.50").await.unwrap();
        let decision = result.expect("decision should be Some");
        assert!(decision.update_required);
        assert!(!decision.update_recommended);
        assert_eq!(decision.min_version, "0.1.100");
        assert_eq!(decision.latest_version, "0.1.110");
        assert_eq!(decision.download_url.as_deref(), Some("https://example.com/sync/latest"));
    }

    #[tokio::test]
    async fn fetch_decision_parses_update_recommended_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/client-version/check"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "clientId": "hq-sync",
                "currentVersion": "0.1.105",
                "minVersion": "0.1.100",
                "latestVersion": "0.1.110",
                "updateRequired": false,
                "updateRecommended": true
            })))
            .mount(&server)
            .await;

        let result = fetch_decision(&server.uri(), "0.1.105").await.unwrap();
        let decision = result.unwrap();
        assert!(!decision.update_required);
        assert!(decision.update_recommended);
    }

    #[tokio::test]
    async fn fetch_decision_returns_none_on_500() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/client-version/check"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let result = fetch_decision(&server.uri(), "0.1.107").await.unwrap();
        assert!(result.is_none(), "5xx should map to Ok(None), not Err");
    }

    #[tokio::test]
    async fn fetch_decision_returns_none_on_404() {
        // 404 mimics "unknown clientId" from the server — treat as no-gate so
        // we can't brick clients via a typo on the server table.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/client-version/check"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let result = fetch_decision(&server.uri(), "0.1.107").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn fetch_decision_errors_on_malformed_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/client-version/check"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("{not-json")
                    .insert_header("content-type", "application/json"),
            )
            .mount(&server)
            .await;

        let result = fetch_decision(&server.uri(), "0.1.107").await;
        assert!(result.is_err(), "malformed JSON should bubble up as Err");
    }

    #[tokio::test]
    async fn fetch_decision_sends_expected_body_shape() {
        // Lock the wire shape so a field rename here can't silently break the
        // server's input validation (which would 400, which we then map to None
        // and silently never gate anything).
        use wiremock::matchers::body_partial_json;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/client-version/check"))
            .and(body_partial_json(json!({
                "clientId": "hq-sync",
                "currentVersion": "0.1.107",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "clientId": "hq-sync",
                "currentVersion": "0.1.107",
                "minVersion": "0.1.0",
                "latestVersion": "0.1.107",
                "updateRequired": false,
                "updateRecommended": false
            })))
            .mount(&server)
            .await;

        let result = fetch_decision(&server.uri(), "0.1.107").await.unwrap();
        assert!(result.is_some(), "matcher should have accepted the body");
    }
}
