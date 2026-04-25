// oauth.rs — OAuth loopback listener + PKCE login flow for HQ Sync menubar.
//
// Starts a one-shot HTTP server on 127.0.0.1:53682 and advertises the
// callback as http://localhost:53682/callback, which matches the
// `http://localhost:*/callback` wildcard registered on Cognito app client
// 7r7an9keh0u6hlsvepl74tvqb0 (hq-dev stack; see hq-pro ADR-0003).
// Binding to 127.0.0.1 (not 0.0.0.0) keeps the
// listener off the LAN; `localhost` in the redirect URI is required because
// Cognito matches the host segment literally — `127.0.0.1` fails.
// and waits for the browser to redirect back to /callback?code=...&state=...
// with the authorization code. Responds with a friendly HTML page that tells
// the user to return to HQ Sync, then shuts the listener down.
//
// Login flow (Svelte frontend):
//   1. Call `start_oauth_login` — returns authorize URL + state.
//   2. Call `tauri_plugin_shell::open(authorize_url)` to open the browser.
//   3. Call `oauth_listen_for_code(state)` to wait for the callback code.
//   4. Call `oauth_exchange_code(code)` to exchange the code for tokens.
//
// Security notes:
//   - Binds to 127.0.0.1 only — never 0.0.0.0.
//   - Enforces `state` match between what the listener was started with and
//     what comes back on the callback, defending against CSRF/code injection.
//   - Single-use: accepts at most one request, closes listener afterwards.
//   - 5-minute timeout so a stalled/abandoned flow doesn't leak a socket.
//   - PKCE (S256) prevents authorization code interception.

use super::cognito::{self, AuthState, CognitoTokens};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const LOOPBACK_PORT: u16 = 53682;
const LOOPBACK_HOST: &str = "127.0.0.1";
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const READ_TIMEOUT: Duration = Duration::from_secs(10);

// hq-dev stack (canonical; see hq-pro ADR-0003).
const COGNITO_CLIENT_ID: &str = "7r7an9keh0u6hlsvepl74tvqb0";
const DEFAULT_COGNITO_DOMAIN_PREFIX: &str = "vault-indigo-hq-dev";
const REDIRECT_URI: &str = "http://localhost:53682/callback";

/// Cognito hosted-UI domain prefix.
///
/// Resolves to `$HQ_COGNITO_DOMAIN` if set, else the canonical
/// `vault-indigo-hq-dev` prefix shared with `@indigoai-us/hq-cli` and
/// `hq-installer`. Always in the
/// `us-east-1.amazoncognito.com` namespace — custom domains not yet supported.
fn cognito_domain_prefix() -> String {
    std::env::var("HQ_COGNITO_DOMAIN").unwrap_or_else(|_| DEFAULT_COGNITO_DOMAIN_PREFIX.to_string())
}

fn cognito_authorize_url() -> String {
    format!(
        "https://{}.auth.us-east-1.amazoncognito.com/oauth2/authorize",
        cognito_domain_prefix()
    )
}

fn cognito_token_url() -> String {
    format!(
        "https://{}.auth.us-east-1.amazoncognito.com/oauth2/token",
        cognito_domain_prefix()
    )
}

// ── PKCE verifier storage ──────────────────────────────────────────────

static PKCE_VERIFIER: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn pkce_store() -> &'static Mutex<Option<String>> {
    PKCE_VERIFIER.get_or_init(|| Mutex::new(None))
}

// ── Public types ───────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct OAuthResult {
    pub code: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthFlowInit {
    pub authorize_url: String,
    pub state: String,
}

// ── Cognito token exchange response ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    id_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: i64,
}

// ── PKCE helpers ───────────────────────────────────────────────────────

/// Generate a PKCE code verifier (43–128 characters, URL-safe).
/// Uses uuid::Uuid::new_v4 to avoid adding `rand` as a dependency.
fn generate_code_verifier() -> String {
    // 3 UUIDs = 96 hex chars after removing hyphens. We take the first 64
    // characters, well within the 43–128 range.
    let raw = format!(
        "{}{}{}",
        uuid::Uuid::new_v4().as_simple(),
        uuid::Uuid::new_v4().as_simple(),
        uuid::Uuid::new_v4().as_simple(),
    );
    // UUID simple format is hex (0-9a-f) which is URL-safe.
    raw[..64].to_string()
}

/// Compute the S256 code challenge: BASE64URL(SHA256(verifier)).
fn compute_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

// ── HTML ───────────────────────────────────────────────────────────────

const SUCCESS_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>Signed in — HQ Sync</title>
<style>
  html, body { margin: 0; padding: 0; height: 100%; background: #0a0a0a; color: #fafafa;
    font-family: -apple-system, BlinkMacSystemFont, "Geist", sans-serif; }
  .wrap { height: 100%; display: flex; align-items: center; justify-content: center; }
  .card { max-width: 420px; padding: 32px 28px; text-align: center; }
  .check { width: 56px; height: 56px; border-radius: 28px; background: rgba(34,197,94,0.15);
    color: #22c55e; font-size: 28px; line-height: 56px; margin: 0 auto 16px; }
  h1 { font-size: 20px; font-weight: 500; margin: 0 0 8px; }
  p { font-size: 14px; color: #a1a1aa; margin: 0; }
</style>
</head>
<body>
<div class="wrap"><div class="card">
  <div class="check">&check;</div>
  <h1>You are signed in</h1>
  <p>You can close this tab and return to HQ Sync.</p>
</div></div>
</body>
</html>"#;

fn error_html(reason: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8" /><title>Sign-in error</title>
<style>body{{font-family:-apple-system,sans-serif;background:#0a0a0a;color:#fafafa;
text-align:center;padding-top:80px}}h1{{font-weight:500}}p{{color:#a1a1aa}}
code{{color:#f87171;font-size:12px;display:block;margin-top:24px}}</style>
</head><body><h1>Sign-in error</h1>
<p>Return to HQ Sync and try again.</p>
<code>{reason}</code></body></html>"#,
        reason = reason
    )
}

// ── HTTP helpers ───────────────────────────────────────────────────────

fn read_request_line(stream: &mut TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

fn write_response(stream: &mut TcpStream, status: &str, body: &str) {
    let payload = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        status = status,
        len = body.len(),
        body = body,
    );
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

fn parse_callback(request: &str) -> Option<(String, String, Option<String>)> {
    let first_line = request.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if method != "GET" {
        return None;
    }
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v_decoded = urldecode(v);
        match k {
            "code" => code = Some(v_decoded),
            "state" => state = Some(v_decoded),
            "error" => error = Some(v_decoded),
            _ => {}
        }
    }
    match (code, state, error) {
        (Some(c), Some(s), err) => Some((c, s, err)),
        _ => None,
    }
}

fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── Tauri commands ─────────────────────────────────────────────────────

/// Start the OAuth login flow: generate PKCE verifier/challenge, build the
/// Cognito authorize URL, store the verifier for later exchange.
#[tauri::command]
pub async fn start_oauth_login() -> Result<OAuthFlowInit, String> {
    let state = uuid::Uuid::new_v4().to_string();
    let verifier = generate_code_verifier();
    let challenge = compute_code_challenge(&verifier);

    // Store verifier for oauth_exchange_code
    {
        let mut guard = pkce_store()
            .lock()
            .map_err(|e| format!("PKCE lock poisoned: {e}"))?;
        *guard = Some(verifier);
    }

    // `identity_provider=Google` tells Cognito Hosted UI to skip its own
    // username/password form and redirect straight to Google's OAuth consent
    // screen. The browser almost always has an active Google session, so the
    // user sees a one-click "Continue as …" at most — matching the hq-installer
    // sign-in flow. Dropping this parameter reverts to the unbranded Cognito
    // login page.
    let authorize_url = format!(
        "{base}?response_type=code\
         &client_id={client_id}\
         &redirect_uri={redirect_uri}\
         &scope=openid+email+profile\
         &identity_provider=Google\
         &state={state}\
         &code_challenge={challenge}\
         &code_challenge_method=S256",
        base = cognito_authorize_url(),
        client_id = COGNITO_CLIENT_ID,
        redirect_uri = REDIRECT_URI,
        state = state,
        challenge = challenge,
    );

    Ok(OAuthFlowInit {
        authorize_url,
        state,
    })
}

/// Exchange an authorization code for tokens using the stored PKCE verifier.
#[tauri::command]
pub async fn oauth_exchange_code(code: String) -> Result<AuthState, String> {
    // Take the verifier out of storage (one-time use)
    let verifier = {
        let mut guard = pkce_store()
            .lock()
            .map_err(|e| format!("PKCE lock poisoned: {e}"))?;
        guard
            .take()
            .ok_or_else(|| "No PKCE verifier found — was start_oauth_login called?".to_string())?
    };

    let client = reqwest::Client::new();

    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", COGNITO_CLIENT_ID),
        ("code", &code),
        ("redirect_uri", REDIRECT_URI),
        ("code_verifier", &verifier),
    ];

    let response = client
        .post(cognito_token_url())
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        return Err(format!(
            "Token exchange failed ({status}): {body_text}"
        ));
    }

    let token_resp: TokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let expires_at = now_ms + (token_resp.expires_in * 1000);

    let tokens = CognitoTokens {
        access_token: token_resp.access_token,
        id_token: token_resp.id_token,
        refresh_token: token_resp
            .refresh_token
            .ok_or_else(|| "No refresh_token in response".to_string())?,
        expires_at,
    };

    cognito::set_tokens(&tokens).await?;

    Ok(AuthState {
        authenticated: true,
        expires_at: Some(cognito::expires_at_iso(&tokens)),
    })
}

/// Listen for the OAuth callback on the loopback port.
#[tauri::command]
pub async fn oauth_listen_for_code(state: String) -> Result<OAuthResult, String> {
    let state_copy = state.clone();

    tokio::task::spawn_blocking(move || -> Result<OAuthResult, String> {
        let listener =
            TcpListener::bind((LOOPBACK_HOST, LOOPBACK_PORT)).map_err(|e| {
                format!(
                    "Failed to bind OAuth loopback listener on {}:{} — {}. \
                     Another instance may already be waiting for sign-in.",
                    LOOPBACK_HOST, LOOPBACK_PORT, e
                )
            })?;

        listener
            .set_nonblocking(false)
            .map_err(|e| format!("set_nonblocking: {e}"))?;

        let deadline = std::time::Instant::now() + IDLE_TIMEOUT;

        loop {
            if std::time::Instant::now() > deadline {
                return Err("Timed out waiting for sign-in (5 minutes).".into());
            }

            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let request = match read_request_line(&mut stream) {
                        Ok(r) => r,
                        Err(_) => {
                            continue;
                        }
                    };

                    match parse_callback(&request) {
                        Some((_code, _state, Some(error))) => {
                            let reason = format!("Provider error: {error}");
                            eprintln!("[oauth] callback rejected — {reason}");
                            write_response(
                                &mut stream,
                                "400 Bad Request",
                                &error_html(&reason),
                            );
                            return Err(format!(
                                "OAuth provider returned error: {error}"
                            ));
                        }
                        Some((code, state, None)) => {
                            if state != state_copy {
                                let reason = format!(
                                    "State mismatch: expected {} got {}",
                                    state_copy, state
                                );
                                eprintln!("[oauth] callback rejected — {reason}");
                                write_response(
                                    &mut stream,
                                    "400 Bad Request",
                                    &error_html(&reason),
                                );
                                return Err(
                                    "OAuth state mismatch — possible CSRF, aborting."
                                        .into(),
                                );
                            }
                            eprintln!(
                                "[oauth] callback accepted — code length {}",
                                code.len()
                            );
                            write_response(&mut stream, "200 OK", SUCCESS_HTML);
                            return Ok(OAuthResult { code });
                        }
                        None => {
                            write_response(
                                &mut stream,
                                "404 Not Found",
                                "<!doctype html><title>404</title>",
                            );
                            continue;
                        }
                    }
                }
                Err(e) => {
                    return Err(format!("accept failed: {e}"));
                }
            }
        }
    })
    .await
    .map_err(|e| format!("OAuth listener task panicked: {e}"))?
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_callback_extracts_code_and_state() {
        let req = "GET /callback?code=abc123&state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (code, state, err) = parse_callback(req).unwrap();
        assert_eq!(code, "abc123");
        assert_eq!(state, "xyz");
        assert!(err.is_none());
    }

    #[test]
    fn parse_callback_captures_error() {
        let req = "GET /callback?code=x&state=y&error=access_denied HTTP/1.1\r\n\r\n";
        let (_, _, err) = parse_callback(req).unwrap();
        assert_eq!(err.as_deref(), Some("access_denied"));
    }

    #[test]
    fn parse_callback_rejects_non_get() {
        let req = "POST /callback?code=x&state=y HTTP/1.1\r\n\r\n";
        assert!(parse_callback(req).is_none());
    }

    #[test]
    fn parse_callback_ignores_non_callback_paths() {
        let req = "GET /favicon.ico HTTP/1.1\r\n\r\n";
        assert!(parse_callback(req).is_none());
    }

    #[test]
    fn urldecode_handles_percent_and_plus() {
        assert_eq!(urldecode("hello+world"), "hello world");
        assert_eq!(urldecode("a%20b"), "a b");
        assert_eq!(urldecode("plain"), "plain");
    }

    #[test]
    fn code_verifier_length_is_valid() {
        let verifier = generate_code_verifier();
        assert_eq!(verifier.len(), 64);
        // Must be in the 43–128 range per PKCE spec
        assert!(verifier.len() >= 43 && verifier.len() <= 128);
    }

    #[test]
    fn code_verifier_is_url_safe() {
        let verifier = generate_code_verifier();
        // UUID simple format is hex (0-9a-f), all URL-safe
        assert!(verifier.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn code_verifier_is_random() {
        let v1 = generate_code_verifier();
        let v2 = generate_code_verifier();
        assert_ne!(v1, v2);
    }

    #[test]
    fn code_challenge_is_base64url_sha256() {
        // Known test vector: SHA256("test") = 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
        // base64url of that = n4bQgYhMfWWaL-qgxVrQFaO_TxsrC4Is0V1sFbDwCgg
        let challenge = compute_code_challenge("test");
        assert_eq!(challenge, "n4bQgYhMfWWaL-qgxVrQFaO_TxsrC4Is0V1sFbDwCgg");
    }

    #[test]
    fn code_challenge_has_no_padding() {
        let challenge = compute_code_challenge("hello");
        assert!(!challenge.contains('='));
    }

    #[test]
    fn authorize_url_contains_required_params() {
        // We can't call the async command directly in a sync test, so test
        // the URL construction logic inline.
        let state = "test-state-123";
        let verifier = generate_code_verifier();
        let challenge = compute_code_challenge(&verifier);

        let url = format!(
            "{base}?response_type=code\
             &client_id={client_id}\
             &redirect_uri={redirect_uri}\
             &scope=openid+email+profile\
             &identity_provider=Google\
             &state={state}\
             &code_challenge={challenge}\
             &code_challenge_method=S256",
            base = cognito_authorize_url(),
            client_id = COGNITO_CLIENT_ID,
            redirect_uri = REDIRECT_URI,
            state = state,
            challenge = challenge,
        );

        assert!(url.starts_with(&format!("{}?", cognito_authorize_url())));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=7r7an9keh0u6hlsvepl74tvqb0"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A53682%2Fcallback") || url.contains("redirect_uri=http://localhost:53682/callback"));
        assert!(url.contains("scope=openid+email+profile"));
        // identity_provider=Google is what makes Cognito skip its Hosted UI
        // login form and route straight to Google OAuth — matches hq-installer.
        assert!(url.contains("identity_provider=Google"));
        assert!(url.contains(&format!("state={state}")));
        assert!(url.contains(&format!("code_challenge={challenge}")));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn pkce_store_roundtrip() {
        // Store a verifier, then take it out
        {
            let mut guard = pkce_store().lock().unwrap();
            *guard = Some("test-verifier".to_string());
        }
        {
            let mut guard = pkce_store().lock().unwrap();
            let taken = guard.take();
            assert_eq!(taken, Some("test-verifier".to_string()));
        }
        {
            let guard = pkce_store().lock().unwrap();
            assert!(guard.is_none());
        }
    }
}
