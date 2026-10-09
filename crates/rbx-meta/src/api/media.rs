//! The icon and thumbnails, in either of the two sets an experience has.
//!
//! **Its own media** (`language_code` unset) is what the Creator Hub's
//! thumbnails page shows. Roblox's OpenAPI document has no endpoint that writes
//! it, so this goes where the Creator Hub goes: `publish.roblox.com` to upload,
//! `develop.roblox.com` to order and delete, cookie and CSRF on all three, and
//! `games.roblox.com` to list, anonymously. Probed on 2026-10-09: each of the
//! three write routes answers 403 "XSRF token invalid" to an anonymous call
//! where a bogus path on the same host answers 404.
//!
//! **A translation** (`language_code` set) goes through the documented
//! localization API with the API key. Roblox refuses the source language
//! there, 400 "You can't update translations for source language", which is
//! why the experience's own media could never be written that way.
//!
//! The two sets do not share ids: an image id from one means nothing to the
//! other's delete or order call. `sync` keeps them apart in the lockfile.

use anyhow::{bail, Context, Result};
use rbx_core::api::{roblox_error, ApiError};
use reqwest::{header, multipart};
use serde::Deserialize;

use super::models::{IconUploadResponse, ThumbnailUploadResponse};
use super::RbxClient;

/// `GET games.roblox.com/v1/games/{id}/media`.
#[derive(Debug, Deserialize)]
struct OwnMediaList {
    #[serde(default)]
    data: Vec<OwnMediaItem>,
}

#[derive(Debug, Deserialize)]
struct OwnMediaItem {
    id: u64,
}

/// `GET gameinternationalization.roblox.com/v1/game-thumbnails/games/{id}/images`.
#[derive(Debug, Deserialize)]
struct TranslatedThumbnails {
    #[serde(default)]
    data: Vec<TranslatedLanguage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranslatedLanguage {
    language_code: String,
    #[serde(default)]
    media_assets: Vec<TranslatedAsset>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranslatedAsset {
    /// A string in the reply, like the upload's.
    media_asset_id: String,
    #[serde(default)]
    media_asset_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceLanguage {
    language_code: String,
}

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
    /// The experience's source language, e.g. `en`. Public.
    pub async fn source_language(&self) -> Result<String> {
        let intl = &self.intl;
        let url = intl.join(&format!("/v1/source-language/games/{}", self.universe_id));
        let parsed: SourceLanguage = self
            .execute_json(|| {
                let request = self.client.get(&url);
                async move { request.send().await.map_err(Into::into) }
            })
            .await
            .context("reading the experience's source language")?;
        Ok(parsed.language_code)
    }

    /// The ids of the thumbnails Roblox holds in the set this client writes,
    /// in display order. Public reads for both sets.
    pub async fn remote_thumbnail_ids(&self) -> Result<Vec<u64>> {
        match &self.language_code {
            None => {
                let games = &self.games;
                let url = games.join(&format!("/v1/games/{}/media", self.universe_id));
                let parsed: OwnMediaList = self
                    .execute_json(|| {
                        let request = self.client.get(&url);
                        async move { request.send().await.map_err(Into::into) }
                    })
                    .await
                    .context("listing the experience's own thumbnails")?;
                Ok(parsed.data.into_iter().map(|item| item.id).collect())
            }
            Some(code) => Ok(self
                .translated_thumbnails(code)
                .await?
                .into_iter()
                .filter_map(|asset| asset.media_asset_id.parse().ok())
                .collect()),
        }
    }

    async fn translated_thumbnails(&self, code: &str) -> Result<Vec<TranslatedAsset>> {
        let intl = &self.intl;
        let url = intl.join(&format!(
            "/v1/game-thumbnails/games/{}/images",
            self.universe_id
        ));
        let parsed: TranslatedThumbnails = self
            .execute_json(|| {
                let request = self.client.get(&url);
                async move { request.send().await.map_err(Into::into) }
            })
            .await
            .context("listing the translated thumbnails")?;
        Ok(parsed
            .data
            .into_iter()
            .find(|language| language.language_code.eq_ignore_ascii_case(code))
            .map(|language| language.media_assets)
            .unwrap_or_default())
    }

    /// A cookie-and-CSRF multipart upload of one PNG, as the Creator Hub
    /// sends it: the part is `request.files` on both publish routes. The form
    /// is rebuilt per attempt because a CSRF retry resends the request, and a
    /// multipart body is consumed by the first send.
    async fn publish_png(&self, url: &str, file_name: &str, png: &[u8]) -> Result<String> {
        let cookie = rbx_core::session::cookie_header(self.cookie_header()?);
        let build = || {
            let part = multipart::Part::bytes(png.to_vec())
                .file_name(file_name.to_string())
                .mime_str("image/png")
                .expect("image/png is a valid mime type");
            self.client
                .post(url)
                .header(header::COOKIE, &cookie)
                .multipart(multipart::Form::new().part("request.files", part))
        };
        let response = self.send_response_with_csrf(build).await?;
        Ok(response.text().await.unwrap_or_default())
    }

    /// Upload (or replace) the icon of the set this client writes.
    pub async fn upload_icon(&self, png_bytes: Vec<u8>) -> Result<IconUploadResponse> {
        let Some(code) = &self.language_code else {
            let publish = &self.publish;
            let url = publish.join(&format!("/v1/games/{}/icon", self.universe_id));
            let body = self
                .publish_png(&url, "icon.png", &png_bytes)
                .await
                .context("uploading the experience's icon")?;
            // The id is a courtesy for the lockfile here: an icon is replaced,
            // never deleted or ordered by id, so a reply without one is not
            // the problem it is for a thumbnail.
            return Ok(serde_json::from_str(&body).unwrap_or(IconUploadResponse {
                image_id: None,
                language_code: None,
            }));
        };
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/legacy-game-internationalization/v1/game-icon/games/{}/language-codes/{}",
            self.universe_id, code
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
        let body = match &self.language_code {
            // `{"targetId": <id>}` comes back here, `mediaAssetId` on the
            // translation route; `ThumbnailUploadResponse` reads either.
            None => {
                let publish = &self.publish;
                let url = publish.join(&format!("/v1/games/{}/thumbnail/image", self.universe_id));
                self.publish_png(&url, "thumbnail.png", &png_bytes)
                    .await
                    .context("uploading a thumbnail")?
            }
            Some(code) => {
                let api_key = self.api_key_header()?.to_string();
                let url = self.api_url(&format!(
                    "/legacy-game-internationalization/v1/game-thumbnails/games/{}/language-codes/{}/image",
                    self.universe_id, code
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
                    return Err(anyhow::Error::from(ApiError::new(status, body))
                        .context("uploading a thumbnail"));
                }
                body
            }
        };

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

    /// Delete a thumbnail of the set this client writes, by its id.
    pub async fn delete_thumbnail(&self, image_id: u64) -> Result<()> {
        let Some(code) = &self.language_code else {
            let cookie = rbx_core::session::cookie_header(self.cookie_header()?);
            let url = self.legacy_url(&format!(
                "/v1/universes/{}/thumbnails/{image_id}",
                self.universe_id
            ));
            let build = || self.client.delete(&url).header(header::COOKIE, &cookie);
            return self
                .send_with_csrf(build)
                .await
                .context("deleting a thumbnail");
        };
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/legacy-game-internationalization/v1/game-thumbnails/games/{}/language-codes/{}/images/{}",
            self.universe_id, code, image_id
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
    ///
    /// Reads the set this client writes, so that `pull` and `sync` agree on
    /// which images the lockfile's ids belong to. Before 0.10.1 `pull` read
    /// the experience's own thumbnails here while `sync` wrote a translation,
    /// and each recorded ids the other could not act on.
    pub async fn fetch_thumbnails(&self) -> Result<Vec<RemoteMedia>> {
        if let Some(code) = &self.language_code {
            return Ok(self
                .translated_thumbnails(code)
                .await?
                .into_iter()
                .filter_map(|asset| {
                    Some(RemoteMedia {
                        target_id: asset.media_asset_id.parse().ok()?,
                        image_url: asset.media_asset_url?,
                    })
                })
                .collect());
        }
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
        let Some(code) = &self.language_code else {
            let cookie = rbx_core::session::cookie_header(self.cookie_header()?);
            let url = self.legacy_url(&format!(
                "/v1/universes/{}/thumbnails/order",
                self.universe_id
            ));
            let body = serde_json::json!({ "thumbnailIds": image_ids });
            let build = || {
                self.client
                    .post(&url)
                    .header(header::COOKIE, &cookie)
                    .json(&body)
            };
            return self
                .send_with_csrf(build)
                .await
                .context("reordering thumbnails");
        };
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/legacy-game-internationalization/v1/game-thumbnails/games/{}/language-codes/{}/images/order",
            self.universe_id, code
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
            Some("en_us".into()),
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

/// The experience's own media: the Creator Hub's routes, each mocked on its own
/// host so a request sent to the wrong one fails rather than being answered.
#[cfg(test)]
mod own_media_tests {
    #![allow(clippy::unwrap_used)]

    use crate::api::RbxClient;
    use serde_json::json;
    use wiremock::matchers::{body_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 66778899001;

    struct Hosts {
        legacy: MockServer,
        publish: MockServer,
        games: MockServer,
        intl: MockServer,
    }

    async fn hosts() -> Hosts {
        Hosts {
            legacy: MockServer::start().await,
            publish: MockServer::start().await,
            games: MockServer::start().await,
            intl: MockServer::start().await,
        }
    }

    fn client(h: &Hosts, cookie: Option<&str>, language: Option<&str>) -> RbxClient {
        RbxClient::new(
            Some("test-key".into()),
            cookie.map(Into::into),
            UNIVERSE,
            1,
            false,
            language.map(Into::into),
        )
        .with_legacy_base_url(h.legacy.uri())
        .with_media_hosts(h.publish.uri(), h.games.uri(), h.intl.uri())
    }

    /// The upload the Creator Hub makes: publish, cookie, `request.files`,
    /// and the id back as `targetId`.
    #[tokio::test]
    async fn an_own_thumbnail_goes_to_publish_with_the_cookie() {
        let h = hosts().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/games/{UNIVERSE}/thumbnail/image")))
            .and(header("cookie", ".ROBLOSECURITY=session"))
            .and(body_string_contains("name=\"request.files\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "targetId": 555 })))
            .expect(1)
            .mount(&h.publish)
            .await;

        let id = client(&h, Some("session"), None)
            .upload_thumbnail(vec![1, 2, 3])
            .await
            .unwrap();
        assert_eq!(id, 555);
    }

    #[tokio::test]
    async fn own_thumbnails_are_deleted_and_ordered_on_develop() {
        let h = hosts().await;
        Mock::given(method("DELETE"))
            .and(path(format!("/v1/universes/{UNIVERSE}/thumbnails/555")))
            .and(header("cookie", ".ROBLOSECURITY=session"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&h.legacy)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/universes/{UNIVERSE}/thumbnails/order")))
            .and(body_json(json!({ "thumbnailIds": [2, 1] })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&h.legacy)
            .await;

        let c = client(&h, Some("session"), None);
        c.delete_thumbnail(555).await.unwrap();
        c.reorder_thumbnails(&[2, 1]).await.unwrap();
    }

    /// Without a session the own set cannot be written at all, and the error
    /// says how to supply one rather than failing at Roblox.
    #[tokio::test]
    async fn the_own_set_needs_a_cookie() {
        let h = hosts().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&h.publish)
            .await;

        let err = client(&h, None, None)
            .upload_thumbnail(vec![1])
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("ROBLOSECURITY"), "{err:#}");
    }

    #[tokio::test]
    async fn each_set_lists_its_own_thumbnails() {
        let h = hosts().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/games/{UNIVERSE}/media")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "data": [{ "id": 1 }, { "id": 2 }] })),
            )
            .mount(&h.games)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/game-thumbnails/games/{UNIVERSE}/images")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
                { "languageCode": "en", "mediaAssets": [] },
                { "languageCode": "en_us", "mediaAssets": [
                    { "mediaAssetId": "7", "state": "Approved", "mediaAssetUrl": "https://example.invalid/7" }
                ]},
            ]})))
            .mount(&h.intl)
            .await;

        assert_eq!(
            client(&h, None, None).remote_thumbnail_ids().await.unwrap(),
            vec![1, 2]
        );
        assert_eq!(
            client(&h, None, Some("en_us"))
                .remote_thumbnail_ids()
                .await
                .unwrap(),
            vec![7]
        );
    }

    #[tokio::test]
    async fn the_source_language_is_read_anonymously() {
        let h = hosts().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/source-language/games/{UNIVERSE}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "English", "nativeName": "English", "languageCode": "en"
            })))
            .expect(1)
            .mount(&h.intl)
            .await;

        assert_eq!(
            client(&h, None, None).source_language().await.unwrap(),
            "en"
        );
    }
}
