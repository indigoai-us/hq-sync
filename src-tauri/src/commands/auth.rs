use super::cognito::{self, AuthState};

#[tauri::command]
pub async fn get_auth_state() -> Result<AuthState, String> {
    let tokens = cognito::get_tokens().await?;

    let Some(tokens) = tokens else {
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
                Ok(AuthState {
                    authenticated: true,
                    expires_at: Some(iso),
                })
            }
            Err(_) => {
                // Refresh failed — treat as unauthenticated
                Ok(AuthState {
                    authenticated: false,
                    expires_at: None,
                })
            }
        }
    } else {
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
