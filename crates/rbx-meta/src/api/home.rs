//! The Home Page thumbnails: `thumbnail-personalization-api`.
//!
//! Documented, and reached with the API key (`universe.thumbnail:read` and
//! `:write`), but every operation is marked `EXPERIMENTAL`, a step below
//! the BETA most of Open Cloud carries.
//!
//! It is not a list of thumbnails in an order. Images are uploaded
//! asynchronously and become homepage thumbnails once processed; Roblox then
//! serves them through a **personalization configuration**, which picks one
//! per player and keeps statistics. Creating a configuration deactivates the
//! previous one and starts the statistics over, while updating the active one
//! replaces its images and keeps them. So `sync` updates when a configuration
//! is active and creates one only when none is.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rbx_core::api::{encode_query_value, ApiError};
use reqwest::multipart;
use serde::Deserialize;

use super::RbxClient;

/// How long `sync` waits for Roblox to process an upload before giving up.
/// Roblox gives no figure; a few seconds is typical, and a minute of polling
/// separates "slow" from "not coming".
const UPLOAD_POLLS: usize = 30;
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    /// File name to operation id. A null id is a file Roblox did not accept.
    #[serde(default)]
    file_to_operation_id_dict: HashMap<String, Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    /// Only completed operations appear, per the document.
    #[serde(default)]
    upload_thumbnail_status_dict: HashMap<String, UploadedThumbnail>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedThumbnail {
    #[serde(default)]
    pub homepage_thumbnail_id: Option<String>,
    #[serde(default)]
    pub moderation_status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigsResponse {
    #[serde(default)]
    personalized_configs: Vec<PersonalizedConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersonalizedConfig {
    id: String,
    #[serde(default)]
    personalized_config_status: Option<String>,
    #[serde(default)]
    thumbnails: Vec<ConfigThumbnail>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigThumbnail {
    homepage_thumbnail_id: String,
}

/// The configuration Roblox is serving from, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveHomeConfig {
    pub id: String,
    pub homepage_thumbnail_ids: Vec<String>,
}

impl RbxClient {
    /// Upload PNGs in one request and wait until each is a homepage
    /// thumbnail. Returns, per file name, what Roblox made of it.
    ///
    /// A file Roblox rejects in moderation, or does not finish processing in
    /// time, is an error naming it. The images that did finish exist on
    /// Roblox regardless, which the error says, since the caller records
    /// nothing for a failed call.
    pub async fn upload_home_thumbnails(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> Result<HashMap<String, UploadedThumbnail>> {
        let api_key = self.api_key_header()?.to_string();
        let base = &self.base;
        let url = base.join(&format!(
            "/thumbnail-personalization-api/v1/universes/{}/thumbnails/uploads",
            self.universe_id
        ));

        let mut form = multipart::Form::new();
        for (name, bytes) in files {
            let part = multipart::Part::bytes(bytes.clone())
                .file_name(name.clone())
                .mime_str("image/png")?;
            form = form.part("files", part);
        }
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .multipart(form)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(anyhow::Error::from(ApiError::new(status, body))
                .context("uploading Home Page thumbnails"));
        }
        let parsed: UploadResponse = serde_json::from_str(&body)
            .with_context(|| format!("reading the Home Page upload reply: {body}"))?;

        let mut operations: HashMap<String, String> = HashMap::new();
        for (name, _) in files {
            match parsed.file_to_operation_id_dict.get(name) {
                Some(Some(op)) => {
                    operations.insert(op.clone(), name.clone());
                }
                _ => bail!("Roblox did not accept the Home Page thumbnail {name}: {body}"),
            }
        }

        let done = self.wait_for_home_uploads(&operations).await?;
        let mut by_file = HashMap::new();
        for (op, uploaded) in done {
            let name = operations[&op].clone();
            if uploaded.moderation_status.as_deref() == Some("Rejected") {
                bail!("Roblox moderation rejected the Home Page thumbnail {name}");
            }
            by_file.insert(name, uploaded);
        }
        Ok(by_file)
    }

    /// Poll until every operation has a homepage thumbnail id.
    ///
    /// The aggregate `uploadStatus` is an integer the document enumerates as
    /// 1 or 2 without saying which is which, so it is not read: an operation
    /// is done when its entry carries a `homepageThumbnailId`.
    async fn wait_for_home_uploads(
        &self,
        operations: &HashMap<String, String>,
    ) -> Result<HashMap<String, UploadedThumbnail>> {
        let api_key = self.api_key_header()?.to_string();
        let base = &self.base;
        let mut url = base.join(&format!(
            "/thumbnail-personalization-api/v1/universes/{}/thumbnails/uploads/status",
            self.universe_id
        ));
        for (i, op) in operations.keys().enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            url.push_str("operationIds=");
            url.push_str(&encode_query_value(op));
        }

        for attempt in 0..UPLOAD_POLLS {
            if attempt > 0 {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            let status: StatusResponse = self
                .execute_json(|| {
                    let request = self.client.get(&url).header("x-api-key", &api_key);
                    async move { request.send().await.map_err(Into::into) }
                })
                .await
                .context("reading the Home Page upload status")?;
            let finished = operations.keys().all(|op| {
                status
                    .upload_thumbnail_status_dict
                    .get(op)
                    .is_some_and(|s| s.homepage_thumbnail_id.is_some() || is_rejected(s))
            });
            if finished {
                return Ok(status
                    .upload_thumbnail_status_dict
                    .into_iter()
                    .filter(|(op, _)| operations.contains_key(op))
                    .collect());
            }
        }
        let names: Vec<&str> = operations.values().map(String::as_str).collect();
        bail!(
            "Roblox had not finished processing the Home Page thumbnails after {} seconds: {}. \
             They may still appear; nothing was recorded, so the next sync uploads them again.",
            UPLOAD_POLLS as u64 * POLL_INTERVAL.as_secs(),
            names.join(", ")
        )
    }

    /// The active personalization configuration, if there is one.
    pub async fn active_home_config(&self) -> Result<Option<ActiveHomeConfig>> {
        let api_key = self.api_key_header()?.to_string();
        let base = &self.base;
        let url = base.join(&format!(
            "/thumbnail-personalization-api/v1/universes/{}/personalization?status=Active",
            self.universe_id
        ));
        let parsed: ConfigsResponse = self
            .execute_json(|| {
                let request = self.client.get(&url).header("x-api-key", &api_key);
                async move { request.send().await.map_err(Into::into) }
            })
            .await
            .context("reading the active Home Page configuration")?;
        Ok(parsed
            .personalized_configs
            .into_iter()
            .find(|c| c.personalized_config_status.as_deref() != Some("Inactive"))
            .map(|c| ActiveHomeConfig {
                id: c.id,
                homepage_thumbnail_ids: c
                    .thumbnails
                    .into_iter()
                    .map(|t| t.homepage_thumbnail_id)
                    .collect(),
            }))
    }

    /// Point the Home Page at `ids`: update the active configuration, which
    /// keeps its statistics, or create one when none is active.
    pub async fn set_home_config(
        &self,
        active: Option<&ActiveHomeConfig>,
        ids: &[String],
    ) -> Result<()> {
        let api_key = self.api_key_header()?.to_string();
        let base = &self.base;
        let (url, body) = match active {
            Some(config) => (
                base.join(&format!(
                    "/thumbnail-personalization-api/v1/universes/{}/personalization/update",
                    self.universe_id
                )),
                serde_json::json!({ "id": config.id, "homepageThumbnailIds": ids }),
            ),
            None => (
                base.join(&format!(
                    "/thumbnail-personalization-api/v1/universes/{}/personalization/create",
                    self.universe_id
                )),
                serde_json::json!({ "homepageThumbnailIds": ids }),
            ),
        };
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::Error::from(ApiError::new(status, text))
                .context("setting the Home Page thumbnails"));
        }
        Ok(())
    }

    /// Delete homepage thumbnails by id. Called only once the configuration
    /// no longer lists them.
    pub async fn delete_home_thumbnails(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let api_key = self.api_key_header()?.to_string();
        let base = &self.base;
        let mut url = base.join(&format!(
            "/thumbnail-personalization-api/v1/universes/{}/thumbnails",
            self.universe_id
        ));
        for (i, id) in ids.iter().enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            url.push_str("homepageThumbnailIds=");
            url.push_str(&encode_query_value(id));
        }
        let response = self
            .client
            .delete(&url)
            .header("x-api-key", &api_key)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::Error::from(ApiError::new(status, text))
                .context("deleting Home Page thumbnails"));
        }
        Ok(())
    }
}

fn is_rejected(status: &UploadedThumbnail) -> bool {
    status.moderation_status.as_deref() == Some("Rejected")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::ActiveHomeConfig;
    use crate::api::RbxClient;
    use serde_json::json;
    use wiremock::matchers::{body_json, body_string_contains, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 66778899001;

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(Some("test-key".into()), None, UNIVERSE, 1, false, None)
            .with_base_url(server.uri())
    }

    fn api(rest: &str) -> String {
        format!("/thumbnail-personalization-api/v1/universes/{UNIVERSE}{rest}")
    }

    #[tokio::test]
    async fn an_upload_waits_for_the_homepage_thumbnail_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(api("/thumbnails/uploads")))
            .and(header("x-api-key", "test-key"))
            .and(body_string_contains("name=\"files\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "fileToOperationIdDict": { "home_1.png": "op-1" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(api("/thumbnails/uploads/status")))
            .and(query_param("operationIds", "op-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadStatus": 2,
                "uploadThumbnailStatusDict": {
                    "op-1": { "homepageThumbnailId": "hp-1", "assetId": 5, "moderationStatus": "Reviewing" }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let done = client(&server)
            .upload_home_thumbnails(&[("home_1.png".into(), vec![1, 2, 3])])
            .await
            .unwrap();
        assert_eq!(
            done["home_1.png"].homepage_thumbnail_id.as_deref(),
            Some("hp-1")
        );
    }

    #[tokio::test]
    async fn a_rejected_image_is_an_error_naming_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(api("/thumbnails/uploads")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "fileToOperationIdDict": { "home_1.png": "op-1" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(api("/thumbnails/uploads/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadStatus": 2,
                "uploadThumbnailStatusDict": {
                    "op-1": { "homepageThumbnailId": null, "assetId": 5, "moderationStatus": "Rejected" }
                }
            })))
            .mount(&server)
            .await;

        let err = client(&server)
            .upload_home_thumbnails(&[("home_1.png".into(), vec![1])])
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("home_1.png"), "{err:#}");
    }

    /// An active configuration is updated, keeping its statistics; one is
    /// created only when none is active.
    #[tokio::test]
    async fn the_active_configuration_is_updated_rather_than_replaced() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(api("/personalization")))
            .and(query_param("status", "Active"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "personalizedConfigs": [{
                    "id": "cfg-1",
                    "personalizedConfigStatus": "Active",
                    "createdUtc": "2026-10-09T00:00:00Z",
                    "thumbnails": [{ "homepageThumbnailId": "hp-old", "assetId": 1,
                                     "personalizedThumbnailStatus": "Active",
                                     "homepageThumbnailStatus": "Active" }]
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(api("/personalization/update")))
            .and(body_json(
                json!({ "id": "cfg-1", "homepageThumbnailIds": ["hp-1", "hp-2"] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!("cfg-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(api("/personalization/create")))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let c = client(&server);
        let active = c.active_home_config().await.unwrap();
        assert_eq!(
            active,
            Some(ActiveHomeConfig {
                id: "cfg-1".into(),
                homepage_thumbnail_ids: vec!["hp-old".into()],
            })
        );
        c.set_home_config(active.as_ref(), &["hp-1".into(), "hp-2".into()])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn with_no_active_configuration_one_is_created() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(api("/personalization/create")))
            .and(body_json(json!({ "homepageThumbnailIds": ["hp-1"] })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!("cfg-new")))
            .expect(1)
            .mount(&server)
            .await;

        client(&server)
            .set_home_config(None, &["hp-1".into()])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn deletes_name_every_id_in_the_query() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(api("/thumbnails")))
            .and(query_param("homepageThumbnailIds", "hp-1"))
            .and(query_param("homepageThumbnailIds", "hp-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        client(&server)
            .delete_home_thumbnails(&["hp-1".into(), "hp-2".into()])
            .await
            .unwrap();
    }
}
