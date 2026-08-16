use std::path::Path;

use anyhow::Result;
use rbx_core::api::ApiError;
use reqwest::multipart;

use super::models::{GamePass, ListGamePassesResponse};
use super::RbxClient;

impl RbxClient {
    pub async fn list_all_game_passes(&self) -> Result<Vec<GamePass>> {
        let api_key = self.api_key_header()?.to_string();
        let mut all_passes = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut url = {
                let cloud = &self.hosts.cloud;
                cloud.join(&format!(
                    "/game-passes/v1/universes/{}/game-passes/creator?pageSize=100",
                    self.universe_id
                ))
            };
            if let Some(token) = &page_token {
                // Encoded, not pasted: the token is opaque and a `+` or `&` in
                // it would silently re-request page one for ever.
                url.push_str("&pageToken=");
                url.push_str(&rbx_core::api::encode_query_value(token));
            }

            let list: ListGamePassesResponse = rbx_core::api::execute_json(|| async {
                Ok(self
                    .client
                    .get(&url)
                    .header("x-api-key", &api_key)
                    .send()
                    .await?)
            })
            .await?;

            all_passes.extend(list.game_passes);

            match list.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => break,
            }
        }

        Ok(all_passes)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_game_pass(
        &self,
        name: &str,
        description: Option<&str>,
        price: Option<u64>,
        icon_path: Option<&Path>,
        is_for_sale: bool,
        is_regional_pricing_enabled: bool,
    ) -> Result<GamePass> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!(
                "/game-passes/v1/universes/{}/game-passes",
                self.universe_id
            ))
        };

        let mut form = multipart::Form::new()
            .text("name", name.to_string())
            .text("description", description.unwrap_or("").to_string())
            .text("isForSale", is_for_sale.to_string())
            .text(
                "isRegionalPricingEnabled",
                is_regional_pricing_enabled.to_string(),
            );

        if let Some(p) = price {
            form = form.text("price", p.to_string());
        }
        if let Some(path) = icon_path {
            let bytes = rbx_core::image::process_image(path, self.bleed)?;
            let part = multipart::Part::bytes(bytes)
                .file_name("icon.png")
                .mime_str("image/png")?;
            form = form.part("imageFile", part);
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

    #[allow(clippy::too_many_arguments)]
    pub async fn update_game_pass(
        &self,
        id: u64,
        name: &str,
        description: Option<&str>,
        price: Option<u64>,
        icon_path: Option<&Path>,
        is_for_sale: bool,
        is_regional_pricing_enabled: bool,
    ) -> Result<GamePass> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!(
                "/game-passes/v1/universes/{}/game-passes/{}",
                self.universe_id, id
            ))
        };

        let mut form = multipart::Form::new()
            .text("name", name.to_string())
            .text("description", description.unwrap_or("").to_string())
            .text("isForSale", is_for_sale.to_string())
            .text(
                "isRegionalPricingEnabled",
                is_regional_pricing_enabled.to_string(),
            );

        if let Some(p) = price {
            form = form.text("price", p.to_string());
        }
        if let Some(path) = icon_path {
            let bytes = rbx_core::image::process_image(path, self.bleed)?;
            let part = multipart::Part::bytes(bytes)
                .file_name("icon.png")
                .mime_str("image/png")?;
            form = form.part("file", part);
        }

        let response = self
            .client
            .patch(&url)
            .header("x-api-key", &api_key)
            .multipart(form)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(ApiError::new(status, body).into());
        }

        // Update returns 204 No Content, so body may be empty
        if body.is_empty() {
            // Fetch the updated pass to return it
            return self.get_game_pass(id).await;
        }

        Ok(serde_json::from_str(&body)?)
    }

    pub async fn get_game_pass(&self, id: u64) -> Result<GamePass> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!(
                "/game-passes/v1/universes/{}/game-passes/{}/creator",
                self.universe_id, id
            ))
        };

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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::api::RbxClient;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 66778899001;

    /// Reserved characters on purpose. The token is Roblox's to shape, and
    /// wiremock decodes the query the way the real server does, so a token
    /// pasted rather than encoded arrives as something else and the second
    /// page stops matching. A plain `page2` would walk the loop without ever
    /// proving the encoding.
    const PAGE_TOKEN: &str = "a+b/c=d&e f";

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(Some("test-key".into()), UNIVERSE, false).with_base_url(server.uri())
    }

    /// The page token loop for passes, which had no test at all. A reserved
    /// character in the token used to re-request page one for ever, so a shop
    /// with more than a hundred passes lost every one past the first page,
    /// silently and without an error.
    #[tokio::test]
    async fn listing_follows_the_page_token_until_it_runs_out() {
        let server = MockServer::start().await;
        let list_path = format!("/game-passes/v1/universes/{UNIVERSE}/game-passes/creator");
        Mock::given(method("GET"))
            .and(path(list_path.clone()))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "gamePasses": [{ "gamePassId": 1, "name": "First" }],
                "nextPageToken": PAGE_TOKEN
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(list_path))
            .and(query_param("pageToken", PAGE_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "gamePasses": [{ "gamePassId": 2, "name": "Second" }],
                "nextPageToken": ""
            })))
            .mount(&server)
            .await;

        let passes = client(&server).list_all_game_passes().await.unwrap();

        assert_eq!(passes.len(), 2);
        assert_eq!(passes[1].name.as_deref(), Some("Second"));
        // `gamePassId`, not `id`: the field is renamed on the model, and a
        // fixture using the wrong name would pass every length check while
        // every pass came back with no id at all.
        assert_eq!(passes[1].id, Some(2));
    }
}
