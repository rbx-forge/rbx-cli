use std::path::Path;

use anyhow::Result;
use rbx_core::api::ApiError;
use reqwest::multipart;

use super::models::{Badge, BadgeIconResponse, ListBadgesResponse};
use super::RbxClient;

impl RbxClient {
    pub async fn list_all_badges(&self, universe_id: u64) -> Result<Vec<Badge>> {
        let api_key = self.api_key_header()?.to_string();
        let mut all_badges = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut url = {
                let badges = &self.hosts.badges;
                badges.join(&format!(
                    "/v1/universes/{}/badges?limit=100&sortOrder=Asc",
                    universe_id
                ))
            };
            if let Some(c) = &cursor {
                // Encoded, not pasted: the cursor is an opaque token and a `+`
                // or `&` in it would silently re-request page one for ever.
                url.push_str("&cursor=");
                url.push_str(&rbx_core::api::encode_query_value(c));
            }

            let list: ListBadgesResponse = rbx_core::api::execute_json(|| async {
                Ok(self
                    .client
                    .get(&url)
                    .header("x-api-key", &api_key)
                    .send()
                    .await?)
            })
            .await?;

            if let Some(data) = list.data {
                all_badges.extend(data);
            }

            match list.next_page_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
        }

        Ok(all_badges)
    }

    pub async fn get_badge(&self, badge_id: u64) -> Result<Badge> {
        let api_key = self.api_key_header()?.to_string();
        let url = self.hosts.badges.join(&format!("/v1/badges/{}", badge_id));

        rbx_core::api::execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await
    }

    pub async fn create_badge(
        &self,
        name: &str,
        description: Option<&str>,
        icon_path: Option<&Path>,
        payment_source: u32,
        expected_cost: u64,
    ) -> Result<Badge> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!(
                "/legacy-badges/v1/universes/{}/badges",
                self.universe_id
            ))
        };

        let mut form = multipart::Form::new()
            .text("name", name.to_string())
            .text("description", description.unwrap_or("").to_string())
            .text("paymentSourceType", payment_source.to_string())
            .text("expectedCost", expected_cost.to_string())
            .text("isActive", "true".to_string());

        if let Some(path) = icon_path {
            let bytes = rbx_core::image::process_image(path, self.bleed)?;
            let part = multipart::Part::bytes(bytes)
                .file_name("icon.png")
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
            return Err(ApiError::new(status, body).into());
        }

        Ok(serde_json::from_str(&body)?)
    }

    pub async fn update_badge(
        &self,
        badge_id: u64,
        name: &str,
        description: Option<&str>,
        enabled: bool,
    ) -> Result<Badge> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!("/legacy-badges/v1/badges/{}", badge_id))
        };

        let body = serde_json::json!({
            "name": name,
            "description": description.unwrap_or(""),
            "enabled": enabled,
        });

        let response = self
            .client
            .patch(&url)
            .header("x-api-key", &api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let resp_body = response.text().await?;
        if !status.is_success() {
            return Err(ApiError::new(status, resp_body).into());
        }

        Ok(serde_json::from_str(&resp_body)?)
    }

    pub async fn update_badge_icon(
        &self,
        badge_id: u64,
        icon_path: &Path,
    ) -> Result<BadgeIconResponse> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!("/legacy-publish/v1/badges/{}/icon", badge_id))
        };

        let bytes = rbx_core::image::process_image(icon_path, self.bleed)?;
        let part = multipart::Part::bytes(bytes)
            .file_name("icon.png")
            .mime_str("image/png")?;
        let form = multipart::Form::new().part("Files", part);

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
            return Err(ApiError::new(status, body).into());
        }

        Ok(serde_json::from_str(&body)?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::api::RbxClient;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 66778899001;

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
        let list_path = format!("/v1/universes/{UNIVERSE}/badges");

        Mock::given(method("GET"))
            .and(path(list_path.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 1, "name": "First" }],
                "nextPageCursor": CURSOR
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(list_path))
            .and(query_param("cursor", CURSOR))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 2, "name": "Second" }],
                "nextPageCursor": ""
            })))
            .mount(&server)
            .await;

        let badges = RbxClient::new(Some("test-key".into()), UNIVERSE, false)
            .with_base_url(server.uri())
            .list_all_badges(UNIVERSE)
            .await
            .unwrap();

        assert_eq!(badges.len(), 2);
        assert_eq!(badges[1].name.as_deref(), Some("Second"));
    }
}
