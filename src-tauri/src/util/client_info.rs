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

pub const CLIENT_NAME: &str = "hq-sync";
pub const CLIENT_VERSION: &str = env!("APP_VERSION");

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
pub fn build_client() -> Client {
    Client::builder()
        .default_headers(client_headers())
        .build()
        .unwrap_or_else(|_| Client::new())
}
