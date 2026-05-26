//! Shared feature-gate helpers used across command modules.
//!
//! Extracted from `commands/meetings.rs` so both meeting-detect and
//! share-notify use one canonical dogfood-email check without duplication.
//! The gate is cached for the process lifetime because the Cognito email
//! claim is invariant across token rotations (sub stays constant).

use std::sync::OnceLock;

use crate::commands::cognito;

const ALLOWED_DOMAIN: &str = "@getindigo.ai";

static CACHED_GATE: OnceLock<bool> = OnceLock::new();

/// Returns true iff the signed-in user's email ends in `@getindigo.ai`.
///
/// Process-lifetime cache — safe because the email claim is stable across
/// Cognito token rotations. Returns false silently on any error so callers
/// never crash due to a missing or malformed token.
pub async fn is_indigo_user() -> bool {
    if let Some(v) = CACHED_GATE.get() {
        return *v;
    }
    let enabled = compute_gate().await;
    let _ = CACHED_GATE.set(enabled);
    enabled
}

async fn compute_gate() -> bool {
    let tokens = match cognito::get_tokens().await {
        Ok(Some(t)) => t,
        _ => return false,
    };
    let id_token = match tokens.id_token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    let claims = match cognito::decode_id_token_claims(id_token) {
        Ok(c) => c,
        Err(_) => return false,
    };
    is_allowed_email(claims.email.as_deref())
}

/// Pure helper. Case-insensitive suffix match on `@getindigo.ai`.
/// The leading `@` prevents look-alike domains like `forgetindigo.ai`.
///
/// `pub` so command modules can unit-test gating logic directly.
pub fn is_allowed_email(email: Option<&str>) -> bool {
    match email {
        Some(s) if !s.is_empty() => s.to_ascii_lowercase().ends_with(ALLOWED_DOMAIN),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_positive_cases() {
        assert!(is_allowed_email(Some("stefan@getindigo.ai")));
        assert!(is_allowed_email(Some("STEFAN@GETINDIGO.AI")));
        assert!(is_allowed_email(Some("admin@getindigo.ai")));
    }

    #[test]
    fn test_negative_cases() {
        assert!(!is_allowed_email(Some("stefan@gmail.com")));
        assert!(!is_allowed_email(None));
        assert!(!is_allowed_email(Some("")));
        // look-alike: prefix doesn't include '@'
        assert!(!is_allowed_email(Some("user@forgetindigo.ai")));
        assert!(!is_allowed_email(Some("user@notgetindigo.ai")));
        // bare domain without @
        assert!(!is_allowed_email(Some("getindigo.ai")));
    }
}
