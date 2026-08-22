use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::RbxClient;

// The baseplate template is the default for create_universe / create_place
// here and the whole of what `rbx open --new` opens, so it lives in rbx-core
// and is re-exported rather than written down twice.
pub use rbx_core::templates::DEFAULT_TEMPLATE_PLACE_ID;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUniverseResponse {
    pub universe_id: u64,
    pub root_place_id: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupUniverseEntry {
    pub id: u64,
    pub name: String,
    pub root_place: RootPlace,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RootPlace {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorPage<T> {
    data: Vec<T>,
    next_page_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct UniverseInfo {
    root_place_id: u64,
}

impl RbxClient {
    pub async fn create_universe(
        &self,
        template_place_id: Option<u64>,
        group_id: Option<u64>,
    ) -> Result<CreateUniverseResponse> {
        let template = template_place_id.unwrap_or(DEFAULT_TEMPLATE_PLACE_ID);
        let mut url = self.hosts().apis.join("/universes/v1/universes/create");
        if let Some(gid) = group_id {
            url.push_str(&format!("?groupId={}", gid));
        }
        let body = serde_json::json!({ "templatePlaceId": template });
        self.auth_json(reqwest::Method::POST, &url, Some(body))
            .await
            .context("Failed to create universe")
    }

    /// Fetch the root place ID of a universe.
    ///
    /// Cookie-aware, but not cookie-dependent: measured, this endpoint answers
    /// 200 on a private universe with no credential at all. The neighbouring
    /// `/v1/places/{id}` is the one that answers 404 anonymously.
    pub async fn get_universe_root_place(&self, universe_id: u64) -> Result<u64> {
        // The base is bound first so that the receiver sits on the same line as
        // `.join(`: `rbx-spec-drift` reads it from there to work out which host
        // this path belongs to, and a chain rustfmt has split across lines
        // reads as the default host instead.
        let develop = &self.hosts().develop;
        let url = develop.join(&format!("/v1/universes/{universe_id}"));
        let cookie = self.optional_cookie_header();
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
        let info: UniverseInfo = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Failed to parse universe info: {}\nBody: {}", e, body))?;
        if info.root_place_id == 0 {
            bail!(
                "Could not resolve root place ID for universe {} (check the ID and your cookie).",
                universe_id
            );
        }
        Ok(info.root_place_id)
    }

    pub async fn list_group_universes(&self, group_id: u64) -> Result<Vec<GroupUniverseEntry>> {
        let cookie = self.optional_cookie_header();
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut url = self.hosts().games.join(&format!(
                "/v2/groups/{group_id}/gamesV2?accessFilter=1&limit=100"
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
            let page: CursorPage<GroupUniverseEntry> = serde_json::from_str(&body)
                .map_err(|e| anyhow::anyhow!("Failed to parse universes: {}\nBody: {}", e, body))?;

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

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(Some("test-cookie".into())).with_base_url(server.uri())
    }

    /// Group ownership is a query parameter, not a body field. Dropping it
    /// creates the universe under the signed-in user instead of the group:
    /// a real resource in the wrong place, and not movable afterwards.
    #[tokio::test]
    async fn creating_under_a_group_puts_the_group_in_the_query() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/universes/v1/universes/create"))
            .and(query_param("groupId", "42"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "universeId": 1, "rootPlaceId": 2 })),
            )
            .mount(&server)
            .await;

        let created = client(&server)
            .create_universe(None, Some(42))
            .await
            .unwrap();
        assert_eq!(created.universe_id, 1);
    }

    #[tokio::test]
    async fn creating_without_a_group_sends_no_group_id_at_all() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/universes/v1/universes/create"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "universeId": 9, "rootPlaceId": 8 })),
            )
            .mount(&server)
            .await;

        client(&server).create_universe(None, None).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert!(
            !requests[0]
                .url
                .query()
                .unwrap_or_default()
                .contains("groupId"),
            "no group means no groupId, not groupId=0"
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
        let list_path = "/v2/groups/42/gamesV2";

        Mock::given(method("GET"))
            .and(path(list_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 1, "name": "First", "rootPlace": { "id": 11 } }],
                "nextPageCursor": CURSOR
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(list_path))
            .and(query_param("cursor", CURSOR))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 2, "name": "Second", "rootPlace": { "id": 22 } }],
                "nextPageCursor": ""
            })))
            .mount(&server)
            .await;

        let universes = client(&server).list_group_universes(42).await.unwrap();

        assert_eq!(universes.len(), 2);
        assert_eq!(universes[1].name, "Second");
    }

    #[tokio::test]
    async fn the_root_place_is_read_from_the_develop_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/universes/777"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "rootPlaceId": 555 })))
            .mount(&server)
            .await;

        assert_eq!(
            client(&server).get_universe_root_place(777).await.unwrap(),
            555
        );
    }
}
