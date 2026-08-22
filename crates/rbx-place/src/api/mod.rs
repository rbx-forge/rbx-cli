pub mod models;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use rbx_core::api::{encode_query_value, execute_json, execute_with_retry, is_api_status, ApiBase};
use reqwest::{Client, StatusCode};

use models::*;

/// The `develop` family, not Open Cloud. Listing a universe's places has no
/// Open Cloud equivalent.
const DEVELOP_HOST: &str = "https://develop.roblox.com";

/// Turn the 409 that place writes answer with into the sentence that explains
/// it.
///
/// Roblox uses `409 Conflict` on the version endpoints to mean one thing only:
/// somebody has the place open in Team Create, and the write cannot land until
/// they close it. Left as a bare status it reads like a merge conflict and
/// sends people looking at their file rather than at Studio.
///
/// This crate used to keep a whole private retry loop so that a 409 could come
/// back as an `Ok` response the caller inspected. With the status recoverable
/// from the error itself, the loop was deletable and this is what replaced it.
///
/// `doing` is the context added to *other* failures. The lock message
/// deliberately gets none: `anyhow` renders only the outermost frame by
/// default, so wrapping it would put "uploading the place file" on screen and
/// bury the sentence that tells the user what to actually do. Any other
/// failure keeps its status and gains the context.
fn place_write_error(error: anyhow::Error, place_id: u64, doing: &'static str) -> anyhow::Error {
    if is_api_status(&error, StatusCode::CONFLICT) {
        return anyhow::anyhow!(
            "Place {} is locked by an active Team Create session.\n\
             Close the Team Create session in Studio and retry.",
            place_id
        );
    }
    error.context(doing)
}

pub struct RbxClient {
    client: Client,
    api_key: String,
    base: ApiBase,
    develop: ApiBase,
}

impl RbxClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: rbx_core::api::build_client(),
            api_key,
            base: ApiBase::default(),
            develop: ApiBase::new(DEVELOP_HOST),
        }
    }

    /// Point every host at one server. Testing only.
    ///
    /// Both bases move together: a test wants one `wiremock` server answering
    /// whatever the code under test asks for, and splitting them would mean
    /// standing up two just to exercise one call.
    ///
    /// This used to be `cfg(test)`. It is reachable from the commands now
    /// because the hidden `--base-url` flag needs it: the `--json` documents
    /// are a contract about what reaches the process's stdout, which only a
    /// test that runs the binary against a mock host can hold it to. `mod api`
    /// stays private, so the method is still not part of the crate's surface.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        self.base = ApiBase::new(url.clone());
        self.develop = ApiBase::new(url);
        self
    }

    // -------------------------------------------------------------------------
    // Place upload
    // -------------------------------------------------------------------------

    /// Upload a .rbxl file. Returns the new version number.
    /// Returns a descriptive error on 409 (Team Create lock).
    ///
    /// `data` is `Bytes` rather than `Vec<u8>` because the send lives inside a
    /// retry closure that may run several times: a refcounted buffer lets the
    /// attempts share one allocation instead of copying a whole place file per
    /// attempt. Callers uploading the same file to several places get the same
    /// deal from their own `clone`.
    pub async fn upload_place(
        &self,
        universe_id: u64,
        place_id: u64,
        data: Bytes,
        published: bool,
    ) -> Result<u64> {
        let version_type = if published { "Published" } else { "Saved" };
        let url = self.base.join(&format!(
            "/universes/v1/{}/places/{}/versions?versionType={}",
            universe_id, place_id, version_type
        ));
        let api_key = self.api_key.clone();

        let response = execute_with_retry(|| async {
            Ok(self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("Content-Type", "application/octet-stream")
                .body(data.clone())
                .send()
                .await?)
        })
        .await
        .map_err(|error| place_write_error(error, place_id, "uploading the place file"))?;

        let body = response.text().await?;
        let result: UploadVersionResponse = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!("Failed to parse upload response: {}\nBody: {}", e, body)
        })?;
        Ok(result.version_number)
    }

    // -------------------------------------------------------------------------
    // Place download
    // -------------------------------------------------------------------------

    /// Returns the CDN download URL for a place (latest if version is None).
    pub async fn get_download_url(&self, place_id: u64, version: Option<u64>) -> Result<String> {
        let url = self.base.join(&match version {
            Some(v) => format!("/asset-delivery-api/v1/assetId/{}/version/{}", place_id, v),
            None => format!("/asset-delivery-api/v1/assetId/{}", place_id),
        });
        let api_key = self.api_key.clone();
        let resp: AssetDeliveryResponse = execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await?;
        Ok(resp.location)
    }

    /// Download raw bytes from a CDN URL (no auth required).
    ///
    /// Handing back the `Bytes` `reqwest` already produced saves a copy of the
    /// whole file, and it is what `upload_place` wants anyway: `promote`
    /// pipes one straight into the other.
    pub async fn download_from_url(&self, url: &str) -> Result<Bytes> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Download request failed")?;
        if !response.status().is_success() {
            bail!("Download failed ({})", response.status());
        }
        Ok(response.bytes().await?)
    }

    // -------------------------------------------------------------------------
    // Version list
    // -------------------------------------------------------------------------

    /// Walk the asset-version pages of a place, newest first, keeping at most
    /// `max` entries.
    ///
    /// `published` is the filter: `Some(true)` keeps live versions, `Some(false)`
    /// keeps drafts, `None` keeps everything. The three public listers below
    /// used to carry their own copy of this loop and differed only in those two
    /// arguments, so the paging (token threading, the empty-token terminator,
    /// the URL shape) is spelled once here instead of three times.
    ///
    /// Pagination stops as soon as `max` is reached, so `find_version` costs one
    /// page in the common case rather than a full walk.
    async fn collect_versions(
        &self,
        place_id: u64,
        max: usize,
        published: Option<bool>,
    ) -> Result<Vec<VersionInfo>> {
        let mut versions = Vec::new();
        let mut page_token: Option<String> = None;
        let api_key = self.api_key.clone();

        loop {
            let url = self.base.join(&format!(
                "/assets/v1/assets/{}/versions?maxPageSize=50{}",
                place_id,
                // Encoded, not pasted: the page token is an opaque token and a
                // `+` or `&` in it would silently re-request page one for ever.
                page_token
                    .as_deref()
                    .map(|t| format!("&pageToken={}", encode_query_value(t)))
                    .unwrap_or_default()
            ));

            let page: AssetVersionsPage = execute_json(|| async {
                Ok(self
                    .client
                    .get(&url)
                    .header("x-api-key", &api_key)
                    .send()
                    .await?)
            })
            .await?;

            for entry in &page.asset_versions {
                let Some(n) = entry.version_number() else {
                    continue;
                };
                let is_published = entry.published.unwrap_or(false);
                if published.is_some_and(|want| want != is_published) {
                    continue;
                }
                versions.push(VersionInfo {
                    version_number: n,
                    create_time: entry.create_time.clone(),
                    published: is_published,
                });
                if versions.len() >= max {
                    return Ok(versions);
                }
            }

            if page.next_page_token.is_empty() {
                break;
            }
            page_token = Some(page.next_page_token);
        }

        Ok(versions)
    }

    /// Find the latest version matching a published/draft filter, paginating as needed.
    /// Returns None if no matching version exists.
    pub async fn find_version(
        &self,
        place_id: u64,
        published: bool,
    ) -> Result<Option<VersionInfo>> {
        Ok(self
            .collect_versions(place_id, 1, Some(published))
            .await?
            .pop())
    }

    /// List up to `max` versions matching a published/draft filter, paginating as needed.
    pub async fn list_versions_filtered(
        &self,
        place_id: u64,
        max: usize,
        published: bool,
    ) -> Result<Vec<VersionInfo>> {
        self.collect_versions(place_id, max, Some(published)).await
    }

    /// List up to `max` versions of a place asset, newest first.
    pub async fn list_versions(&self, place_id: u64, max: usize) -> Result<Vec<VersionInfo>> {
        self.collect_versions(place_id, max, None).await
    }

    // -------------------------------------------------------------------------
    // Rollback
    // -------------------------------------------------------------------------

    /// Roll back a place to a specific version. Returns the new version number.
    /// Returns a descriptive error on 409 (Team Create lock).
    pub async fn rollback_place(&self, place_id: u64, version: u64) -> Result<u64> {
        let url = self
            .base
            .join(&format!("/assets/v1/assets/{}/versions:rollback", place_id));
        let api_key = self.api_key.clone();
        let body = serde_json::json!({
            "assetVersion": format!("assets/{}/versions/{}", place_id, version)
        });

        let response = execute_with_retry(|| async {
            Ok(self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?)
        })
        .await
        .map_err(|error| place_write_error(error, place_id, "rolling the place back"))?;

        let text = response.text().await?;
        let result: RollbackResponse = serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!("Failed to parse rollback response: {}\nBody: {}", e, text)
        })?;
        result
            .version_number()
            .ok_or_else(|| anyhow::anyhow!("Could not extract version number from: {}", text))
    }

    // -------------------------------------------------------------------------
    // Universe places
    // -------------------------------------------------------------------------

    /// List all places in a universe (paginates automatically).
    pub async fn list_universe_places(&self, universe_id: u64) -> Result<Vec<PlaceEntry>> {
        let mut places = Vec::new();
        let mut next_cursor: Option<String> = None;

        loop {
            // Encoded, not pasted: the cursor is an opaque token and a `+` or
            // `&` in it would silently re-request page one for ever.
            let cursor_param = next_cursor
                .as_deref()
                .map(|c| format!("&cursor={}", encode_query_value(c)))
                .unwrap_or_default();

            let url = self.develop.join(&format!(
                "/v1/universes/{}/places?isUniverseCreation=false&limit=100&sortOrder=Asc{}",
                universe_id, cursor_param
            ));

            let page: DevelopPlacesPage =
                execute_json(|| async { Ok(self.client.get(&url).send().await?) }).await?;

            places.extend(page.data.into_iter().map(|e| e.into()));

            if page.next_page_cursor.is_none()
                || page.next_page_cursor.as_ref().is_none_or(|c| c.is_empty())
            {
                break;
            }
            next_cursor = page.next_page_cursor;
        }

        Ok(places)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const PLACE: u64 = 123456789012345;
    const UNIVERSE: u64 = 9876543210;

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new("test-key".into()).with_base_url(server.uri())
    }

    /// 409 is the one status these endpoints overload, and the message it
    /// earns is the difference between "close Studio" and a shrug. The
    /// mapping used to live in a private retry loop that returned the 409 as
    /// a success; these cover it now that it goes through the shared helper.
    #[tokio::test]
    async fn a_409_on_upload_names_team_create_rather_than_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/universes/v1/{UNIVERSE}/places/{PLACE}/versions"
            )))
            .respond_with(ResponseTemplate::new(409).set_body_string("conflict"))
            .mount(&server)
            .await;

        let error = client(&server)
            .upload_place(UNIVERSE, PLACE, Bytes::from_static(b"rbxl"), true)
            .await
            .expect_err("a locked place is a failure")
            .to_string();
        assert!(error.contains("Team Create"), "got: {error}");
        assert!(error.contains(&PLACE.to_string()), "got: {error}");
    }

    #[tokio::test]
    async fn a_409_on_rollback_names_team_create_too() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/assets/v1/assets/{PLACE}/versions:rollback")))
            .respond_with(ResponseTemplate::new(409).set_body_string("conflict"))
            .mount(&server)
            .await;

        let error = client(&server)
            .rollback_place(PLACE, 3)
            .await
            .expect_err("a locked place is a failure")
            .to_string();
        assert!(error.contains("Team Create"), "got: {error}");
    }

    #[tokio::test]
    async fn a_failure_that_is_not_a_lock_keeps_its_own_error() {
        // The mapping must be narrow. Blaming Team Create for a rejected key
        // would send somebody to close a session that was never open.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/universes/v1/{UNIVERSE}/places/{PLACE}/versions"
            )))
            .respond_with(ResponseTemplate::new(403).set_body_string("no permission"))
            .mount(&server)
            .await;

        let error = format!(
            "{:#}",
            client(&server)
                .upload_place(UNIVERSE, PLACE, Bytes::from_static(b"rbxl"), true)
                .await
                .expect_err("403 is a failure")
        );
        assert!(!error.contains("Team Create"), "got: {error}");
        assert!(error.contains("403"), "got: {error}");
    }

    #[tokio::test]
    async fn a_successful_upload_reports_the_new_version() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/universes/v1/{UNIVERSE}/places/{PLACE}/versions"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "versionNumber": 42
            })))
            .mount(&server)
            .await;

        let version = client(&server)
            .upload_place(UNIVERSE, PLACE, Bytes::from_static(b"rbxl"), true)
            .await
            .unwrap();
        assert_eq!(version, 42);
    }

    /// The page token carries reserved characters on purpose: it is opaque, so
    /// it has to reach the server byte for byte. Pasted raw, `a+b` arrives as
    /// `a b` and everything after the `&` becomes its own parameter, which is
    /// page one again rather than an error.
    const PAGE_TOKEN: &str = "a+b/c=d&e f";

    /// Two pages of versions, newest first: 5 and 4 are live, 3 is a draft, 2 is
    /// live. The draft only appears on the second page, so anything that finds
    /// it had to follow the page token.
    async fn paged_versions_server() -> MockServer {
        let server = MockServer::start().await;
        let versions_path = format!("/assets/v1/assets/{PLACE}/versions");

        Mock::given(method("GET"))
            .and(path(versions_path.clone()))
            .and(query_param_is_missing("pageToken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "assetVersions": [
                    { "path": format!("assets/{PLACE}/versions/5"), "createTime": "2024-01-05T00:00:00Z", "published": true },
                    { "path": format!("assets/{PLACE}/versions/4"), "createTime": "2024-01-04T00:00:00Z", "published": true },
                ],
                "nextPageToken": PAGE_TOKEN
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(versions_path))
            .and(query_param("pageToken", PAGE_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "assetVersions": [
                    { "path": format!("assets/{PLACE}/versions/3"), "createTime": "2024-01-03T00:00:00Z", "published": false },
                    { "path": format!("assets/{PLACE}/versions/2"), "createTime": "2024-01-02T00:00:00Z", "published": true },
                ],
                "nextPageToken": ""
            })))
            .mount(&server)
            .await;

        server
    }

    #[tokio::test]
    async fn finding_a_draft_follows_the_page_token_past_the_live_ones() {
        let server = paged_versions_server().await;
        let found = client(&server)
            .find_version(PLACE, false)
            .await
            .unwrap()
            .expect("version 3 is a draft");
        assert_eq!(found.version_number, 3);
        assert!(!found.published);
    }

    #[tokio::test]
    async fn a_filtered_list_keeps_only_its_side_across_pages() {
        let server = paged_versions_server().await;
        let live: Vec<u64> = client(&server)
            .list_versions_filtered(PLACE, 10, true)
            .await
            .unwrap()
            .into_iter()
            .map(|v| v.version_number)
            .collect();
        assert_eq!(live, vec![5, 4, 2]);
    }

    #[tokio::test]
    async fn an_unfiltered_list_stops_at_max_without_asking_for_page_two() {
        // The second page is mounted but must go unused: reaching `max` is a
        // terminator, not just a truncation of everything fetched.
        let server = paged_versions_server().await;
        let all: Vec<u64> = client(&server)
            .list_versions(PLACE, 2)
            .await
            .unwrap()
            .into_iter()
            .map(|v| v.version_number)
            .collect();
        assert_eq!(all, vec![5, 4]);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    /// A cursor is an opaque token. Roblox returns base64url today, but the
    /// value is theirs to change, and one pasted raw into the query string is
    /// re-parsed by the server as something else: `a+b` decodes to `a b`, and
    /// everything after an `&` becomes a separate parameter. Both ask for
    /// page one again, so the listing loops on the first page for ever rather
    /// than erroring.
    #[tokio::test]
    async fn a_cursor_with_reserved_characters_reaches_the_server_intact() {
        const CURSOR: &str = "a+b/c=d&e f";
        let server = MockServer::start().await;
        let list_path = format!("/v1/universes/{UNIVERSE}/places");

        Mock::given(method("GET"))
            .and(path(list_path.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "id": 1, "name": "First" }],
                "nextPageCursor": CURSOR
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(list_path))
            .and(query_param("cursor", CURSOR))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "id": 2, "name": "Second" }],
                "nextPageCursor": ""
            })))
            .mount(&server)
            .await;

        let places = client(&server)
            .list_universe_places(UNIVERSE)
            .await
            .unwrap();

        assert_eq!(places.len(), 2);
        assert_eq!(places[1].display_name, "Second");
    }
}
