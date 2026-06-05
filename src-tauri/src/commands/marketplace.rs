//! Creator-marketplace browse client (US-008).
//!
//! Backs the desktop-alt **Marketplace** tab. Calls the PUBLIC hq-pro listings
//! routes — `GET /v1/listings` (browse) and `GET /v1/listings/{id}` (detail) —
//! both declared `authorizationType: NONE`, so NO Cognito token is attached.
//! Everything returned has already passed through the server-side
//! `toPublicListing` allowlist (US-005): only `approved` listings, redacted to a
//! public shape that never leaks moderation state, the author's internal uid, or
//! the raw S3 key. We mirror that public projection 1:1 here.
//!
//! Wire contract (hq-pro `src/listings/public-projection.ts`):
//!   * Browse  → `{ "listings": PublicListing[] }`
//!   * Detail  → `{ "listing": PublicListing & { downloadUrl } }`
//!   * PublicListing = { id, type, name, slug, version, author, summary?,
//!     contributes?, createdAt } — `author` is the public HANDLE string (not an
//!     object; the internal `creatorUid` is never exposed).
//!
//! Base URL resolution reuses `sync::resolve_vault_api_url` (env override →
//! `~/.hq/config.json` → default), the same resolver the desktop-alt Board /
//! Activity / Secrets readers use. The HTTP client is the shared timeout-guarded
//! `util::client_info::build_client`.
//!
//! These are app-registered Tauri commands authorized by `core:default` in
//! `capabilities/desktop-alt.json` (custom commands are not gated by per-command
//! permission identifiers), so no allow-* tokens are added. Unlike the Board /
//! Library readers this surface is intentionally NOT behind the Indigo gate: the
//! marketplace is public, so any signed-in (or not) desktop user can browse it.

use std::path::{Component, Path, PathBuf};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::commands::sync::{resolve_jwt, resolve_vault_api_url};
use crate::commands::vault_client::VaultClient;
use crate::util::client_info::build_client;
use crate::util::logfile::log;
use crate::util::paths;

// ---- wire types (camelCase, mirror the hq-pro public projection) ------------

/// One approved listing as exposed by the public browse/detail routes. Every
/// field here is something the server explicitly allowlisted for anonymous
/// callers (US-005) — there is no moderation state, no creator uid, no S3 key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceListing {
    /// Stable listing id — the key the detail command takes.
    pub id: String,
    /// What the pack contains (`skill` | `worker`).
    #[serde(rename = "type")]
    pub type_: String,
    /// Human-readable listing name.
    pub name: String,
    /// Pack slug — the install identifier (`hq install marketplace:<slug>`).
    pub slug: String,
    /// Published semantic version.
    pub version: String,
    /// Author's PUBLIC handle (a string, not an object — the internal
    /// `creatorUid` is never exposed by the public projection).
    #[serde(default)]
    pub author: String,
    /// Short directory description, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Human-readable summary of what the pack contributes, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributes: Option<String>,
    /// ISO-8601 publish timestamp (recency sort on the server).
    #[serde(default)]
    pub created_at: String,
}

/// Public detail payload — a listing plus the short-lived presigned tarball URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceListingDetail {
    #[serde(flatten)]
    pub listing: MarketplaceListing,
    /// Presigned GET URL for the pack tarball (24h expiry). Only the detail
    /// route returns this; absent on browse rows.
    #[serde(default)]
    pub download_url: String,
}

// ---- response envelopes -----------------------------------------------------

#[derive(Debug, Deserialize)]
struct BrowseEnvelope {
    #[serde(default)]
    listings: Vec<MarketplaceListing>,
}

#[derive(Debug, Deserialize)]
struct DetailEnvelope {
    listing: MarketplaceListingDetail,
}

// ---- base URL ---------------------------------------------------------------

/// Resolve the vault API base, trimming any trailing slash. Reuses the same
/// resolver as the sync pipeline and the desktop-alt Board/Activity readers.
fn api_base() -> Result<String, String> {
    resolve_vault_api_url().map(|u| u.trim_end_matches('/').to_string())
}

/// Reject a listing id that isn't a clean URL-path segment (defense-in-depth so
/// a crafted id can't append query params or escape the path).
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

// ---- commands ---------------------------------------------------------------

/// Browse approved marketplace listings. Public route — NO auth token attached.
/// An optional `query` is forwarded as `?q=` so the server filters server-side
/// (the UI also filters client-side over the returned set for instant feedback).
#[tauri::command]
pub async fn list_marketplace_listings(
    query: Option<String>,
) -> Result<Vec<MarketplaceListing>, String> {
    let base = api_base()?;
    let mut url = format!("{base}/v1/listings");
    if let Some(q) = query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        url = format!("{url}?q={}", urlencoding_encode(q));
    }

    let res = build_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("listings fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("listings read: {e}"))?;

    parse_browse_response(status, &text)
}

/// Fetch one approved listing's public detail (incl. the presigned download
/// URL). Public route — NO auth token attached.
#[tauri::command]
pub async fn get_marketplace_listing(id: String) -> Result<MarketplaceListingDetail, String> {
    let id = id.trim();
    if !is_safe_id(id) {
        return Err(format!("invalid listing id: {id:?}"));
    }
    let base = api_base()?;
    let url = format!("{base}/v1/listings/{id}");

    let res = build_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("listing fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("listing read: {e}"))?;

    parse_detail_response(status, &text)
}

// ---- pure parsers (status + body → typed result) ---------------------------

fn parse_browse_response(status: StatusCode, text: &str) -> Result<Vec<MarketplaceListing>, String> {
    if status == StatusCode::NO_CONTENT {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        return Err(format!("listings HTTP {status}: {text}"));
    }
    let body = text.trim();
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let env: BrowseEnvelope = serde_json::from_str(body)
        .map_err(|e| format!("listings response is not valid JSON: {e}"))?;
    Ok(env.listings)
}

fn parse_detail_response(
    status: StatusCode,
    text: &str,
) -> Result<MarketplaceListingDetail, String> {
    if status == StatusCode::NOT_FOUND {
        return Err("listing not found".to_string());
    }
    if !status.is_success() {
        return Err(format!("listing HTTP {status}: {text}"));
    }
    let env: DetailEnvelope = serde_json::from_str(text.trim())
        .map_err(|e| format!("listing response is not valid JSON: {e}"))?;
    Ok(env.listing)
}

/// Minimal percent-encoder for a `?q=` value (no extra crate dependency). Encodes
/// everything that isn't an unreserved URL char so a search term with spaces or
/// `&`/`=`/`#` can't break the query string.
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// =============================================================================
// US-009 — install-to-personal-or-company (tenant-isolated scope picker)
// =============================================================================
//
// The detail slide-over's Install action shells out to the `hq` CLI to install a
// marketplace pack, EITHER into the operator's personal scope OR under a specific
// company's `companies/{co}/` directory (so the company's existing hq-sync fans
// the pack out to teammates). The security crux of this story lives HERE, in
// Rust — never trust the UI:
//
//   1. Admin gate (default-deny). A company-scoped install re-checks, against the
//      vault membership truth (`GET /membership/person/{uid}`), that the caller
//      is an ADMIN/OWNER of THAT company. UI disabling is convenience only; this
//      backend gate is the real authority. Unknown/missing role → denied.
//
//   2. Path containment. The company install target is resolved to an absolute,
//      symlink-canonicalised path and asserted to be UNDER `companies/{co}/`. A
//      crafted slug that tries to escape (`../other-co`, absolute, `..`) is
//      rejected. No cross-company write is possible from this command.
//
//   3. Hook-consent is NOT bypassed. We deliberately DO NOT pass `--allow-hooks`
//      to `hq install`, so the CLI's per-machine hook-consent gate fires on THIS
//      machine, and — because a company-scoped pack rides hq-sync to teammates —
//      that same gate RE-FIRES on each teammate's machine when their scan/wire
//      path encounters the synced pack (AC5). Cross-company credential isolation
//      (AC6) is enforced by HQ's existing per-company credential isolation: a
//      hook/script under `companies/{co}/` runs in that company's scope and
//      cannot read another company's vault/creds. Our job is to keep the pack
//      confined to the company prefix (containment above) and not weaken that
//      isolation. See `core/policies/credential-access-protocol.md`.

/// Where an install lands. `Personal` → the operator's personal overlay (CLI
/// default scope). `Company { slug }` → under `companies/{slug}/`, distributed
/// to teammates by the company's existing hq-sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InstallScope {
    Personal,
    Company { slug: String },
}

/// Roles that grant company-admin authority for a company-scoped install.
/// Anything else (member, viewer, pending, unknown, absent) is default-DENIED.
fn role_is_admin(role: Option<&str>) -> bool {
    matches!(
        role.map(|r| r.trim().to_ascii_lowercase()).as_deref(),
        Some("admin") | Some("owner")
    )
}

/// Validate a company slug as a single clean path segment (defense-in-depth so a
/// crafted slug can't escape the `companies/` prefix). No separators, no `..`,
/// no leading dot, ascii-lowercase / digit / `-` / `_` only.
fn is_safe_company_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 128
        && !slug.starts_with('.')
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
}

/// Build the `marketplace:<slug>[@version]` source string the CLI takes, after
/// validating both halves so neither can inject CLI args or path tricks.
fn marketplace_source(slug: &str, version: Option<&str>) -> Result<String, String> {
    let slug = slug.trim();
    if !is_safe_id(slug) {
        return Err(format!("invalid pack slug: {slug:?}"));
    }
    match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => {
            // Versions are semver-ish: digits, dots, dashes, plus, alnum.
            if !v
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
                || v.len() > 64
            {
                return Err(format!("invalid pack version: {v:?}"));
            }
            Ok(format!("marketplace:{slug}@{v}"))
        }
        None => Ok(format!("marketplace:{slug}")),
    }
}

/// Resolve the absolute company-install directory and ASSERT it is contained
/// within `<hq_root>/companies/`. Returns the canonical company dir on success.
///
/// Containment is the cross-company-isolation guarantee: even if some upstream
/// check were bypassed, the resolved target can never point at another company's
/// tree or anywhere outside `companies/`. Mirrors the `is_within` thinking used
/// by the desktop-alt local readers and the US-020 safe-extraction work.
fn resolve_company_dir(hq_root: &Path, slug: &str) -> Result<PathBuf, String> {
    if !is_safe_company_slug(slug) {
        return Err(format!("invalid company slug: {slug:?}"));
    }
    let companies = hq_root.join("companies");
    let target = companies.join(slug);

    // Reject any traversal component up front (belt-and-suspenders before any FS
    // canonicalisation, which may not exist yet for a brand-new company dir).
    if target
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(format!("company path escapes companies/: {slug:?}"));
    }

    // Canonicalise the `companies/` root (it always exists in a real HQ tree) and
    // verify the *lexical* target sits under it. We canonicalise the parent, not
    // the target itself, because the company dir may not exist before install.
    let companies_canon = companies
        .canonicalize()
        .unwrap_or_else(|_| companies.clone());
    let target_lexical = companies_canon.join(slug);
    if !target_lexical.starts_with(&companies_canon) {
        return Err(format!(
            "resolved company target {} is not under {}",
            target_lexical.display(),
            companies_canon.display()
        ));
    }
    // If the dir already exists, canonicalise and re-check (catches a symlinked
    // company dir that points outside the tree — cross-company escape via link).
    if target_lexical.exists() {
        let real = target_lexical
            .canonicalize()
            .map_err(|e| format!("canonicalize company dir: {e}"))?;
        if !real.starts_with(&companies_canon) {
            return Err(format!(
                "company dir {} resolves outside companies/ ({})",
                slug,
                real.display()
            ));
        }
        return Ok(real);
    }
    Ok(target_lexical)
}

/// Resolve the user's HQ folder (same 4-tier resolver every CLI-spawning command
/// uses — mirrors `packages.rs::resolve_hq_folder`).
fn resolve_hq_folder() -> PathBuf {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    )
}

/// Verify, against vault membership truth, that the signed-in operator is an
/// admin/owner of `company_slug`. Default-deny: any failure to positively
/// confirm an admin/owner membership for that exact company → error.
async fn assert_company_admin(company_slug: &str) -> Result<(), String> {
    let vault_url = resolve_vault_api_url()?;
    let jwt = resolve_jwt().await?;
    let vault = VaultClient::new(&vault_url, &jwt);

    // Find the company entity by slug, then confirm the caller has an
    // admin/owner membership for that entity's uid.
    let entity = vault
        .find_entity_by_slug("company", company_slug)
        .await
        .map_err(|e| format!("resolve company '{company_slug}': {e}"))?
        .ok_or_else(|| format!("company '{company_slug}' not found in your cloud"))?;
    if entity.deleted {
        return Err(format!("company '{company_slug}' is deleted"));
    }

    // Resolve the caller's own person entity (oldest = canonical, same heuristic
    // as workspaces.rs), then list that person's memberships.
    let mut persons = vault
        .list_entities_by_type("person")
        .await
        .map_err(|e| format!("list person entities: {e}"))?;
    persons.sort_by(|a, b| match a.created_at.cmp(&b.created_at) {
        std::cmp::Ordering::Equal => a.uid.cmp(&b.uid),
        ord => ord,
    });
    let person = persons
        .into_iter()
        .next()
        .ok_or_else(|| "no person entity for the signed-in user".to_string())?;

    let memberships = vault
        .list_memberships(&person.uid)
        .await
        .map_err(|e| format!("list memberships: {e}"))?;

    let admin = memberships.iter().any(|m| {
        m.company_uid == entity.uid
            && m.status.eq_ignore_ascii_case("active")
            && role_is_admin(m.role.as_deref())
    });
    if !admin {
        return Err(format!(
            "company install requires company-admin: you are not an admin of '{company_slug}'"
        ));
    }
    Ok(())
}

/// Build the `hq install` argv for a given source + scope.
///
/// Personal → `hq install <source>` (CLI default scope).
/// Company  → `hq install <source> --company <slug>` (lands under companies/<slug>/).
///
/// We INTENTIONALLY never add `--allow-hooks`: the hook-consent gate must fire on
/// this machine (AC4) and re-fire on each teammate's machine when the synced
/// company pack is wired (AC5). Adding `--allow-hooks` would silently auto-wire
/// hooks — exactly the supply-chain amplification this story forbids.
fn install_argv(source: &str, scope: &InstallScope) -> Vec<String> {
    let mut argv = vec!["install".to_string(), source.to_string()];
    if let InstallScope::Company { slug } = scope {
        argv.push("--company".to_string());
        argv.push(slug.clone());
    }
    argv
}

/// Install a marketplace pack into the chosen scope. Streams `hq install` output
/// to the window as `marketplace:install-progress` lines, terminating with
/// `marketplace:install-complete` / `marketplace:install-error`.
///
/// Security (enforced here, not in the UI):
///   * Company scope re-verifies admin via vault membership truth (default-deny).
///   * Company target is contained to `companies/{co}/` (no cross-company write).
///   * Hook-consent is preserved (no `--allow-hooks`) so wiring is never silent —
///     including on teammates' machines after the pack syncs.
#[tauri::command]
pub async fn install_marketplace_pack(
    app: AppHandle,
    slug: String,
    version: Option<String>,
    scope: InstallScope,
) -> Result<(), String> {
    let source = marketplace_source(&slug, version.as_deref())?;
    let hq_root = resolve_hq_folder();

    // ---- Security gate: company scope requires admin + path containment ----
    if let InstallScope::Company { slug: co } = &scope {
        // (a) Path containment FIRST (cheap, no network) — reject a malformed or
        // escaping slug before we ever touch the vault or spawn a process.
        let company_dir = resolve_company_dir(&hq_root, co)?;
        log(
            "marketplace",
            &format!("company install target contained at {}", company_dir.display()),
        );
        // (b) Admin gate against vault truth — default-deny on any failure.
        assert_company_admin(co).await?;
        log(
            "marketplace",
            &format!("admin gate passed for company '{co}' install of {source}"),
        );
    }

    let argv = install_argv(&source, &scope);
    stream_install(&app, &source, &scope, &hq_root, argv).await
}

/// Spawn `hq install …`, relaying stdout/stderr as progress events and a terminal
/// complete/error event. Hook-consent prompts the CLI emits flow through as
/// progress lines so the UI can surface them.
async fn stream_install(
    app: &AppHandle,
    source: &str,
    scope: &InstallScope,
    hq_root: &Path,
    argv: Vec<String>,
) -> Result<(), String> {
    let hq = paths::resolve_bin("hq");
    let scope_label = match scope {
        InstallScope::Personal => "personal".to_string(),
        InstallScope::Company { slug } => format!("company:{slug}"),
    };
    log(
        "marketplace",
        &format!("install `hq {}` (scope={scope_label})", argv.join(" ")),
    );

    let mut child = tokio::process::Command::new(&hq)
        .args(&argv)
        // node-shebang PATH fix — same as packages.rs.
        .env("PATH", paths::child_path())
        .current_dir(hq_root)
        .env("HQ_NO_UPDATE_CHECK", "1")
        .env("HQ_ROOT", hq_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn `hq {}`: {e}", argv.join(" ")))?;

    let emit_line = |app: &AppHandle, source: &str, scope_label: &str, line: String| {
        let _ = app.emit(
            "marketplace:install-progress",
            serde_json::json!({ "source": source, "scope": scope_label, "line": line }),
        );
    };

    if let Some(out) = child.stdout.take() {
        let app = app.clone();
        let source = source.to_string();
        let scope_label = scope_label.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                emit_line(&app, &source, &scope_label, line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let app = app.clone();
        let source = source.to_string();
        let scope_label = scope_label.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                emit_line(&app, &source, &scope_label, line);
            }
        });
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("await `hq {}`: {e}", argv.join(" ")))?;

    if status.success() {
        let _ = app.emit(
            "marketplace:install-complete",
            serde_json::json!({ "source": source, "scope": scope_label }),
        );
        Ok(())
    } else {
        let msg = format!(
            "`hq {}` exited {}",
            argv.join(" "),
            status.code().unwrap_or(-1)
        );
        let _ = app.emit(
            "marketplace:install-error",
            serde_json::json!({ "source": source, "scope": scope_label, "message": msg }),
        );
        Err(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_parses_listings_envelope() {
        let body = r#"{"listings":[
            {"id":"lst_1","type":"skill","name":"Impeccable","slug":"impeccable",
             "version":"1.2.0","author":"corey","summary":"Improve a UI",
             "contributes":"1 skill","createdAt":"2026-06-01T00:00:00Z"},
            {"id":"lst_2","type":"worker","name":"Architect","slug":"architect",
             "version":"0.1.0","author":"jane","createdAt":"2026-06-02T00:00:00Z"}
        ]}"#;
        let listings = parse_browse_response(StatusCode::OK, body).expect("parsed");
        assert_eq!(listings.len(), 2);

        let first = &listings[0];
        assert_eq!(first.id, "lst_1");
        assert_eq!(first.type_, "skill");
        assert_eq!(first.name, "Impeccable");
        assert_eq!(first.author, "corey");
        assert_eq!(first.version, "1.2.0");
        assert_eq!(first.summary.as_deref(), Some("Improve a UI"));
        assert_eq!(first.contributes.as_deref(), Some("1 skill"));

        // Optional fields absent → None, still parses.
        let second = &listings[1];
        assert_eq!(second.author, "jane");
        assert!(second.summary.is_none());
        assert!(second.contributes.is_none());
    }

    #[test]
    fn browse_handles_empty_and_no_content() {
        assert!(parse_browse_response(StatusCode::NO_CONTENT, "")
            .unwrap()
            .is_empty());
        assert!(parse_browse_response(StatusCode::OK, "  \n ")
            .unwrap()
            .is_empty());
        assert!(parse_browse_response(StatusCode::OK, r#"{"listings":[]}"#)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn browse_surfaces_http_errors() {
        let err = parse_browse_response(StatusCode::INTERNAL_SERVER_ERROR, "boom").unwrap_err();
        assert!(err.contains("500"));
        assert!(parse_browse_response(StatusCode::OK, "{not json").is_err());
    }

    #[test]
    fn detail_parses_listing_with_download_url() {
        let body = r#"{"listing":{"id":"lst_1","type":"skill","name":"Impeccable",
            "slug":"impeccable","version":"1.2.0","author":"corey",
            "contributes":"1 skill","createdAt":"2026-06-01T00:00:00Z",
            "downloadUrl":"https://example.com/pack.tar.gz?sig=abc"}}"#;
        let detail = parse_detail_response(StatusCode::OK, body).expect("parsed");
        assert_eq!(detail.listing.id, "lst_1");
        assert_eq!(detail.listing.author, "corey");
        assert_eq!(detail.listing.contributes.as_deref(), Some("1 skill"));
        assert_eq!(detail.download_url, "https://example.com/pack.tar.gz?sig=abc");
    }

    #[test]
    fn detail_maps_404_and_errors() {
        assert_eq!(
            parse_detail_response(StatusCode::NOT_FOUND, "").unwrap_err(),
            "listing not found"
        );
        assert!(parse_detail_response(StatusCode::BAD_GATEWAY, "x")
            .unwrap_err()
            .contains("502"));
    }

    #[test]
    fn id_safety_rejects_path_tricks() {
        assert!(is_safe_id("lst_abc-123.v1"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("lst/../secret"));
        assert!(!is_safe_id("lst?q=1"));
        assert!(!is_safe_id("lst 1"));
        assert!(!is_safe_id(&"a".repeat(300)));
    }

    #[test]
    fn query_is_percent_encoded() {
        assert_eq!(urlencoding_encode("hello world"), "hello%20world");
        assert_eq!(urlencoding_encode("a&b=c#d"), "a%26b%3Dc%23d");
        assert_eq!(urlencoding_encode("safe-_.~"), "safe-_.~");
    }

    // ---- US-009: install scope / tenant-isolation security tests -----------

    #[test]
    fn admin_role_gate_is_default_deny() {
        // Only admin/owner (case-insensitive, trimmed) grant company-install.
        assert!(role_is_admin(Some("admin")));
        assert!(role_is_admin(Some("owner")));
        assert!(role_is_admin(Some("  Admin ")));
        assert!(role_is_admin(Some("OWNER")));
        // Everything else is denied — including the unknown/absent case
        // (default-deny: a missing or unrecognized role is NOT an admin).
        assert!(!role_is_admin(Some("member")));
        assert!(!role_is_admin(Some("viewer")));
        assert!(!role_is_admin(Some("pending")));
        assert!(!role_is_admin(Some("administrator-ish")));
        assert!(!role_is_admin(Some("")));
        assert!(!role_is_admin(None));
    }

    #[test]
    fn company_slug_safety_rejects_path_tricks() {
        assert!(is_safe_company_slug("indigo"));
        assert!(is_safe_company_slug("acme-co_2"));
        // Escapes / separators / case / dot-leading are all rejected.
        assert!(!is_safe_company_slug(""));
        assert!(!is_safe_company_slug(".."));
        assert!(!is_safe_company_slug("../other"));
        assert!(!is_safe_company_slug("a/b"));
        assert!(!is_safe_company_slug("a\\b"));
        assert!(!is_safe_company_slug(".hidden"));
        assert!(!is_safe_company_slug("Indigo")); // uppercase not allowed
        assert!(!is_safe_company_slug("co with space"));
        assert!(!is_safe_company_slug(&"a".repeat(200)));
    }

    #[test]
    fn company_dir_is_contained_under_companies() {
        let tmp = std::env::temp_dir().join(format!("hq-mk-test-{}", std::process::id()));
        let companies = tmp.join("companies");
        std::fs::create_dir_all(companies.join("indigo")).unwrap();

        // Happy path: resolves under <root>/companies/indigo.
        let dir = resolve_company_dir(&tmp, "indigo").expect("contained");
        assert!(dir.ends_with("companies/indigo"));
        let companies_canon = companies.canonicalize().unwrap();
        assert!(dir.starts_with(&companies_canon));

        // A not-yet-existing (but well-formed) company is allowed — install
        // creates it — and still resolves under companies/.
        let fresh = resolve_company_dir(&tmp, "newco").expect("fresh contained");
        assert!(fresh.ends_with("companies/newco"));
        assert!(fresh.starts_with(&companies_canon));

        // Path-escape attempts are rejected (no cross-company / outside write).
        assert!(resolve_company_dir(&tmp, "../etc").is_err());
        assert!(resolve_company_dir(&tmp, "..").is_err());
        assert!(resolve_company_dir(&tmp, "foo/bar").is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn company_dir_symlink_escape_is_rejected() {
        // A company dir that is a symlink pointing OUTSIDE companies/ (e.g. at
        // another company's tree or anywhere on disk) must be rejected so a
        // company-scoped install can never write across the tenant boundary.
        let tmp = std::env::temp_dir().join(format!("hq-mk-symlink-{}", std::process::id()));
        let companies = tmp.join("companies");
        std::fs::create_dir_all(&companies).unwrap();
        let outside = tmp.join("outside-the-tree");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, companies.join("evil")).unwrap();

        let err = resolve_company_dir(&tmp, "evil").unwrap_err();
        assert!(err.contains("outside companies/"), "got: {err}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn marketplace_source_validates_and_formats() {
        assert_eq!(
            marketplace_source("impeccable", None).unwrap(),
            "marketplace:impeccable"
        );
        assert_eq!(
            marketplace_source("impeccable", Some("1.2.0")).unwrap(),
            "marketplace:impeccable@1.2.0"
        );
        // Empty/whitespace version → no @ suffix.
        assert_eq!(
            marketplace_source("impeccable", Some("  ")).unwrap(),
            "marketplace:impeccable"
        );
        // Crafted slug / version that could inject args or path tricks is rejected.
        assert!(marketplace_source("a b", None).is_err());
        assert!(marketplace_source("../x", None).is_err());
        assert!(marketplace_source("ok", Some("1.0 --allow-hooks")).is_err());
        assert!(marketplace_source("ok", Some("a/b")).is_err());
    }

    #[test]
    fn personal_install_argv_has_no_company_flag() {
        let argv = install_argv("marketplace:impeccable", &InstallScope::Personal);
        assert_eq!(argv, vec!["install", "marketplace:impeccable"]);
    }

    #[test]
    fn company_install_argv_targets_company_dir() {
        let argv = install_argv(
            "marketplace:impeccable@1.2.0",
            &InstallScope::Company {
                slug: "indigo".into(),
            },
        );
        assert_eq!(
            argv,
            vec![
                "install",
                "marketplace:impeccable@1.2.0",
                "--company",
                "indigo"
            ]
        );
    }

    #[test]
    fn install_never_bypasses_hook_consent() {
        // AC4/AC5: the hook-consent gate must fire on this machine AND re-fire on
        // each teammate's machine when the synced company pack is wired. We must
        // NEVER pass --allow-hooks (which would auto-wire hooks silently). Assert
        // it's absent from BOTH scopes' argv.
        let personal = install_argv("marketplace:hooky", &InstallScope::Personal);
        let company = install_argv(
            "marketplace:hooky",
            &InstallScope::Company {
                slug: "indigo".into(),
            },
        );
        assert!(
            !personal.iter().any(|a| a == "--allow-hooks"),
            "personal install must not bypass hook consent"
        );
        assert!(
            !company.iter().any(|a| a == "--allow-hooks"),
            "company install must not bypass hook consent (no silent code push)"
        );
    }

    #[test]
    fn install_scope_serde_roundtrip() {
        // The UI sends a tagged InstallScope; confirm both variants parse.
        let personal: InstallScope = serde_json::from_str(r#"{"kind":"personal"}"#).unwrap();
        assert_eq!(personal, InstallScope::Personal);
        let company: InstallScope =
            serde_json::from_str(r#"{"kind":"company","slug":"indigo"}"#).unwrap();
        assert_eq!(
            company,
            InstallScope::Company {
                slug: "indigo".into()
            }
        );
    }

    #[test]
    fn synced_company_pack_re_fires_consent_on_teammates() {
        // AC5 (re-consent on teammate sync) seam test.
        //
        // A company-scoped pack rides hq-sync to teammates as files UNDER
        // `companies/{co}/`. The guarantee that hooks aren't auto-wired silently
        // on a teammate's machine has two halves, both anchored here:
        //
        //   1. We never pass `--allow-hooks` (see install_never_bypasses_hook_consent)
        //      so the install/wire path keeps its hook-consent gate.
        //   2. Per-machine consent / hook-wiring state lives under the company's
        //      `settings/` (and host-side `.claude/hooks/`), which hq-sync does
        //      NOT distribute. So a teammate's machine can NEVER inherit an
        //      "already consented" marker — it must re-run the consent gate before
        //      wiring any hook/script from the synced pack.
        //
        // This test pins half (2): assert the consent-bearing class is not synced
        // while the pack's own content (skills/workers) is.
        use crate::util::ignore::IgnoreFilter;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("companies")).unwrap();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();

        // Per-company settings (where consent / wiring state lives) must NOT sync.
        assert!(
            !filter.should_sync(&root.join("companies/indigo/settings/hook-consent.json")),
            "company consent/wiring state must not ride hq-sync to teammates"
        );
        // The marketplace pack's actual content DOES sync (that's the whole point
        // of company scope) — but it lands as inert files until the teammate's
        // consent-gated wire path runs.
        assert!(
            filter.should_sync(&root.join("companies/indigo/skills/impeccable/SKILL.md")),
            "company-scoped pack content should sync to teammates"
        );
    }
}
