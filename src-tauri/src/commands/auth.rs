use super::cognito::{self, AuthState, CognitoTokens};

/// Update Sentry's scoped user context to the Cognito identity carried in
/// `tokens`. Best-effort: a malformed/missing id_token just clears the user
/// rather than failing — Sentry stays useful even when claims parsing breaks.
fn set_sentry_user_from_tokens(tokens: &CognitoTokens) {
    let claims = tokens
        .id_token
        .as_deref()
        .and_then(|tok| cognito::decode_id_token_claims(tok).ok());
    sentry::configure_scope(|scope| match claims {
        Some(c) => scope.set_user(Some(sentry::User {
            id: c.sub.clone(),
            email: c.email.clone(),
            username: Some(c.display_name()),
            ..Default::default()
        })),
        None => scope.set_user(None),
    });
}

fn clear_sentry_user() {
    sentry::configure_scope(|scope| scope.set_user(None));
}

/// Extract the `email` claim from an id_token. Returns `None` for missing,
/// empty, or unparseable tokens. Email casing is preserved verbatim — the
/// caller decides how to compare it (the @getindigo.ai gate is case-insensitive).
pub fn extract_email_from_id_token(id_token: &str) -> Option<String> {
    let claims = cognito::decode_id_token_claims(id_token).ok()?;
    claims.email.filter(|s| !s.is_empty())
}

/// Tauri command that returns the signed-in user's email, or `None` when no
/// stored token is present. Used by the Auto-sync (Beta) feature flag in
/// Settings to decide whether to render the toggle.
#[tauri::command]
pub async fn get_user_email() -> Result<Option<String>, String> {
    let Some(tokens) = cognito::get_tokens().await? else {
        return Ok(None);
    };
    let Some(id_token) = tokens.id_token else {
        return Ok(None);
    };
    Ok(extract_email_from_id_token(&id_token))
}

#[tauri::command]
pub async fn get_auth_state() -> Result<AuthState, String> {
    let tokens = cognito::get_tokens().await?;

    let Some(tokens) = tokens else {
        clear_sentry_user();
        return Ok(AuthState {
            authenticated: false,
            expires_at: None,
        });
    };

    if cognito::is_expired(&tokens) {
        // Attempt silent refresh
        match cognito::refresh_access_token(&tokens.refresh_token).await {
            Ok(new_tokens) => {
                let iso = cognito::expires_at_iso(&new_tokens);
                cognito::set_tokens(&new_tokens).await?;
                set_sentry_user_from_tokens(&new_tokens);
                Ok(AuthState {
                    authenticated: true,
                    expires_at: Some(iso),
                })
            }
            Err(_) => {
                // Refresh failed — treat as unauthenticated
                clear_sentry_user();
                Ok(AuthState {
                    authenticated: false,
                    expires_at: None,
                })
            }
        }
    } else {
        set_sentry_user_from_tokens(&tokens);
        Ok(AuthState {
            authenticated: true,
            expires_at: Some(cognito::expires_at_iso(&tokens)),
        })
    }
}

/// Returns true when `~/.hq/cognito-tokens.json` exists and contains a
/// non-empty `accessToken`. Used by the onboarding UI to skip the sign-in
/// step when a token is already on disk, without round-tripping to Cognito
/// for an expiry/refresh check.
#[tauri::command]
pub async fn has_stored_token() -> Result<bool, String> {
    cognito::has_non_empty_stored_token().await
}

#[tauri::command]
pub async fn refresh_tokens() -> Result<AuthState, String> {
    let tokens = cognito::get_tokens().await?;

    let Some(tokens) = tokens else {
        return Err("No tokens found — user is not signed in".to_string());
    };

    let new_tokens = cognito::refresh_access_token(&tokens.refresh_token).await?;
    let iso = cognito::expires_at_iso(&new_tokens);
    cognito::set_tokens(&new_tokens).await?;

    Ok(AuthState {
        authenticated: true,
        expires_at: Some(iso),
    })
}

#[cfg(test)]
mod extract_email_from_id_token_tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    fn make_token(payload_json: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        format!("{header}.{payload}.")
    }

    #[test]
    fn returns_email_when_claim_present_and_non_empty() {
        let token = make_token(r#"{"sub":"u","email":"alice@getindigo.ai"}"#);
        assert_eq!(
            extract_email_from_id_token(&token),
            Some("alice@getindigo.ai".to_string())
        );
    }

    #[test]
    fn returns_none_when_email_claim_absent() {
        let token = make_token(r#"{"sub":"u"}"#);
        assert_eq!(extract_email_from_id_token(&token), None);
    }

    #[test]
    fn returns_none_when_email_claim_empty_string() {
        let token = make_token(r#"{"sub":"u","email":""}"#);
        assert_eq!(extract_email_from_id_token(&token), None);
    }

    #[test]
    fn returns_none_when_token_missing_payload_segment() {
        // No dots → split('.').nth(1) is None → decode_id_token_claims errors
        assert_eq!(extract_email_from_id_token("not-a-jwt"), None);
    }

    #[test]
    fn returns_none_when_payload_is_invalid_base64() {
        assert_eq!(extract_email_from_id_token("aaa.!!!not-base64!!!.zzz"), None);
    }

    #[test]
    fn returns_none_when_payload_is_invalid_json() {
        // Valid base64url but the bytes aren't JSON
        let payload = URL_SAFE_NO_PAD.encode(b"not-json");
        let token = format!("aaa.{payload}.zzz");
        assert_eq!(extract_email_from_id_token(&token), None);
    }

    #[test]
    fn preserves_email_casing_verbatim() {
        // The @getindigo.ai gate is the caller's job — this helper just returns
        // the claim as Cognito stored it. Verify we don't lower/upper-case it.
        let token = make_token(r#"{"email":"Alice.Smith@GetIndigo.AI"}"#);
        assert_eq!(
            extract_email_from_id_token(&token),
            Some("Alice.Smith@GetIndigo.AI".to_string())
        );
    }
}
