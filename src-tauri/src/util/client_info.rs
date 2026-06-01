//! Shared HTTP-client construction so every outbound `reqwest::Client` in the
//! menubar carries the same client-attribution headers. Used by `vault_client`
//! (talks to OUR vault), `cognito` and `oauth` (AWS Cognito — third party but
//! still useful for User-Agent attribution), and `hq_cli_update` (GitHub
//! releases — GitHub *requires* a User-Agent for anonymous API access).
//!
//! `CLIENT_VERSION` comes from the shipped npm/tauri.conf.json version (emitted
//! by `build.rs` as `APP_VERSION`), NOT `env!("CARGO_PKG_VERSION")`. The two
//! version numbers are deliberately decoupled — the Rust crate version is
//! internal bookkeeping, and the npm package.json version is what users see
//! in About dialogs and DMG names. Stamping the wrong one defeats the
//! attribution/rollout telemetry this helper is here to provide.

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use std::time::Duration;

pub const CLIENT_NAME: &str = "hq-sync";
pub const CLIENT_VERSION: &str = env!("APP_VERSION");

/// Total per-request budget. Long enough to cover the slowest legitimate
/// vault endpoint (`/v1/calendar/events` fans out across calendars) but
/// short enough that a hung upstream surfaces as a clean reqwest error.
/// 2026-04 regression: hq-pro's KMS-IAM bug 500'd `/v1/calendar/events` and
/// `/v1/google/accounts` for hours; without a timeout the MeetingsWindow
/// refresh sat on a pending future and never re-engaged the 30s poll.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Connect budget — fires before the full request timeout so a misrouted
/// host (DNS pointing nowhere, network partition) errors quickly instead of
/// burning the whole 15s on a TCP handshake that's never going to complete.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Build a HeaderMap with our standard client-attribution headers.
///
/// All three header values are ASCII compile-time constants, so the
/// `HeaderValue::from_str` calls cannot fail in practice. We guard with `if
/// let Ok(...)` defensively — silently dropping a header is safer than
/// panicking inside a Tauri command handler.
pub fn client_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    let user_agent = format!("{}/{}", CLIENT_NAME, CLIENT_VERSION);
    if let Ok(v) = HeaderValue::from_str(&user_agent) {
        headers.insert(reqwest::header::USER_AGENT, v);
    }
    if let Ok(v) = HeaderValue::from_str(CLIENT_NAME) {
        headers.insert("x-hq-client-name", v);
    }
    if let Ok(v) = HeaderValue::from_str(CLIENT_VERSION) {
        headers.insert("x-hq-client-version", v);
    }
    headers
}

/// Build a reqwest Client preconfigured with the standard client headers as
/// defaults. Callers can layer more headers per request — `default_headers`
/// is merged with per-call headers by reqwest.
///
/// Request + connect timeouts apply to *every* caller of this helper. Sync
/// streaming (`commands/sync.rs`) is a separate subprocess pipeline and is
/// not affected. The ontology-participants helper in `commands/meetings.rs`
/// wraps its call in `tokio::time::timeout(2s)` for a tighter budget; that
/// wrapper becomes redundant once the client itself has a default, but it
/// stays as defense-in-depth for the bot-invite hot path.
pub fn build_client() -> Client {
    Client::builder()
        .default_headers(client_headers())
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Regression test for the 2026-04 hq-pro KMS-IAM 500 outage:
    /// `/v1/calendar/events` returned 500 for hours and the MeetingsWindow
    /// refresh path sat on a pending future because `build_client()` had
    /// no timeout. Lock in a request timeout so a hung upstream surfaces
    /// as a clean reqwest error in well under the wiremock-side delay.
    ///
    /// Slow on purpose (~15s): the test wedges a 30s server-side delay
    /// behind the configured 15s budget. If the timeout regresses to
    /// unset, the test will block past 20s and the elapsed-time assertion
    /// will fail.
    #[tokio::test]
    async fn build_client_times_out_on_slow_endpoint() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
            )
            .mount(&server)
            .await;

        let client = build_client();
        let start = Instant::now();
        let result = client.get(server.uri()).send().await;
        let elapsed = start.elapsed();

        assert!(
            result.is_err(),
            "expected timeout error from build_client(), got {result:?}",
        );
        // 15s configured + 5s slack for slow CI runners. Anything past
        // 20s means the timeout silently dropped from the builder.
        assert!(
            elapsed < Duration::from_secs(20),
            "build_client() did not time out (elapsed {elapsed:?}) — \
             a timeout regression has shipped",
        );
    }
}
