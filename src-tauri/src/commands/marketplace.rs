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

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;

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
}
