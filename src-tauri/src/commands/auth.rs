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

/// Sign out: clear the locally stored Cognito tokens (file + in-memory cache)
/// and reset the Sentry user scope. After this, `get_auth_state` / a relaunch
/// both report unauthenticated — without it, a frontend-only sign-out leaves the
/// token file on disk and the app re-authenticates silently on next launch.
#[tauri::command]
pub async fn sign_out() -> Result<(), String> {
    cognito::clear_tokens().await?;
    clear_sentry_user();
    Ok(())
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

