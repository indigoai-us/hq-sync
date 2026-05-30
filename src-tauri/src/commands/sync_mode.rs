//! `get_sync_mode` / `set_sync_mode` Tauri commands — the menubar surface for
//! per-company selective-download (Phase D of the HQ Pro selective-download
//! project; the CLI equivalent is `hq sync mode <all|shared|custom>`).
//!
//! ## What "mode" means
//!
//! A membership's sync-mode governs ONLY what a sync DOWNLOADS to this machine
//! (the local footprint) — it never changes access. Owners/admins keep their
//! role-bypass regardless of mode; flipping to `shared` just means the next
//! sync pulls only the prefixes that have been explicitly shared/granted (plus
//! any pins from `.hq/pins.json`, which `resolvePullScope` in hq-cloud's
//! sync-runner unions into scope). Flipping back to `all` restores the full
//! download. The authoritative store is server-side, per-membership, via the
//! vault `…/sync-config` endpoints (see [`VaultClient::get_membership_sync_config`]
//! / [`VaultClient::set_membership_sync_config`]).
//!
//! ## Why these commands resolve a membership_key
//!
//! The frontend speaks in company *slugs* (what the popover renders), but the
//! sync-config endpoints are keyed by the composite `membership_key`
//! (`personUid#companyUid`). [`resolve_membership_key`] bridges the two: it
//! finds the company entity by slug, then locates the caller's membership for
//! that company. The `personal` vault is not a company membership and has no
//! sync-config row — callers should not toggle it (the UI hides the control
//! for `kind === 'personal'`).

use crate::commands::sync::{resolve_jwt, resolve_vault_api_url};
use crate::commands::vault_client::{
    MembershipSyncConfig, SetMembershipSyncConfigInput, VaultClient,
};

/// The two modes the menubar toggle exposes. `custom` is intentionally CLI-only
/// (`hq sync mode custom --paths …`) because it needs a path list the popover
/// has no surface to collect — a bare `custom` with no paths would be rejected
/// server-side anyway.
fn validate_toggle_mode(mode: &str) -> Result<(), String> {
    match mode {
        "all" | "shared" => Ok(()),
        "custom" => Err(
            "custom sync mode needs a path list — use `hq sync mode custom --paths …` from the CLI"
                .to_string(),
        ),
        other => Err(format!(
            "unsupported sync mode '{other}' — expected 'all' or 'shared'"
        )),
    }
}

/// Build a VaultClient from the stored Cognito token + resolved vault base URL.
/// Mirrors how `list_syncable_workspaces` / telemetry construct the client.
async fn vault_client() -> Result<VaultClient, String> {
    let url = resolve_vault_api_url()?;
    let jwt = resolve_jwt().await?;
    Ok(VaultClient::new(&url, &jwt))
}

/// Resolve the composite `membership_key` (`personUid#companyUid`) for a
/// company slug from the caller's own memberships.
///
/// Two vault calls: `find_entity_by_slug` (slug → company UID) then
/// `list_my_memberships` (company UID → membership). Falls back to synthesizing
/// the key from `person_uid#company_uid` when the live API omits
/// `membership_key` (older vault builds / test fixtures).
async fn resolve_membership_key(
    vault: &VaultClient,
    company_slug: &str,
) -> Result<String, String> {
    let entity = vault
        .find_entity_by_slug("company", company_slug)
        .await
        .map_err(|e| format!("resolve company '{company_slug}': {e}"))?
        .ok_or_else(|| format!("no cloud company found for '{company_slug}'"))?;

    let memberships = vault
        .list_my_memberships()
        .await
        .map_err(|e| format!("list memberships: {e}"))?;

    let mem = memberships
        .iter()
        .find(|m| m.company_uid == entity.uid)
        .ok_or_else(|| format!("you have no membership in '{company_slug}'"))?;

    Ok(mem
        .membership_key
        .clone()
        .unwrap_or_else(|| format!("{}#{}", mem.person_uid, mem.company_uid)))
}

// ── Tauri commands ──────────────────────────────────────────────────────────

/// Resolve the current sync-mode for a company. Returns the full config so the
/// UI can distinguish an explicit `shared` from the `all` default
/// (`isDefault: true` means no sync-config row exists yet → effective `all`).
#[tauri::command]
pub async fn get_sync_mode(company_slug: String) -> Result<MembershipSyncConfig, String> {
    let vault = vault_client().await?;
    let key = resolve_membership_key(&vault, &company_slug).await?;
    vault
        .get_membership_sync_config(&key)
        .await
        .map_err(|e| e.to_string())
}

/// Set a company's sync-mode (footprint only — access is unaffected). The
/// menubar toggle is restricted to `all` / `shared`; `custom` is CLI-only.
/// Returns the resulting config so the caller can update its UI optimistically
/// without a follow-up `get_sync_mode`.
#[tauri::command]
pub async fn set_sync_mode(
    company_slug: String,
    mode: String,
) -> Result<MembershipSyncConfig, String> {
    validate_toggle_mode(&mode)?;
    let vault = vault_client().await?;
    let key = resolve_membership_key(&vault, &company_slug).await?;
    vault
        .set_membership_sync_config(
            &key,
            &SetMembershipSyncConfigInput {
                sync_mode: mode,
                custom_paths: None,
            },
        )
        .await
        .map_err(|e| e.to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(url: &str) -> VaultClient {
        VaultClient::new(url, "test-token")
    }

    #[test]
    fn validate_toggle_mode_accepts_all_and_shared() {
        assert!(validate_toggle_mode("all").is_ok());
        assert!(validate_toggle_mode("shared").is_ok());
    }

    #[test]
    fn validate_toggle_mode_rejects_custom_with_guidance() {
        let err = validate_toggle_mode("custom").unwrap_err();
        assert!(err.contains("hq sync mode custom"), "got: {err}");
    }

    #[test]
    fn validate_toggle_mode_rejects_garbage() {
        let err = validate_toggle_mode("everything").unwrap_err();
        assert!(err.contains("unsupported sync mode 'everything'"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_membership_key_uses_membership_key_from_api() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/company/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "entity": {
                    "uid": "cmp_a", "slug": "acme", "type": "company",
                    "name": "Acme", "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/membership/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "memberships": [
                    { "personUid": "prs_x", "companyUid": "cmp_other",
                      "membershipKey": "prs_x#cmp_other", "status": "active" },
                    { "personUid": "prs_x", "companyUid": "cmp_a",
                      "membershipKey": "prs_x#cmp_a", "status": "active" }
                ]
            })))
            .mount(&server)
            .await;

        let key = resolve_membership_key(&client(&server.uri()), "acme")
            .await
            .unwrap();
        assert_eq!(key, "prs_x#cmp_a");
    }

    #[tokio::test]
    async fn resolve_membership_key_synthesizes_when_api_omits_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/company/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "entity": {
                    "uid": "cmp_a", "slug": "acme", "type": "company",
                    "status": "active", "createdAt": "2026-01-01T00:00:00Z"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/membership/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "memberships": [
                    { "personUid": "prs_x", "companyUid": "cmp_a", "status": "active" }
                ]
            })))
            .mount(&server)
            .await;

        let key = resolve_membership_key(&client(&server.uri()), "acme")
            .await
            .unwrap();
        assert_eq!(key, "prs_x#cmp_a");
    }

    #[tokio::test]
    async fn resolve_membership_key_errors_when_company_unknown() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/company/ghost"))
            .respond_with(ResponseTemplate::new(404).set_body_json(&json!({"error": "not found"})))
            .mount(&server)
            .await;

        let err = resolve_membership_key(&client(&server.uri()), "ghost")
            .await
            .unwrap_err();
        assert!(err.contains("no cloud company found for 'ghost'"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_membership_key_errors_when_not_a_member() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/company/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "entity": {
                    "uid": "cmp_a", "slug": "acme", "type": "company",
                    "status": "active", "createdAt": "2026-01-01T00:00:00Z"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/membership/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({ "memberships": [] })))
            .mount(&server)
            .await;

        let err = resolve_membership_key(&client(&server.uri()), "acme")
            .await
            .unwrap_err();
        assert!(err.contains("no membership in 'acme'"), "got: {err}");
    }
}
