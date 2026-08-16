use std::path::Path;

use anyhow::Result;
use rbx_core::api::ApiError;
use reqwest::multipart;

use super::models::{DeveloperProduct, ListDeveloperProductsResponse};
use super::RbxClient;

impl RbxClient {
    pub async fn list_all_developer_products(&self) -> Result<Vec<DeveloperProduct>> {
        let api_key = self.api_key_header()?.to_string();
        let mut all_products = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut url = {
                let cloud = &self.hosts.cloud;
                cloud.join(&format!(
                    "/developer-products/v2/universes/{}/developer-products/creator?pageSize=50",
                    self.universe_id
                ))
            };
            if let Some(token) = &page_token {
                // Encoded, not pasted: the token is opaque and a `+` or `&` in
                // it would silently re-request page one for ever.
                url.push_str("&pageToken=");
                url.push_str(&rbx_core::api::encode_query_value(token));
            }

            let list: ListDeveloperProductsResponse = rbx_core::api::execute_json(|| async {
                Ok(self
                    .client
                    .get(&url)
                    .header("x-api-key", &api_key)
                    .send()
                    .await?)
            })
            .await?;

            all_products.extend(list.developer_products);

            match list.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => break,
            }
        }

        Ok(all_products)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_developer_product(
        &self,
        name: &str,
        description: Option<&str>,
        price: u64,
        icon_path: Option<&Path>,
        is_for_sale: bool,
        is_regional_pricing_enabled: bool,
    ) -> Result<DeveloperProduct> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!(
                "/developer-products/v2/universes/{}/developer-products",
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
            )
            .text("price", price.to_string());

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
    pub async fn update_developer_product(
        &self,
        id: u64,
        name: &str,
        description: Option<&str>,
        price: u64,
        icon_path: Option<&Path>,
        is_for_sale: bool,
        is_regional_pricing_enabled: bool,
        store_page_enabled: bool,
    ) -> Result<DeveloperProduct> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!(
                "/developer-products/v2/universes/{}/developer-products/{}",
                self.universe_id, id
            ))
        };

        // The API validates isForSale against the CURRENT remote state, not the
        // state being sent. So setting isForSale=false while the product is
        // currently on the store page fails with InvalidIsForSale, even if
        // storePageEnabled=false is sent in the same request.
        // Workaround: first remove from store page, then set off sale.
        if !is_for_sale {
            let disable_store_form = multipart::Form::new()
                .text("name", name.to_string())
                .text("description", description.unwrap_or("").to_string())
                .text("isForSale", "true")
                .text(
                    "isRegionalPricingEnabled",
                    is_regional_pricing_enabled.to_string(),
                )
                .text("storePageEnabled", "false")
                .text("price", price.to_string());

            let resp = self
                .client
                .patch(&url)
                .header("x-api-key", &api_key)
                .multipart(disable_store_form)
                .send()
                .await?;

            if !resp.status().is_success() && !resp.status().is_redirection() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                // Context rather than a bespoke message: the status stays
                // recoverable, and "which call failed" is what the sentence
                // was actually adding.
                return Err(anyhow::Error::from(ApiError::new(status, body))
                    .context("disabling the store page"));
            }
        }

        let effective_store_page = store_page_enabled && is_for_sale;

        let mut form = multipart::Form::new()
            .text("name", name.to_string())
            .text("description", description.unwrap_or("").to_string())
            .text("isForSale", is_for_sale.to_string())
            .text(
                "isRegionalPricingEnabled",
                is_regional_pricing_enabled.to_string(),
            )
            .text("storePageEnabled", effective_store_page.to_string())
            .text("price", price.to_string());

        if let Some(path) = icon_path {
            let bytes = rbx_core::image::process_image(path, self.bleed)?;
            let part = multipart::Part::bytes(bytes)
                .file_name("icon.png")
                .mime_str("image/png")?;
            form = form.part("imageFile", part);
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

        if body.is_empty() {
            return self.get_developer_product(id).await;
        }

        Ok(serde_json::from_str(&body)?)
    }

    pub async fn get_developer_product(&self, id: u64) -> Result<DeveloperProduct> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!(
                "/developer-products/v2/universes/{}/developer-products/{}/creator",
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
    /// pasted rather than encoded arrives as something else and this fixture
    /// stops matching. A plain `page2` would walk the loop without ever
    /// proving the encoding.
    const PAGE_TOKEN: &str = "a+b/c=d&e f";

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(Some("test-key".into()), UNIVERSE, false).with_base_url(server.uri())
    }

    /// The page token loop, which had never run. Most catalogues fit in one
    /// page of fifty, so a broken second page only shows up on a large shop —
    /// and shows up as products silently missing from a diff.
    #[tokio::test]
    async fn listing_follows_the_page_token_until_it_runs_out() {
        let server = MockServer::start().await;
        let list_path =
            format!("/developer-products/v2/universes/{UNIVERSE}/developer-products/creator");
        Mock::given(method("GET"))
            .and(path(list_path.clone()))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "developerProducts": [{ "productId": 1, "name": "First" }],
                "nextPageToken": PAGE_TOKEN
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(list_path))
            .and(query_param("pageToken", PAGE_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "developerProducts": [{ "productId": 2, "name": "Second" }],
                "nextPageToken": ""
            })))
            .mount(&server)
            .await;

        let products = client(&server).list_all_developer_products().await.unwrap();

        assert_eq!(products.len(), 2);
        assert_eq!(products[1].name.as_deref(), Some("Second"));
        // `productId`, not `id`: the field is renamed on the model, and a
        // fixture using the wrong name would pass every length check while
        // every product came back with no id at all.
        assert_eq!(products[1].id, Some(2));
    }

    /// `""` is how this endpoint says "no more pages", where others use `null`.
    /// Treating it as a real token fetches page one forever.
    #[tokio::test]
    async fn an_empty_page_token_ends_the_listing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/developer-products/v2/universes/{UNIVERSE}/developer-products/creator"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "developerProducts": [{ "productId": 1, "name": "Only" }],
                "nextPageToken": ""
            })))
            .mount(&server)
            .await;

        let products = client(&server).list_all_developer_products().await.unwrap();

        assert_eq!(products.len(), 1);
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "an empty token means stop"
        );
    }

    /// Creating a developer product is the one call in this workspace that
    /// puts something up for sale. It must go to the universe's own path, as a
    /// POST, carrying the key.
    #[tokio::test]
    async fn creating_a_product_posts_to_the_universes_own_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/developer-products/v2/universes/{UNIVERSE}/developer-products"
            )))
            .and(header("x-api-key", "test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "productId": 99, "name": "Gems" })),
            )
            .mount(&server)
            .await;

        let created = client(&server)
            .create_developer_product("Gems", None, 100, None, true, false)
            .await
            .unwrap();
        assert_eq!(created.id, Some(99));
    }

    /// A refused create must not read as a success with a missing id: the
    /// caller writes the returned id into the lockfile, and a zero there
    /// detaches the entry from the real product for good.
    #[tokio::test]
    async fn a_refused_create_is_an_error_rather_than_a_product_without_an_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/developer-products/v2/universes/{UNIVERSE}/developer-products"
            )))
            .respond_with(ResponseTemplate::new(403).set_body_string("no scope"))
            .mount(&server)
            .await;

        assert!(client(&server)
            .create_developer_product("Gems", None, 100, None, true, false)
            .await
            .is_err());
    }
}
