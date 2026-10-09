use anyhow::{bail, Context, Result};
use rbx_core::api::{roblox_error, ApiError};
use reqwest::{header, multipart};
use serde::Deserialize;

use super::models::{IconUploadResponse, ThumbnailUploadResponse};
use super::RbxClient;

#[derive(Debug, Deserialize)]
struct ThumbnailServiceResponse<T> {
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IconEntry {
    target_id: u64,
    image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThumbnailGroup {
    thumbnails: Vec<ThumbnailEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThumbnailEntry {
    target_id: u64,
    image_url: Option<String>,
}

/// Pair of (target_id, image_url) returned by the public thumbnails service.
pub struct RemoteMedia {
    pub target_id: u64,
    pub image_url: String,
}

impl RbxClient {
    /// Upload (or replace) the universe icon for the configured language.
    pub async fn upload_icon(&self, png_bytes: Vec<u8>) -> Result<IconUploadResponse> {
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/legacy-game-internationalization/v1/game-icon/games/{}/language-codes/{}",
            self.universe_id, self.language_code
        ));

        let part = multipart::Part::bytes(png_bytes)
            .file_name("icon.png")
            .mime_str("image/png")?;
        let form = multipart::Form::new().part("request.files", part);

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
                .context("uploading the universe icon"));
        }

        let parsed: IconUploadResponse =
            serde_json::from_str(&body).unwrap_or(IconUploadResponse {
                image_id: None,
                language_code: None,
            });
        Ok(parsed)
    }

    /// Upload a thumbnail. Roblox appends to the universe's thumbnail list.
    ///
    /// Returns the new image's id, and fails when there is none to return. An
    /// upload the lockfile cannot record is not a success: the next sync would
    /// not recognise the image, upload it again, and Roblox would keep both.
    pub async fn upload_thumbnail(&self, png_bytes: Vec<u8>) -> Result<u64> {
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/legacy-game-internationalization/v1/game-thumbnails/games/{}/language-codes/{}/image",
            self.universe_id, self.language_code
        ));

        let part = multipart::Part::bytes(png_bytes)
            .file_name("thumbnail.png")
            .mime_str("image/png")?;
        let form = multipart::Form::new().part("gameThumbnailRequest.files", part);

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
            return Err(
                anyhow::Error::from(ApiError::new(status, body)).context("uploading a thumbnail")
            );
        }

        // The image is on Roblox by now, whatever happens below, so both errors
        // say so and point at the command that records what Roblox has.
        let parsed: ThumbnailUploadResponse = serde_json::from_str(&body).with_context(|| {
            format!(
                "the thumbnail was uploaded, but Roblox's reply could not be read, so it is not \
                 recorded in the lockfile. Run `rbx meta pull` before the next sync, or it will \
                 upload the image again. Reply: {body}"
            )
        })?;
        match parsed.image_id {
            Some(id) => Ok(id),
            None => bail!(
                "the thumbnail was uploaded, but Roblox's reply carries no media asset id, so it \
                 is not recorded in the lockfile. Run `rbx meta pull` before the next sync, or \
                 it will upload the image again. Reply: {body}"
            ),
        }
    }

    /// Delete a thumbnail by its media asset ID.
    pub async fn delete_thumbnail(&self, image_id: u64) -> Result<()> {
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/legacy-game-internationalization/v1/game-thumbnails/games/{}/language-codes/{}/images/{}",
            self.universe_id, self.language_code, image_id
        ));

        let response = self
            .client
            .delete(&url)
            .header("x-api-key", &api_key)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(
                anyhow::Error::from(ApiError::new(status, body)).context("deleting a thumbnail")
            );
        }
        Ok(())
    }

    /// Fetch the current icon URL via the public thumbnails service.
    /// Returns `None` if the universe has no icon.
    pub async fn fetch_icon(&self) -> Result<Option<RemoteMedia>> {
        let url = format!(
            "https://thumbnails.roblox.com/v1/games/icons?universeIds={}&size=512x512&format=Png&isCircular=false",
            self.universe_id
        );
        let mut req = self.client.get(&url);
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, format!(".ROBLOSECURITY={}", c));
        }
        let response = req.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(roblox_error(status, &body).context("reading from the thumbnails service"));
        }
        let parsed: ThumbnailServiceResponse<IconEntry> = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Failed to parse icon response: {}\nBody: {}", e, body))?;
        Ok(parsed.data.into_iter().next().and_then(|e| {
            e.image_url.map(|url| RemoteMedia {
                target_id: e.target_id,
                image_url: url,
            })
        }))
    }

    /// Fetch the universe's thumbnail URLs (in display order) via the public thumbnails service.
    pub async fn fetch_thumbnails(&self) -> Result<Vec<RemoteMedia>> {
        let url = format!(
            "https://thumbnails.roblox.com/v1/games/multiget/thumbnails?universeIds={}&size=768x432&format=Png&countPerUniverse=10",
            self.universe_id
        );
        let mut req = self.client.get(&url);
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, format!(".ROBLOSECURITY={}", c));
        }
        let response = req.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(roblox_error(status, &body).context("reading from the thumbnails service"));
        }
        let parsed: ThumbnailServiceResponse<ThumbnailGroup> = serde_json::from_str(&body)
            .map_err(|e| {
                anyhow::anyhow!("Failed to parse thumbnails response: {}\nBody: {}", e, body)
            })?;
        let Some(group) = parsed.data.into_iter().next() else {
            return Ok(Vec::new());
        };
        Ok(group
            .thumbnails
            .into_iter()
            .filter_map(|t| {
                t.image_url.map(|url| RemoteMedia {
                    target_id: t.target_id,
                    image_url: url,
                })
            })
            .collect())
    }

    /// Download raw bytes from a Roblox CDN URL.
    pub async fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.client.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            bail!("Failed to download from {}: {}", url, status);
        }
        Ok(response.bytes().await?.to_vec())
    }

    /// Reorder thumbnails. `image_ids` is the desired final order.
    pub async fn reorder_thumbnails(&self, image_ids: &[u64]) -> Result<()> {
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/legacy-game-internationalization/v1/game-thumbnails/games/{}/language-codes/{}/images/order",
            self.universe_id, self.language_code
        ));

        let body = serde_json::json!({ "mediaAssetIds": image_ids });

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(
                anyhow::Error::from(ApiError::new(status, body)).context("reordering thumbnails")
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::api::RbxClient;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 66778899001;

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(
            Some("test-key".into()),
            None,
            UNIVERSE,
            1,
            false,
            "en_us".into(),
        )
        .with_base_url(server.uri())
    }

    async fn mount_upload(server: &MockServer, reply: serde_json::Value) {
        Mock::given(method("POST"))
            .and(path(format!(
                "/legacy-game-internationalization/v1/game-thumbnails/games/{UNIVERSE}/language-codes/en_us/image"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply))
            .mount(server)
            .await;
    }

    /// The shape the document gives, `{mediaAssetId: string}`. Read as a
    /// number, this parsed to nothing and every sync re-uploaded every image.
    #[tokio::test]
    async fn the_media_asset_id_is_read_from_a_string() {
        let server = MockServer::start().await;
        mount_upload(&server, json!({ "mediaAssetId": "18234567890" })).await;

        let id = client(&server)
            .upload_thumbnail(vec![1, 2, 3])
            .await
            .unwrap();
        assert_eq!(id, 18234567890);
    }

    #[tokio::test]
    async fn a_numeric_media_asset_id_is_read_too() {
        let server = MockServer::start().await;
        mount_upload(&server, json!({ "mediaAssetId": 18234567890u64 })).await;

        let id = client(&server)
            .upload_thumbnail(vec![1, 2, 3])
            .await
            .unwrap();
        assert_eq!(id, 18234567890);
    }

    /// A success with no id is an error, not an entry recorded without one.
    /// The error has to say the image is already on Roblox, since that is
    /// what decides what to do next.
    #[tokio::test]
    async fn an_upload_with_no_id_in_the_reply_is_an_error() {
        let server = MockServer::start().await;
        mount_upload(&server, json!({})).await;

        let err = client(&server)
            .upload_thumbnail(vec![1, 2, 3])
            .await
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("was uploaded"), "{message}");
        assert!(message.contains("rbx meta pull"), "{message}");
    }
}
