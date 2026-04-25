use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::sync::Mutex;

mod expires_at_flexible {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i64(*value)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<i64, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum FlexibleExpiresAt {
            Number(i64),
            Text(String),
        }

        match FlexibleExpiresAt::deserialize(deserializer)? {
            FlexibleExpiresAt::Number(n) => Ok(n),
            FlexibleExpiresAt::Text(s) => {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.timestamp_millis())
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

static TOKEN_CACHE: std::sync::OnceLock<Mutex<Option<CachedTokens>>> = std::sync::OnceLock::new();

fn cache() -> &'static Mutex<Option<CachedTokens>> {
    TOKEN_CACHE.get_or_init(|| Mutex::new(None))
}

// hq-dev stack (canonical; see hq-pro ADR-0003).
const COGNITO_CLIENT_ID: &str = "7r7an9keh0u6hlsvepl74tvqb0";
const COGNITO_ENDPOINT: &str = "https://cognito-idp.us-east-1.amazonaws.com/";
/// 2-minute buffer before expiry (in milliseconds)
const EXPIRY_BUFFER_MS: i64 = 120_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CognitoTokens {
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: String,
    /// Unix epoch milliseconds. Accepts both i64 and ISO 8601 string on deserialization.
    #[serde(with = "expires_at_flexible")]
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub authenticated: bool,
    pub expires_at: Option<String>,
}

#[derive(Debug)]
struct CachedTokens {
    tokens: CognitoTokens,
    file_mtime: SystemTime,
}

fn tokens_file_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Cannot determine home directory".to_string())?;
    Ok(home.join(".hq").join("cognito-tokens.json"))
}

fn file_mtime(path: &PathBuf) -> Result<SystemTime, String> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("Failed to read file mtime: {}", e))
}

enum TokenReadError {
    Io(std::io::Error),
    Parse(serde_json::Error),
}

fn read_tokens_from_path(path: &Path) -> Result<Option<CognitoTokens>, TokenReadError> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path).map_err(TokenReadError::Io)?;
    let tokens: CognitoTokens =
        serde_json::from_str(&contents).map_err(TokenReadError::Parse)?;
    Ok(Some(tokens))
}

pub fn read_tokens_from_file() -> Result<Option<CognitoTokens>, String> {
    let path = tokens_file_path()?;
    read_tokens_from_path(&path).map_err(|e| match e {
        TokenReadError::Io(e) => format!("Failed to read token file: {}", e),
        TokenReadError::Parse(e) => format!("Failed to parse token file: {}", e),
    })
}

/// Returns true when `path` exists and its `accessToken` is non-empty.
/// Shared reader (see `read_tokens_from_path`), so this stays in sync with
/// `read_tokens_from_file` / `get_tokens`. Malformed JSON is logged and
/// reported as "not signed in" so a half-written file can't trap a user on
/// the login step; I/O errors still bubble. Freshness is intentionally not
/// validated here — the frontend overrides `get_auth_state` on presence.
///
/// Production uses `has_non_empty_stored_token` (async, cache-backed);
/// this path-parameterized variant is kept so tests can exercise the
/// malformed-file / empty-token edges without touching `~/.hq`.
#[allow(dead_code)]
pub fn has_non_empty_token_at(path: &Path) -> Result<bool, String> {
    match read_tokens_from_path(path) {
        Ok(Some(tokens)) => Ok(!tokens.access_token.is_empty()),
        Ok(None) => Ok(false),
        Err(TokenReadError::Parse(e)) => {
            eprintln!(
                "[cognito] has_non_empty_token_at: unreadable token file, treating as absent: {}",
                e
            );
            Ok(false)
        }
        Err(TokenReadError::Io(e)) => Err(format!("Failed to read token file: {}", e)),
    }
}

/// Async variant backed by the shared `TOKEN_CACHE`, so repeated UI calls
/// don't re-read the file. Any upstream failure is logged and collapsed to
/// `Ok(false)` for the skip-login signal only.
pub async fn has_non_empty_stored_token() -> Result<bool, String> {
    match get_tokens().await {
        Ok(Some(tokens)) => Ok(!tokens.access_token.is_empty()),
        Ok(None) => Ok(false),
        Err(e) => {
            eprintln!(
                "[cognito] has_non_empty_stored_token: treating unreadable token as absent: {}",
                e
            );
            Ok(false)
        }
    }
}

pub fn write_tokens_to_file(tokens: &CognitoTokens) -> Result<(), String> {
    let path = tokens_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create .hq directory: {}", e))?;
    }
    let contents = serde_json::to_string_pretty(tokens)
        .map_err(|e| format!("Failed to serialize tokens: {}", e))?;

    let tmp_path = path.with_file_name(format!(
        ".cognito-tokens.json.tmp.{}",
        std::process::id()
    ));
    std::fs::write(&tmp_path, &contents)
        .map_err(|e| format!("Failed to write temp token file: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&tmp_path, perms)
            .map_err(|e| format!("Failed to set temp file permissions: {}", e))?;
    }

    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to rename temp token file: {}", e))?;
    Ok(())
}

/// Get tokens, using in-memory cache with mtime invalidation.
pub async fn get_tokens() -> Result<Option<CognitoTokens>, String> {
    let path = tokens_file_path()?;

    // Get mtime — treat NotFound as "no file" (avoids TOCTOU with path.exists())
    let current_mtime = match std::fs::metadata(&path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut guard = cache().lock().await;
            *guard = None;
            return Ok(None);
        }
        Err(e) => return Err(format!("Failed to read file mtime: {}", e)),
    };
    let guard = cache().lock().await;

    if let Some(ref cached) = *guard {
        if cached.file_mtime == current_mtime {
            return Ok(Some(cached.tokens.clone()));
        }
    }

    // Cache miss or mtime changed — re-read
    drop(guard);
    let tokens = read_tokens_from_file()?;
    if let Some(ref tokens) = tokens {
        let mut guard = cache().lock().await;
        *guard = Some(CachedTokens {
            tokens: tokens.clone(),
            file_mtime: current_mtime,
        });
    }
    Ok(tokens)
}

/// Update both the file and the in-memory cache.
pub async fn set_tokens(tokens: &CognitoTokens) -> Result<(), String> {
    write_tokens_to_file(tokens)?;
    let path = tokens_file_path()?;
    let mtime = file_mtime(&path)?;
    let mut guard = cache().lock().await;
    *guard = Some(CachedTokens {
        tokens: tokens.clone(),
        file_mtime: mtime,
    });
    Ok(())
}

pub fn is_expired(tokens: &CognitoTokens) -> bool {
    if tokens.expires_at <= 0 {
        return true; // treat corrupt/zero timestamps as expired
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    tokens.expires_at - now_ms < EXPIRY_BUFFER_MS
}

pub fn expires_at_iso(tokens: &CognitoTokens) -> String {
    format_unix_ms_as_iso(tokens.expires_at.max(0))
}

fn format_unix_ms_as_iso(ms: i64) -> String {
    let total_secs = ms / 1000;
    let millis = ms % 1000;

    // Days since epoch
    let days = total_secs / 86400;
    let day_secs = total_secs % 86400;
    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    // Convert days since epoch to year-month-day
    // Algorithm from Howard Hinnant
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, m, d, hours, minutes, seconds, millis
    )
}

/// Cognito InitiateAuth response shape (partial)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InitiateAuthResponse {
    authentication_result: AuthenticationResult,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AuthenticationResult {
    access_token: String,
    id_token: Option<String>,
    expires_in: i64,
    // Cognito does not return a new refresh token on REFRESH_TOKEN_AUTH
}

pub async fn refresh_access_token(refresh_token: &str) -> Result<CognitoTokens, String> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "AuthFlow": "REFRESH_TOKEN_AUTH",
        "ClientId": COGNITO_CLIENT_ID,
        "AuthParameters": {
            "REFRESH_TOKEN": refresh_token
        }
    });

    let response = client
        .post(COGNITO_ENDPOINT)
        .header("Content-Type", "application/x-amz-json-1.1")
        .header(
            "X-Amz-Target",
            "AWSCognitoIdentityProviderService.InitiateAuth",
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Cognito refresh request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        return Err(format!(
            "Cognito refresh failed ({}): {}",
            status, body_text
        ));
    }

    let result: InitiateAuthResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Cognito response: {}", e))?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    Ok(CognitoTokens {
        access_token: result.authentication_result.access_token,
        id_token: result.authentication_result.id_token,
        refresh_token: refresh_token.to_string(),
        expires_at: now_ms + (result.authentication_result.expires_in * 1000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile;

    #[test]
    fn test_is_expired_future_token() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let tokens = CognitoTokens {
            access_token: "test".to_string(),
            id_token: Some("test".to_string()),
            refresh_token: "test".to_string(),
            expires_at: now_ms + 300_000, // 5 minutes from now
        };
        assert!(!is_expired(&tokens));
    }

    #[test]
    fn test_is_expired_within_buffer() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let tokens = CognitoTokens {
            access_token: "test".to_string(),
            id_token: Some("test".to_string()),
            refresh_token: "test".to_string(),
            expires_at: now_ms + 60_000, // 1 minute from now (within 2-min buffer)
        };
        assert!(is_expired(&tokens));
    }

    #[test]
    fn test_is_expired_past_token() {
        let tokens = CognitoTokens {
            access_token: "test".to_string(),
            id_token: Some("test".to_string()),
            refresh_token: "test".to_string(),
            expires_at: 1000, // long past
        };
        assert!(is_expired(&tokens));
    }

    #[test]
    fn test_format_unix_ms_as_iso() {
        // 2024-01-15T12:30:45.123Z
        let iso = format_unix_ms_as_iso(1705321845123);
        assert_eq!(iso, "2024-01-15T12:30:45.123Z");
    }

    #[test]
    fn test_format_unix_ms_as_iso_epoch() {
        let iso = format_unix_ms_as_iso(0);
        assert_eq!(iso, "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn test_expires_at_iso() {
        let tokens = CognitoTokens {
            access_token: "test".to_string(),
            id_token: None,
            refresh_token: "test".to_string(),
            expires_at: 1705321845123,
        };
        let iso = expires_at_iso(&tokens);
        assert_eq!(iso, "2024-01-15T12:30:45.123Z");
    }

    #[test]
    fn test_cognito_tokens_serialize_deserialize() {
        let tokens = CognitoTokens {
            access_token: "acc".to_string(),
            id_token: Some("id".to_string()),
            refresh_token: "ref".to_string(),
            expires_at: 1705321845123,
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let parsed: CognitoTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "acc");
        assert_eq!(parsed.refresh_token, "ref");
        assert_eq!(parsed.expires_at, 1705321845123);
        assert_eq!(parsed.id_token, Some("id".to_string()));
    }

    #[test]
    fn test_cognito_tokens_deserialize_without_id_token() {
        let json = r#"{"accessToken":"acc","refreshToken":"ref","expiresAt":123}"#;
        let tokens: CognitoTokens = serde_json::from_str(json).unwrap();
        assert_eq!(tokens.access_token, "acc");
        assert_eq!(tokens.id_token, None);
    }

    #[test]
    fn test_write_and_read_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognito-tokens.json");
        let tokens = CognitoTokens {
            access_token: "a".to_string(),
            id_token: Some("i".to_string()),
            refresh_token: "r".to_string(),
            expires_at: 999,
        };
        let contents = serde_json::to_string_pretty(&tokens).unwrap();
        std::fs::write(&path, &contents).unwrap();

        let read_back: CognitoTokens =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read_back.access_token, "a");
        assert_eq!(read_back.expires_at, 999);
    }

    #[test]
    fn test_auth_state_serialization() {
        let state = AuthState {
            authenticated: true,
            expires_at: Some("2024-01-15T12:30:45.123Z".to_string()),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"authenticated\":true"));
        assert!(json.contains("\"expiresAt\""));
    }

    #[test]
    fn test_auth_state_unauthenticated() {
        let state = AuthState {
            authenticated: false,
            expires_at: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"authenticated\":false"));
        assert!(json.contains("\"expiresAt\":null"));
    }

    #[test]
    fn test_deserialize_expires_at_as_number() {
        let json = r#"{"accessToken":"a","refreshToken":"r","expiresAt":1705321845123}"#;
        let tokens: CognitoTokens = serde_json::from_str(json).unwrap();
        assert_eq!(tokens.expires_at, 1705321845123);
    }

    #[test]
    fn test_deserialize_expires_at_as_iso_string() {
        let json =
            r#"{"accessToken":"a","refreshToken":"r","expiresAt":"2024-01-15T12:30:45.123Z"}"#;
        let tokens: CognitoTokens = serde_json::from_str(json).unwrap();
        assert_eq!(tokens.expires_at, 1705321845123);
    }

    #[test]
    fn test_deserialize_expires_at_invalid_string_fails() {
        let json = r#"{"accessToken":"a","refreshToken":"r","expiresAt":"not-a-date"}"#;
        let result: Result<CognitoTokens, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_expires_at_always_number() {
        let tokens = CognitoTokens {
            access_token: "a".to_string(),
            id_token: None,
            refresh_token: "r".to_string(),
            expires_at: 1705321845123,
        };
        let json = serde_json::to_string(&tokens).unwrap();
        assert!(json.contains("\"expiresAt\":1705321845123"));
    }

    #[test]
    fn test_has_non_empty_token_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognito-tokens.json");
        assert!(!has_non_empty_token_at(&path).unwrap());
    }

    #[test]
    fn test_has_non_empty_token_with_real_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognito-tokens.json");
        let tokens = CognitoTokens {
            access_token: "abc123".to_string(),
            id_token: Some("id".to_string()),
            refresh_token: "r".to_string(),
            expires_at: 1,
        };
        std::fs::write(&path, serde_json::to_string(&tokens).unwrap()).unwrap();
        assert!(has_non_empty_token_at(&path).unwrap());
    }

    #[test]
    fn test_has_non_empty_token_empty_access_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognito-tokens.json");
        let json = r#"{"accessToken":"","refreshToken":"r","expiresAt":1}"#;
        std::fs::write(&path, json).unwrap();
        assert!(!has_non_empty_token_at(&path).unwrap());
    }

    #[test]
    fn test_has_non_empty_token_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognito-tokens.json");
        std::fs::write(&path, "{not valid json").unwrap();
        // Malformed content → treat as not-logged-in rather than bubbling an error.
        assert!(!has_non_empty_token_at(&path).unwrap());
    }

    #[test]
    fn test_has_non_empty_token_with_expired_token_still_true() {
        // Freshness is not validated here — an expired but non-empty token
        // still counts as "logged in" for the onboarding skip signal.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cognito-tokens.json");
        let tokens = CognitoTokens {
            access_token: "still-here".to_string(),
            id_token: None,
            refresh_token: "r".to_string(),
            expires_at: 1, // ancient
        };
        std::fs::write(&path, serde_json::to_string(&tokens).unwrap()).unwrap();
        assert!(has_non_empty_token_at(&path).unwrap());
    }

    #[test]
    fn test_atomic_write_no_leftover_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let hq_dir = dir.path().join(".hq");
        std::fs::create_dir_all(&hq_dir).unwrap();

        let path = hq_dir.join("cognito-tokens.json");
        let tokens = CognitoTokens {
            access_token: "a".to_string(),
            id_token: Some("i".to_string()),
            refresh_token: "r".to_string(),
            expires_at: 999,
        };
        let contents = serde_json::to_string_pretty(&tokens).unwrap();

        let tmp_path = path.with_file_name(format!(
            ".cognito-tokens.json.tmp.{}",
            std::process::id()
        ));
        std::fs::write(&tmp_path, &contents).unwrap();
        std::fs::rename(&tmp_path, &path).unwrap();

        assert!(path.exists());
        assert!(!tmp_path.exists());

        let read_back: CognitoTokens =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read_back.access_token, "a");
        assert_eq!(read_back.expires_at, 999);
    }
}
