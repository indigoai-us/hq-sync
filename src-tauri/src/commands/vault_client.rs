use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum VaultClientError {
    Request(reqwest::Error),
    Http { status: u16, body: String },
    Json(String),
    /// 403 with body `{"code":"SELF_OWNERSHIP_MISMATCH"}` from POST /sts/vend-self.
    SelfOwnershipMismatch,
}

impl std::fmt::Display for VaultClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(e) => write!(f, "transport error: {e}"),
            Self::Http { status, body } => write!(f, "HTTP {status}: {body}"),
            Self::Json(msg) => write!(f, "JSON error: {msg}"),
            Self::SelfOwnershipMismatch => write!(f, "403 SELF_OWNERSHIP_MISMATCH"),
        }
    }
}

impl std::error::Error for VaultClientError {}

impl From<reqwest::Error> for VaultClientError {
    fn from(e: reqwest::Error) -> Self {
        Self::Request(e)
    }
}

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityInfo {
    pub uid: String,
    pub slug: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: Option<String>,
    pub bucket_name: Option<String>,
    pub status: String,
    /// Non-optional: server always writes `createdAt: now` on every createEntity.
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEntityInput {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketInfo {
    pub bucket_name: String,
    pub kms_key_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskScope {
    pub allowed_prefixes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_actions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendChildInput {
    pub company_uid: String,
    pub task_id: String,
    pub task_description: String,
    pub task_scope: TaskScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendChildCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendChildResult {
    pub credentials: VendChildCredentials,
    pub session_name: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembershipInfo {
    /// Synthetic identifier — the vault API does NOT return a top-level
    /// `uid` for memberships (the canonical identifier is `membership_key`,
    /// the composite `personUid#companyUid`). Kept here as `#[serde(default)]`
    /// so legacy callers/tests that still mock a uid field continue to work.
    /// For logging / display, prefer `display_id()`.
    #[serde(default)]
    pub uid: String,
    pub person_uid: String,
    pub company_uid: String,
    /// Server-side: "active" | "pending" | (future).
    pub status: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    /// Composite key from the API (`{personUid}#{companyUid}`). Always
    /// returned by the live vault; absent in some test fixtures.
    #[serde(default)]
    pub membership_key: Option<String>,
}

impl MembershipInfo {
    /// Best identifier for log lines and error messages. Falls back through
    /// `uid` → `membership_key` → synthesized composite, so we always have
    /// SOMETHING meaningful to show.
    pub fn display_id(&self) -> String {
        if !self.uid.is_empty() {
            self.uid.clone()
        } else if let Some(k) = &self.membership_key {
            k.clone()
        } else {
            format!("{}#{}", self.person_uid, self.company_uid)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendSelfInput {
    pub person_uid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendSelfCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    #[serde(default)]
    pub expiration: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendSelfResult {
    pub credentials: VendSelfCredentials,
    pub expires_at: String,
}

// ── Telemetry types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryOptInResponse {
    pub enabled: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBatch {
    pub machine_id: String,
    pub installer_version: String,
    pub events: Vec<serde_json::Value>,
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct VaultClient {
    base_url: String,
    auth_token: String,
    client: Client,
}

impl VaultClient {
    pub fn new(base_url: impl Into<String>, auth_token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            auth_token: auth_token.into(),
            client: Client::new(),
        }
    }

    /// `POST /entity` — create a new entity; returns the created EntityInfo.
    pub async fn create_entity(
        &self,
        input: &CreateEntityInput,
    ) -> Result<EntityInfo, VaultClientError> {
        let resp = self
            .client
            .post(format!("{}/entity", self.base_url))
            .bearer_auth(&self.auth_token)
            .json(input)
            .send()
            .await?;
        let wrapper: serde_json::Value = self.handle_response(resp).await?;
        serde_json::from_value(wrapper["entity"].clone())
            .map_err(|e| VaultClientError::Json(e.to_string()))
    }

    /// `GET /entity/by-type/{type}` — list all entities of the given type owned by the caller.
    pub async fn list_entities_by_type(
        &self,
        entity_type: &str,
    ) -> Result<Vec<EntityInfo>, VaultClientError> {
        let resp = self
            .client
            .get(format!("{}/entity/by-type/{}", self.base_url, entity_type))
            .bearer_auth(&self.auth_token)
            .send()
            .await?;
        let wrapper: serde_json::Value = self.handle_response(resp).await?;
        serde_json::from_value(wrapper["entities"].clone())
            .map_err(|e| VaultClientError::Json(e.to_string()))
    }

    /// `GET /entity/by-slug/{type}/{slug}` — find a single entity by its type + slug.
    /// Returns `None` on 404; `Err` on any other non-2xx.
    pub async fn find_entity_by_slug(
        &self,
        entity_type: &str,
        slug: &str,
    ) -> Result<Option<EntityInfo>, VaultClientError> {
        let resp = self
            .client
            .get(format!(
                "{}/entity/by-slug/{}/{}",
                self.base_url, entity_type, slug
            ))
            .bearer_auth(&self.auth_token)
            .send()
            .await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        let wrapper: serde_json::Value = self.handle_response(resp).await?;
        serde_json::from_value(wrapper["entity"].clone())
            .map(Some)
            .map_err(|e| VaultClientError::Json(e.to_string()))
    }

    /// `GET /entity/{uid}` — fetch a single entity by its UID. Returns `None` on 404.
    ///
    /// Used by the workspaces command to resolve memberships (which carry only
    /// `companyUid`) into full Company entities (`name`, `slug`, `bucketName`).
    pub async fn find_entity_by_uid(
        &self,
        uid: &str,
    ) -> Result<Option<EntityInfo>, VaultClientError> {
        let resp = self
            .client
            .get(format!("{}/entity/{}", self.base_url, uid))
            .bearer_auth(&self.auth_token)
            .send()
            .await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        let wrapper: serde_json::Value = self.handle_response(resp).await?;
        serde_json::from_value(wrapper["entity"].clone())
            .map(Some)
            .map_err(|e| VaultClientError::Json(e.to_string()))
    }

    /// `GET /membership/person/{personUid}` — list memberships for a person.
    ///
    /// Returns all memberships (`active` + `pending`); the caller filters as
    /// needed. Empty list is a valid result (signed-in user with no companies).
    pub async fn list_memberships(
        &self,
        person_uid: &str,
    ) -> Result<Vec<MembershipInfo>, VaultClientError> {
        let resp = self
            .client
            .get(format!(
                "{}/membership/person/{}",
                self.base_url, person_uid
            ))
            .bearer_auth(&self.auth_token)
            .send()
            .await?;
        let wrapper: serde_json::Value = self.handle_response(resp).await?;
        serde_json::from_value(wrapper["memberships"].clone())
            .map_err(|e| VaultClientError::Json(e.to_string()))
    }

    /// `POST /provision/bucket` — provision (or idempotently confirm) an S3 bucket for `uid`.
    pub async fn provision_bucket(&self, uid: &str) -> Result<BucketInfo, VaultClientError> {
        let body = serde_json::json!({ "companyUid": uid });
        let resp = self
            .client
            .post(format!("{}/provision/bucket", self.base_url))
            .bearer_auth(&self.auth_token)
            .json(&body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    /// `POST /sts/vend-child` — vend task-scoped child credentials for a company entity (`cmp_*`).
    pub async fn vend_child(
        &self,
        input: &VendChildInput,
    ) -> Result<VendChildResult, VaultClientError> {
        let resp = self
            .client
            .post(format!("{}/sts/vend-child", self.base_url))
            .bearer_auth(&self.auth_token)
            .json(input)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    /// `GET /v1/usage/opt-in` — check whether the authenticated user has opted in to telemetry.
    pub async fn get_telemetry_opt_in(&self) -> Result<TelemetryOptInResponse, VaultClientError> {
        let resp = self
            .client
            .get(format!("{}{}", self.base_url, "/v1/usage/opt-in"))
            .bearer_auth(&self.auth_token)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    /// `POST /v1/usage` — upload a batch of sanitized telemetry events.
    ///
    /// Body: `{ machineId, installerVersion, events }` — no `personUid` (resolved server-side).
    /// Returns `()` on 200; any non-2xx becomes `VaultClientError::Http`.
    pub async fn post_usage(&self, batch: &UsageBatch) -> Result<(), VaultClientError> {
        let resp = self
            .client
            .post(format!("{}{}", self.base_url, "/v1/usage"))
            .bearer_auth(&self.auth_token)
            .json(batch)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(VaultClientError::Http {
                status: status.as_u16(),
                body,
            });
        }
        Ok(())
    }

    /// `POST /sts/vend-self` — vend full-access credentials for the caller's own person entity (`prs_*`).
    ///
    /// Returns `VaultClientError::SelfOwnershipMismatch` when the server responds with
    /// 403 + `{"code":"SELF_OWNERSHIP_MISMATCH"}`.
    pub async fn vend_self(
        &self,
        input: &VendSelfInput,
    ) -> Result<VendSelfResult, VaultClientError> {
        let resp = self
            .client
            .post(format!("{}{}", self.base_url, "/sts/vend-self"))
            .bearer_auth(&self.auth_token)
            .json(input)
            .send()
            .await?;

        let status = resp.status();
        let body_text = resp.text().await?;

        if status.as_u16() == 403 {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body_text) {
                if v.get("code").and_then(|c| c.as_str()) == Some("SELF_OWNERSHIP_MISMATCH") {
                    return Err(VaultClientError::SelfOwnershipMismatch);
                }
            }
            return Err(VaultClientError::Http {
                status: 403,
                body: body_text,
            });
        }

        if !status.is_success() {
            return Err(VaultClientError::Http {
                status: status.as_u16(),
                body: body_text,
            });
        }

        serde_json::from_str(&body_text).map_err(|e| VaultClientError::Json(e.to_string()))
    }

    async fn handle_response<T: DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, VaultClientError> {
        let status = resp.status();
        let body_text = resp.text().await?;
        if !status.is_success() {
            return Err(VaultClientError::Http {
                status: status.as_u16(),
                body: body_text,
            });
        }
        serde_json::from_str(&body_text).map_err(|e| VaultClientError::Json(e.to_string()))
    }
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

    #[tokio::test]
    async fn list_entities_by_type_roundtrip() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "entities": [{
                    "uid": "prs_x",
                    "slug": "a",
                    "type": "person",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .list_entities_by_type("person")
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uid, "prs_x");
        assert_eq!(result[0].created_at, "2026-01-01T00:00:00Z");
    }

    #[tokio::test]
    async fn find_entity_by_slug_some() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/company/newco"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "entity": {
                    "uid": "cmp_y", "slug": "newco", "type": "company",
                    "name": "NewCo", "status": "active", "createdAt": "2026-01-01T00:00:00Z"
                }
            })))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .find_entity_by_slug("company", "newco")
            .await
            .unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().uid, "cmp_y");
    }

    #[tokio::test]
    async fn find_entity_by_slug_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/company/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(&json!({"error": "not found"})))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .find_entity_by_slug("company", "missing")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn create_entity_roundtrip() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/entity"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "entity": {
                    "uid": "cmp_y", "slug": "newco", "type": "company",
                    "name": "NewCo", "status": "active", "createdAt": "2026-01-01T00:00:00Z"
                }
            })))
            .mount(&server)
            .await;

        let input = CreateEntityInput {
            entity_type: "company".into(),
            slug: "newco".into(),
            name: "NewCo".into(),
            email: None,
            owner_uid: None,
        };
        let result = client(&server.uri()).create_entity(&input).await.unwrap();
        assert_eq!(result.uid, "cmp_y");
        assert_eq!(result.slug, "newco");
    }

    #[tokio::test]
    async fn provision_bucket_idempotent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/provision/bucket"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "bucketName": "hq-vault-cmp-x",
                "kmsKeyId": "key-123"
            })))
            .mount(&server)
            .await;

        let result = client(&server.uri()).provision_bucket("cmp_x").await.unwrap();
        assert_eq!(result.bucket_name, "hq-vault-cmp-x");
        assert_eq!(result.kms_key_id, "key-123");
    }

    #[tokio::test]
    async fn vend_child_roundtrip() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sts/vend-child"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "credentials": {
                    "accessKeyId": "ASIA",
                    "secretAccessKey": "secret",
                    "sessionToken": "tok"
                },
                "sessionName": "prs_x--task--t",
                "expiresAt": "2026-01-01T01:00:00Z"
            })))
            .mount(&server)
            .await;

        let input = VendChildInput {
            company_uid: "cmp_x".into(),
            task_id: "t".into(),
            task_description: "test".into(),
            task_scope: TaskScope {
                allowed_prefixes: vec!["".into()],
                allowed_actions: Some(vec!["read".into(), "write".into()]),
            },
            duration_seconds: None,
        };
        let result = client(&server.uri()).vend_child(&input).await.unwrap();
        assert_eq!(result.credentials.access_key_id, "ASIA");
        assert_eq!(result.session_name, "prs_x--task--t");
        assert_eq!(result.expires_at, "2026-01-01T01:00:00Z");
    }

    #[tokio::test]
    async fn vend_self_roundtrip() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sts/vend-self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "credentials": {
                    "accessKeyId": "ASIA",
                    "secretAccessKey": "secret",
                    "sessionToken": "tok",
                    "expiration": "2026-01-01T01:00:00Z"
                },
                "expiresAt": "2026-01-01T01:00:00Z"
            })))
            .mount(&server)
            .await;

        let input = VendSelfInput {
            person_uid: "prs_x".into(),
            duration_seconds: None,
        };
        let result = client(&server.uri()).vend_self(&input).await.unwrap();
        assert_eq!(result.credentials.access_key_id, "ASIA");
        assert_eq!(result.expires_at, "2026-01-01T01:00:00Z");

        // assert wiremock received POST /sts/vend-self with body {"personUid":"prs_x"}
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["personUid"], "prs_x");
        assert!(
            body.get("durationSeconds").is_none(),
            "duration_seconds must not be serialized when None"
        );
    }

    #[tokio::test]
    async fn find_entity_by_uid_some() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/cmp_x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "entity": {
                    "uid": "cmp_x", "slug": "acme", "type": "company",
                    "name": "Acme", "bucketName": "hq-vault-cmp-x",
                    "status": "active", "createdAt": "2026-01-01T00:00:00Z"
                }
            })))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .find_entity_by_uid("cmp_x")
            .await
            .unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.uid, "cmp_x");
        assert_eq!(info.slug, "acme");
        assert_eq!(info.bucket_name.as_deref(), Some("hq-vault-cmp-x"));
    }

    #[tokio::test]
    async fn find_entity_by_uid_none_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/entity/cmp_missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(&json!({"error": "not found"})))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .find_entity_by_uid("cmp_missing")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_memberships_roundtrip() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/membership/person/prs_x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "memberships": [
                    {
                        "uid": "mem_1",
                        "personUid": "prs_x",
                        "companyUid": "cmp_a",
                        "status": "active",
                        "role": "owner",
                        "createdAt": "2026-01-01T00:00:00Z"
                    },
                    {
                        "uid": "mem_2",
                        "personUid": "prs_x",
                        "companyUid": "cmp_b",
                        "status": "pending"
                    }
                ]
            })))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .list_memberships("prs_x")
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].uid, "mem_1");
        assert_eq!(result[0].company_uid, "cmp_a");
        assert_eq!(result[0].status, "active");
        assert_eq!(result[0].role.as_deref(), Some("owner"));
        assert_eq!(result[1].status, "pending");
        // Optional fields tolerate omission.
        assert!(result[1].role.is_none());
        assert!(result[1].created_at.is_none());
    }

    /// Real vault response shape: no top-level `uid`, but includes
    /// `membershipKey`. The struct must deserialize cleanly without uid.
    /// This was the v0.1.24 cloud-unreachable bug — `serde(default)` on uid
    /// would NOT have flagged the missing field, but the OLD struct without
    /// the default would error out, propagating as "Cloud unreachable" in
    /// the menubar. Locks in the live API contract.
    #[tokio::test]
    async fn list_memberships_real_vault_shape_no_uid() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/membership/person/prs_x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "memberships": [
                    {
                        // Note: no top-level `uid` field — matches the
                        // production vault response.
                        "personUid": "prs_x",
                        "companyUid": "cmp_a",
                        "membershipKey": "prs_x#cmp_a",
                        "status": "active",
                        "role": "owner",
                        "createdAt": "2026-04-25T16:53:12.508Z",
                        "invitedAt": "2026-04-25T16:53:12.508Z",
                        "invitedBy": "prs_x",
                        "acceptedAt": "2026-04-25T16:53:13.088Z",
                        "updatedAt": "2026-04-25T16:53:13.088Z",
                        "schemaVersion": 1
                    }
                ]
            })))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .list_memberships("prs_x")
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].uid.is_empty(), "missing uid in API → empty string per serde(default)");
        assert_eq!(result[0].membership_key.as_deref(), Some("prs_x#cmp_a"));
        assert_eq!(result[0].company_uid, "cmp_a");
        // display_id() falls back to membership_key when uid is empty.
        assert_eq!(result[0].display_id(), "prs_x#cmp_a");
    }

    #[tokio::test]
    async fn list_memberships_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/membership/person/prs_lonely"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&json!({
                "memberships": []
            })))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .list_memberships("prs_lonely")
            .await
            .unwrap();
        assert!(result.is_empty(), "empty memberships are a valid signal, not an error");
    }

    #[tokio::test]
    async fn list_memberships_propagates_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/membership/person/prs_x"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let err = client(&server.uri())
            .list_memberships("prs_x")
            .await
            .expect_err("5xx must surface as Err");
        match err {
            VaultClientError::Http { status, .. } => assert_eq!(status, 500),
            other => panic!("expected Http(500), got {other}"),
        }
    }

    #[tokio::test]
    async fn vend_self_mismatch_403() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sts/vend-self"))
            .respond_with(ResponseTemplate::new(403).set_body_json(&json!({
                "error": "body.personUid does not match caller's canonical person entity",
                "code": "SELF_OWNERSHIP_MISMATCH"
            })))
            .mount(&server)
            .await;

        let input = VendSelfInput {
            person_uid: "prs_forged".into(),
            duration_seconds: None,
        };
        let err = client(&server.uri())
            .vend_self(&input)
            .await
            .expect_err("should fail with SelfOwnershipMismatch");
        assert!(
            matches!(err, VaultClientError::SelfOwnershipMismatch),
            "expected SelfOwnershipMismatch, got: {err}"
        );
    }
}
