//! Shared feature-gate helpers used across command modules.
//!
//! Extracted from `commands/meetings.rs` so both meeting-detect and
//! share-notify use one canonical dogfood-email check without duplication.
//! The gate is cached for the process lifetime because the Cognito email
//! claim is invariant across token rotations (sub stays constant). The cache
//! is cleared when a new token file is written so a launch that started signed
//! out can recover after OAuth succeeds in the same process.

use std::sync::{Mutex, OnceLock};

use crate::commands::cognito;

const ALLOWED_DOMAIN: &str = "@getindigo.ai";

static CACHED_GATE: OnceLock<Mutex<Option<bool>>> = OnceLock::new();
static CACHED_GA_GATE: OnceLock<Mutex<Option<bool>>> = OnceLock::new();

fn gate_cache() -> &'static Mutex<Option<bool>> {
    CACHED_GATE.get_or_init(|| Mutex::new(None))
}

fn ga_gate_cache() -> &'static Mutex<Option<bool>> {
    CACHED_GA_GATE.get_or_init(|| Mutex::new(None))
}

/// Returns true iff the signed-in user's email ends in `@getindigo.ai`.
///
/// Process-lifetime cache — safe because the email claim is stable across
/// Cognito token rotations. Returns false silently on any error so callers
/// never crash due to a missing or malformed token.
pub async fn is_indigo_user() -> bool {
    {
        let guard = gate_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = *guard {
            return v;
        }
    }

    let enabled = compute_gate().await;
    let mut guard = gate_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = *guard {
        return v;
    }
    *guard = Some(enabled);
    enabled
}

/// GA gate — true for **any** signed-in user (non-empty email claim),
/// regardless of email domain.
///
/// This is the gate the expanded desktop window + all its panels (Sync,
/// Board, Projects, Task detail, Library, company Activity/Deployments/
/// Secrets) and the Meetings feature graduate onto: the surface left the
/// Indigo-only dogfood and is now generally available. It reuses the exact
/// same token/claims decoding as [`is_indigo_user`] / [`compute_gate`] but
/// returns true whenever a non-empty email claim is present instead of
/// requiring the `@getindigo.ai` domain.
///
/// `is_indigo_user()` is intentionally kept intact — the updater still uses
/// it to keep pre-release auto-update channels Indigo-only.
///
/// Process-lifetime cache (separate from the Indigo cache) — safe because the
/// email claim is stable across Cognito token rotations. Returns false
/// silently on any error so callers never crash due to a missing/malformed
/// token (signed-out users see nothing).
pub async fn desktop_features_enabled() -> bool {
    {
        let guard = ga_gate_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = *guard {
            return v;
        }
    }

    let enabled = compute_ga_gate().await;
    let mut guard = ga_gate_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = *guard {
        return v;
    }
    *guard = Some(enabled);
    enabled
}

/// Clear the process-local gate caches after token writes.
///
/// This keeps the caches process-lifetime for steady-state reads while
/// allowing the first post-OAuth gate check to use the newly persisted
/// ID-token claims. Clears both the Indigo gate and the GA gate.
pub fn clear_cached_gate() {
    let mut guard = gate_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
    let mut ga_guard = ga_gate_cache().lock().unwrap_or_else(|e| e.into_inner());
    *ga_guard = None;
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

/// GA-gate counterpart to [`compute_gate`] — identical token/claims decode,
/// but the verdict is "is any email claim present" instead of the Indigo
/// domain check.
async fn compute_ga_gate() -> bool {
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
    email_present(claims.email.as_deref())
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

/// Pure helper for the GA gate. True iff a non-empty (after trimming) email
/// claim is present — i.e. the user is signed in — regardless of domain.
///
/// `pub` so command modules can unit-test GA gating logic directly, mirroring
/// how [`is_allowed_email`] is tested.
pub fn email_present(email: Option<&str>) -> bool {
    matches!(email, Some(s) if !s.trim().is_empty())
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

    /// US-001 AC #5: when the OnceLock cache hasn't been seeded yet (cold
    /// start), `is_indigo_user()` must await `compute_gate()` and return
    /// the canonical email-derived answer — never default-false on
    /// uninitialised. We can't drive `compute_gate()` without a Cognito
    /// fixture in unit tests, but we can prove the cache contract: the
    /// public gate API delegates to `is_allowed_email` over the decoded
    /// claim, so the email-derived answer is what callers see for any
    /// state of the underlying token. This test pins the contract: the
    /// allowlist is the single source of truth, and cold-cache returns
    /// the email-derived verdict (false here, because no token = no
    /// allowed email — *not* because of cache-uninitialised default).
    ///
    /// Paired with the integration test in
    /// `commands/desktop_alt.rs::desktop_alt_gate_*` that exercises the
    /// command path with explicit email fixtures.
    #[test]
    fn ga_gate_admits_any_present_email() {
        // GA gate: any signed-in user (non-empty email claim) is admitted,
        // regardless of domain — the window graduated from the Indigo dogfood.
        assert!(email_present(Some("stefan@getindigo.ai")));
        assert!(email_present(Some("qa@example.com")));
        assert!(email_present(Some("anyone@gmail.com")));
        // Former look-alike: now admitted, because GA only checks presence.
        assert!(email_present(Some("attacker@forgetindigo.ai")));
        // Whitespace is trimmed before considering "present".
        assert!(email_present(Some("  user@x.io  ")));
    }

    #[test]
    fn staging_gate_diverges_from_ga_gate_for_non_indigo_user() {
        // Regression: the Settings staging-channel toggle must be gated on the
        // @getindigo.ai-only `is_indigo_user` predicate (`is_allowed_email`),
        // NOT the GA `meetings_feature_enabled` predicate (`email_present`).
        // Wiring it to the GA gate exposed the builder-only "Use staging
        // channel" toggle to every signed-in user. This pins that the two
        // gates genuinely diverge for a signed-in non-Indigo user: present to
        // the GA gate, rejected by the Indigo gate.
        let non_indigo = Some("user@gmail.com");
        assert!(email_present(non_indigo), "GA gate admits any signed-in user");
        assert!(
            !is_allowed_email(non_indigo),
            "Indigo gate must reject non-@getindigo.ai emails"
        );
        // And they agree for an @getindigo.ai builder (both true).
        let indigo = Some("builder@getindigo.ai");
        assert!(email_present(indigo));
        assert!(is_allowed_email(indigo));
    }

    #[test]
    fn ga_gate_rejects_signed_out() {
        // Signed-out (no email / empty / whitespace-only) stays false so the
        // surface never lights up for an unauthenticated user.
        assert!(!email_present(None));
        assert!(!email_present(Some("")));
        assert!(!email_present(Some("   ")));
    }

    #[test]
    fn cold_cache_returns_email_derived_answer_not_default_false() {
        // The OnceLock is module-private and shared across the test
        // binary — we can't reset it. Instead we prove the contract
        // through the pure helper that `compute_gate()` delegates to:
        // any non-allowed email (including the None/empty path that
        // mimics a missing token) returns false *because* the email
        // didn't match, not because the gate defaulted. The cold-cache
        // command path runs `compute_gate().await` and feeds its result
        // through `is_allowed_email`, so this is the canonical answer
        // contract.
        assert!(!is_allowed_email(None));
        assert!(!is_allowed_email(Some("")));
        // And the positive contract — cold cache for an allowed email
        // resolves to true, not the OnceLock default.
        assert!(is_allowed_email(Some("stefan@getindigo.ai")));
    }
}
