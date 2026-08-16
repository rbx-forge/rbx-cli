use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::universes::DEFAULT_TEMPLATE_PLACE_ID;
use super::RbxClient;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaceResponse {
    pub place_id: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceEntry {
    #[serde(default, alias = "placeId")]
    pub id: u64,
    #[serde(default, alias = "displayName")]
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevelopPlacesPage {
    data: Vec<PlaceEntry>,
    next_page_cursor: Option<String>,
}

impl RbxClient {
    pub async fn create_place(
        &self,
        universe_id: u64,
        template_place_id: Option<u64>,
    ) -> Result<CreatePlaceResponse> {
        let template = template_place_id.unwrap_or(DEFAULT_TEMPLATE_PLACE_ID);
        let url = self.hosts().apis.join(&format!(
            "/universes/v1/user/universes/{universe_id}/places"
        ));
        let body = serde_json::json!({ "templatePlaceId": template });
        self.auth_json(reqwest::Method::POST, &url, Some(body))
            .await
            .context("Failed to create place")
    }

    /// PATCH the place's display name. Cookie + CSRF required.
    /// "Renaming a universe" really means renaming its root place, since the universe's
    /// public display name is derived from the root place.
    pub async fn rename_place(&self, place_id: u64, name: &str) -> Result<()> {
        let url = self.hosts().develop.join(&format!("/v2/places/{place_id}"));
        let body = serde_json::json!({ "name": name });
        let _: serde_json::Value = self
            .auth_json(reqwest::Method::PATCH, &url, Some(body))
            .await
            .with_context(|| format!("Failed to rename place {}", place_id))?;
        Ok(())
    }

    pub async fn list_universe_places(&self, universe_id: u64) -> Result<Vec<PlaceEntry>> {
        let cookie = self.optional_cookie_header();
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut url = self.hosts().develop.join(&format!(
                "/v1/universes/{universe_id}/places?isUniverseCreation=false&limit=100&sortOrder=Asc"
            ));
            if let Some(c) = &cursor {
                // Encoded, not pasted: the cursor is an opaque token and a `+`
                // or `&` in it would silently re-request page one for ever.
                url.push_str("&cursor=");
                url.push_str(&rbx_core::api::encode_query_value(c));
            }

            let cookie_ref = cookie.as_deref();
            let response = self
                .execute_public(|| async {
                    let mut req = self.client.get(&url);
                    if let Some(c) = cookie_ref {
                        req = req.header(reqwest::header::COOKIE, c);
                    }
                    Ok(req.send().await?)
                })
                .await?;
            let body = response.text().await?;
            let page: DevelopPlacesPage = serde_json::from_str(&body)
                .map_err(|e| anyhow::anyhow!("Failed to parse places: {}\nBody: {}", e, body))?;

            all.extend(page.data);

            match page.next_page_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
        }

        Ok(all)
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

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(Some("test-cookie".into())).with_base_url(server.uri())
    }

    /// The cursor loop, which had never run: most universes fit in one page,
    /// so a broken second page would only show up on a large one.
    #[tokio::test]
    async fn listing_follows_the_cursor_until_it_runs_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/universes/{UNIVERSE}/places")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 1, "name": "Start" }],
                "nextPageCursor": "page2"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/universes/{UNIVERSE}/places")))
            .and(query_param("cursor", "page2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 2, "name": "Second" }],
                "nextPageCursor": null
            })))
            .mount(&server)
            .await;

        let places = client(&server)
            .list_universe_places(UNIVERSE)
            .await
            .unwrap();

        assert_eq!(places.len(), 2);
        assert_eq!(places[1].name, "Second");
    }

    /// Roblox ends a listing with `""` on some endpoints and `null` on others.
    /// Treating the empty string as a cursor fetches page one forever.
    #[tokio::test]
    async fn an_empty_cursor_ends_the_listing_rather_than_repeating_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/universes/{UNIVERSE}/places")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 1, "name": "Only" }],
                "nextPageCursor": ""
            })))
            .mount(&server)
            .await;

        let places = client(&server)
            .list_universe_places(UNIVERSE)
            .await
            .unwrap();

        assert_eq!(places.len(), 1);
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "an empty cursor means stop"
        );
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

        let places = client(&server)
            .list_universe_places(UNIVERSE)
            .await
            .unwrap();

        assert_eq!(places.len(), 2);
        assert_eq!(places[1].name, "Second");
    }

    /// `placeId`/`displayName` is what `develop.roblox.com` sends; the aliases
    /// on `PlaceEntry` exist for it, and without them every place reads as
    /// id 0 with an empty name.
    #[tokio::test]
    async fn the_develop_field_names_are_accepted_through_their_aliases() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/universes/{UNIVERSE}/places")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "placeId": 77, "displayName": "Aliased" }],
                "nextPageCursor": null
            })))
            .mount(&server)
            .await;

        let places = client(&server)
            .list_universe_places(UNIVERSE)
            .await
            .unwrap();

        assert_eq!(places[0].id, 77);
        assert_eq!(places[0].name, "Aliased");
    }
}
