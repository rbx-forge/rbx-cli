pub mod models;

use std::collections::BTreeMap;

use anyhow::Result;
use rbx_core::api::{execute_json, is_api_status, ApiBase};
use reqwest::{Client, StatusCode};
use serde_json::Value as Json;

use models::*;

const CONFIGS_PATH: &str = "/creator-configs-public-api/v1/configs/universes";
const REPOSITORY: &str = "InExperienceConfig";

pub struct RbxConfigClient {
    client: Client,
    api_key: String,
    /// Where the configs API lives.
    ///
    /// Injectable so the request shaping and the 404-means-empty rule can run
    /// against a mock server. It was a `const` with the host baked in, which
    /// is why this crate had no HTTP tests at all.
    base: ApiBase,
}

/// Hand-written rather than derived, because the derive would print
/// `api_key`, and a client is exactly the sort of value that ends up in a
/// `{:?}` inside an error context or a debug log.
impl std::fmt::Debug for RbxConfigClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RbxConfigClient")
            .field("base", &self.base)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl RbxConfigClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: rbx_core::api::build_client(),
            api_key,
            base: ApiBase::default(),
        }
    }

    /// Point the client at another host. Tests only, and compiled only for
    /// them: `pub(crate)` rather than `pub` so it cannot become a production
    /// code path outside this crate.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base = ApiBase::new(url);
        self
    }

    fn configs_url(&self) -> String {
        self.base.join(CONFIGS_PATH)
    }

    fn repo_url(&self, universe_id: u64) -> String {
        format!(
            "{}/{}/repositories/{}",
            self.configs_url(),
            universe_id,
            REPOSITORY
        )
    }

    // -------------------------------------------------------------------------
    // GET: live published config
    // -------------------------------------------------------------------------

    /// Fetch the live published config. Returns an empty snapshot on 404
    /// (no config published yet for this universe).
    pub async fn get_config(&self, universe_id: u64) -> Result<ConfigSnapshot> {
        let url = self.repo_url(universe_id);
        let api_key = self.api_key.clone();

        let result: Result<ConfigSnapshot> = execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await;

        match result {
            Ok(snapshot) => Ok(snapshot),
            // A universe that has never published a config answers 404. That
            // is the starting state, not a failure: `sync` creates the first
            // draft from it.
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => {
                Ok(ConfigSnapshot::default())
            }
            Err(error) => Err(error),
        }
    }

    // -------------------------------------------------------------------------
    // PUT /draft:overwrite: replace entire draft (handles deletions)
    // -------------------------------------------------------------------------

    pub async fn overwrite_draft(
        &self,
        universe_id: u64,
        entries: &BTreeMap<String, Json>,
        previous_draft_hash: Option<&str>,
    ) -> Result<DraftResult> {
        let url = format!("{}/draft:overwrite", self.repo_url(universe_id));
        let api_key = self.api_key.clone();
        let body = serde_json::to_string(&OverwriteBody {
            entries,
            previous_draft_hash,
        })?;

        execute_json(|| async {
            Ok(self
                .client
                .put(&url)
                .header("x-api-key", &api_key)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await?)
        })
        .await
    }

    // -------------------------------------------------------------------------
    // POST /publish
    // -------------------------------------------------------------------------

    pub async fn publish(
        &self,
        universe_id: u64,
        message: &str,
        strategy: &str,
        draft_hash: Option<&str>,
    ) -> Result<PublishResult> {
        let url = format!("{}/publish", self.repo_url(universe_id));
        let api_key = self.api_key.clone();
        let body = serde_json::to_string(&PublishBody {
            message,
            deployment_strategy: strategy,
            draft_hash,
        })?;

        execute_json(|| async {
            Ok(self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await?)
        })
        .await
    }

    // -------------------------------------------------------------------------
    // Convenience: overwrite draft + publish (used by sync)
    // -------------------------------------------------------------------------

    pub async fn overwrite_and_publish(
        &self,
        universe_id: u64,
        entries: &BTreeMap<String, Json>,
        message: &str,
        strategy: &str,
    ) -> Result<PublishResult> {
        let draft = self.overwrite_draft(universe_id, entries, None).await?;
        self.publish(universe_id, message, strategy, draft.draft_hash.as_deref())
            .await
    }

    // -------------------------------------------------------------------------
    // GET /revisions: list revision history
    // -------------------------------------------------------------------------

    pub async fn list_revisions(&self, universe_id: u64, max: usize) -> Result<Vec<RevisionEntry>> {
        let url = format!(
            "{}/revisions?MaxPageSize={}&SortOrder=SORT_ORDER_DESCENDING",
            self.repo_url(universe_id),
            max
        );
        let api_key = self.api_key.clone();

        let response: ListRevisionsResponse = execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await?;

        Ok(response.revisions)
    }

    // -------------------------------------------------------------------------
    // POST /revisions/{id}/restore: stage a revert to a revision
    // -------------------------------------------------------------------------

    pub async fn restore_revision(&self, universe_id: u64, revision_id: &str) -> Result<String> {
        let url = format!(
            "{}/revisions/{}:restore",
            self.repo_url(universe_id),
            revision_id
        );
        let api_key = self.api_key.clone();

        let response: RestoreResponse = execute_json(|| async {
            Ok(self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await?;

        Ok(response.draft_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 9876543210;
    const REPO_PATH: &str =
        "/creator-configs-public-api/v1/configs/universes/9876543210/repositories/InExperienceConfig";

    fn client(server: &MockServer) -> RbxConfigClient {
        RbxConfigClient::new("test-key".into()).with_base_url(server.uri())
    }

    /// The rule this crate is built on: a universe that has never published a
    /// config answers 404, and that is the starting state rather than a
    /// failure. It used to be read off the response inside a private retry
    /// loop; it is now read off the error's status, and this is what says the
    /// two behave the same.
    #[tokio::test]
    async fn a_universe_with_no_config_yet_reads_as_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(REPO_PATH))
            .respond_with(ResponseTemplate::new(404).set_body_string(""))
            .mount(&server)
            .await;

        let snapshot = client(&server)
            .get_config(UNIVERSE)
            .await
            .expect("404 means no config published, not a failure");
        assert!(snapshot.entries.is_empty());
        assert_eq!(snapshot.metadata.config_version, 0);
    }

    #[tokio::test]
    async fn a_refusal_is_not_mistaken_for_an_empty_config() {
        // The dangerous confusion: treating 403 as "nothing published" would
        // make `sync` compute a diff against an empty remote and offer to
        // overwrite a live config with everything in the file.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(REPO_PATH))
            .respond_with(ResponseTemplate::new(403).set_body_string("no access"))
            .mount(&server)
            .await;

        let error = client(&server)
            .get_config(UNIVERSE)
            .await
            .expect_err("403 is a failure")
            .to_string();
        assert!(error.contains("403"), "got: {error}");
    }

    #[tokio::test]
    async fn the_api_key_is_sent_on_reads() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(REPO_PATH))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        client(&server).get_config(UNIVERSE).await.unwrap();
    }
}
