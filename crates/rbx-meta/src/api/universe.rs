use anyhow::Result;
use rbx_core::api::ApiError;
use serde_json::Value;

use super::models::Universe;
use super::RbxClient;

fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|l| format!("{}{}", prefix, l))
        .collect::<Vec<_>>()
        .join("\n")
}

impl RbxClient {
    pub async fn get_universe(&self) -> Result<Universe> {
        let api_key = self.api_key_header()?.to_string();
        let url = self.api_url(&format!("/cloud/v2/universes/{}", self.universe_id));

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

    /// PATCH the universe with the given body and update mask.
    ///
    /// `body` must be a JSON object containing the fields to update; use explicit
    /// `null` to clear a field (e.g., removing a social link). `update_mask` must
    /// list every field included in the body (or being cleared).
    pub async fn patch_universe(&self, body: Value, update_mask: &[&str]) -> Result<Universe> {
        let api_key = self.api_key_header()?.to_string();
        let mask = update_mask.join(",");
        let url = self.api_url(&format!(
            "/cloud/v2/universes/{}?updateMask={}",
            self.universe_id, mask
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
            let mut hint = String::new();
            // Roblox returns 500 INTERNAL (instead of a 4xx) when paid private
            // servers are requested on a private experience. The local preflight
            // can't catch this if the user's config says public but the actual
            // remote is still private (drift).
            if status.as_u16() == 500 && body.get("privateServerPriceRobux").is_some() {
                hint = format!(
                        "\n  Open Cloud masks the real Roblox error here. To see it, open Creator Hub:\n    \
                         https://create.roblox.com/dashboard/creations/experiences/{}/access\n    \
                         and try the change manually.\n  \
                         Common reasons Roblox refuses this PATCH:\n    \
                         • 60-day cooldown after the last price change (just wait it out).\n    \
                         • Experience is PRIVATE (paid private servers need PUBLIC visibility).\n    \
                         • Creator not eligible (must be ID-verified or have made a real-money purchase since 2025-01-01).",
                        self.universe_id
                    );
            }
            // As in places.rs: the request goes in the context, the response
            // stays on the ApiError that anyhow prints as the cause. The hint
            // belongs with the context — it is advice about the request we
            // made, not about the bytes that came back.
            return Err(
                anyhow::Error::from(ApiError::new(status, resp_body)).context(format!(
                    "PATCH {}\n  mask: {}\n  request body:\n{}{}",
                    url,
                    mask,
                    indent(&req_pretty, "    "),
                    hint,
                )),
            );
        }

        let parsed: Universe = serde_json::from_str(&resp_body)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}\nBody: {}", e, resp_body))?;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::api::RbxClient;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 66778899001;

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(
            Some("test-key".into()),
            None,
            UNIVERSE,
            1,
            false,
            "en-us".into(),
        )
        .with_base_url(server.uri())
    }

    #[tokio::test]
    async fn get_reads_the_cloud_v2_path_with_the_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/cloud/v2/universes/{UNIVERSE}")))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "displayName": "Test Experience",
                "visibility": "PRIVATE"
            })))
            .mount(&server)
            .await;

        let universe = client(&server).get_universe().await.unwrap();
        assert_eq!(universe.display_name.as_deref(), Some("Test Experience"));
    }

    /// The mask is not decoration: Roblox ignores any field the body carries
    /// that the mask does not name, so a wrong one is a write that reports
    /// success and changes nothing.
    #[tokio::test]
    async fn patch_sends_the_body_and_names_every_field_in_the_mask() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/cloud/v2/universes/{UNIVERSE}")))
            .and(query_param("updateMask", "displayName,description"))
            .and(body_json(
                json!({ "displayName": "New", "description": "Also new" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "displayName": "New" })))
            .mount(&server)
            .await;

        let updated = client(&server)
            .patch_universe(
                json!({ "displayName": "New", "description": "Also new" }),
                &["displayName", "description"],
            )
            .await
            .unwrap();
        assert_eq!(updated.display_name.as_deref(), Some("New"));
    }

    /// Clearing a field is an explicit `null` plus its name in the mask.
    /// Either half alone leaves the field as it was.
    #[tokio::test]
    async fn patch_clears_a_field_with_an_explicit_null() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/cloud/v2/universes/{UNIVERSE}")))
            .and(query_param("updateMask", "facebookSocialLink"))
            .and(body_json(json!({ "facebookSocialLink": null })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        client(&server)
            .patch_universe(
                json!({ "facebookSocialLink": null }),
                &["facebookSocialLink"],
            )
            .await
            .unwrap();
    }

    /// The everyday failure — a key without `universe:write` — has to arrive
    /// as an error rather than as a success over an unchanged universe.
    #[tokio::test]
    async fn a_rejected_patch_is_an_error_not_a_silent_no_op() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/cloud/v2/universes/{UNIVERSE}")))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "message": "Insufficient scope"
            })))
            .mount(&server)
            .await;

        let error = client(&server)
            .patch_universe(json!({ "displayName": "New" }), &["displayName"])
            .await
            .unwrap_err();

        // `{:?}` rather than `{}`: the top-level context is the request that
        // failed — method, mask and body, which is what you need to see — and
        // the status lives further down the chain.
        let chain = format!("{error:?}").to_lowercase();
        assert!(
            chain.contains("403"),
            "the status should survive: {error:?}"
        );
        assert!(
            chain.contains("updatemask=displayname"),
            "the failing request should stay visible: {error:?}"
        );
    }

    /// A missing credential is reported as itself rather than as a 401 from
    /// Roblox, which means no request goes out at all.
    #[tokio::test]
    async fn a_missing_api_key_fails_before_any_request() {
        let server = MockServer::start().await;
        let error = RbxClient::new(None, None, UNIVERSE, 1, false, "en-us".into())
            .with_base_url(server.uri())
            .get_universe()
            .await
            .unwrap_err();

        assert!(error.to_string().contains("api-key"), "got: {error}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
