use anyhow::Result;
use rbx_core::api::ApiError;
use serde_json::Value;

use super::models::Place;
use super::RbxClient;

impl RbxClient {
    pub async fn get_place(&self) -> Result<Place> {
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!(
            "/cloud/v2/universes/{}/places/{}",
            self.universe_id, self.place_id
        ));

        self.execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await
    }

    /// PATCH the place with the given body and update mask.
    pub async fn patch_place(&self, body: Value, update_mask: &[&str]) -> Result<Place> {
        let api_key = self.api_key_header()?.to_string();
        let mask = update_mask.join(",");
        let url = self.api_url(&format!(
            "/cloud/v2/universes/{}/places/{}?updateMask={}",
            self.universe_id, self.place_id, mask
        ));

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
            let req_pretty =
                serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());
            // What we sent goes in the context; what came back stays on the
            // ApiError underneath, which anyhow prints as the cause. Repeating
            // the response in both would just make a long message longer.
            return Err(
                anyhow::Error::from(ApiError::new(status, resp_body)).context(format!(
                    "PATCH {}\n  mask: {}\n  request body:\n{}",
                    url,
                    mask,
                    indent(&req_pretty, "    ")
                )),
            );
        }

        let parsed: Place = serde_json::from_str(&resp_body)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}\nBody: {}", e, resp_body))?;
        Ok(parsed)
    }
}

fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|l| format!("{}{}", prefix, l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::api::RbxClient;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 66778899001;
    const PLACE: u64 = 77889900112233;

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(
            Some("test-key".into()),
            None,
            UNIVERSE,
            PLACE,
            false,
            "en-us".into(),
        )
        .with_base_url(server.uri())
    }

    /// A place is addressed under its universe, not on its own. Getting that
    /// wrong reads somebody else's place if the ids happen to resolve.
    #[tokio::test]
    async fn get_reads_the_place_under_its_universe() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/cloud/v2/universes/{UNIVERSE}/places/{PLACE}"
            )))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "displayName": "Main",
                "serverSize": 50
            })))
            .mount(&server)
            .await;

        let place = client(&server).get_place().await.unwrap();
        assert_eq!(place.display_name.as_deref(), Some("Main"));
        assert_eq!(place.server_size, Some(50));
    }

    #[tokio::test]
    async fn patch_carries_its_own_mask() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!(
                "/cloud/v2/universes/{UNIVERSE}/places/{PLACE}"
            )))
            .and(query_param("updateMask", "serverSize"))
            .and(body_json(json!({ "serverSize": 30 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "serverSize": 30 })))
            .mount(&server)
            .await;

        let place = client(&server)
            .patch_place(json!({ "serverSize": 30 }), &["serverSize"])
            .await
            .unwrap();
        assert_eq!(place.server_size, Some(30));
    }

    #[tokio::test]
    async fn a_missing_api_key_fails_before_any_request() {
        let server = MockServer::start().await;
        let error = RbxClient::new(None, None, UNIVERSE, PLACE, false, "en-us".into())
            .with_base_url(server.uri())
            .get_place()
            .await
            .unwrap_err();

        assert!(error.to_string().contains("api-key"), "got: {error}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
