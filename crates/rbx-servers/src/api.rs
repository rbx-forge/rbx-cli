//! `server-management/v1` client.
//!
//! Note the host: this family does not live under `cloud/v2` and is absent from
//! the Open Cloud reference index, so its shapes were established by calling it
//! rather than by reading about it.

use anyhow::Result;
use reqwest::Client;

use rbx_core::api::{
    build_client, encode_query_value, execute_json, explain_missing_scope, ApiBase,
};

use crate::model::{FilterOptions, GameServerLogPage, GameServerPage};

/// Roblox caps this at 100. Asking for more is rejected, not clamped.
pub const MAX_PAGE_SIZE: u32 = 100;

#[derive(Debug, Clone)]
pub struct ServersApi {
    client: Client,
    base: ApiBase,
    api_key: String,
}

impl ServersApi {
    pub fn new(api_key: impl Into<String>, base: ApiBase) -> Self {
        Self {
            client: build_client(),
            base,
            api_key: api_key.into(),
        }
    }

    /// Which place versions have servers.
    ///
    /// Callers need this before anything else: the list endpoint takes a
    /// version in its path and offers no "every version" form.
    pub async fn filter_options(&self, universe_id: u64, place_id: u64) -> Result<FilterOptions> {
        let url = self.base.join(&format!(
            "/server-management/v1/universes/{universe_id}/places/{place_id}/game-servers:filter-options"
        ));
        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    /// One page of servers for one place version.
    pub async fn list_page(
        &self,
        universe_id: u64,
        place_id: u64,
        version: &str,
        page_size: u32,
        page_token: Option<&str>,
    ) -> Result<GameServerPage> {
        let mut url = self.base.join(&format!(
            "/server-management/v1/universes/{universe_id}/places/{place_id}/versions/{version}/game-servers"
        ));
        url.push_str(&format!("?MaxPageSize={}", page_size.min(MAX_PAGE_SIZE)));
        if let Some(token) = page_token {
            url.push_str("&PageToken=");
            url.push_str(&encode_query_value(token));
        }

        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    /// What one server wrote before it stopped.
    ///
    /// The version is part of the path even though a job id already identifies
    /// the server uniquely, so a caller has to carry it through from the row it
    /// found the job id on.
    pub async fn list_logs(
        &self,
        universe_id: u64,
        place_id: u64,
        version: &str,
        job_id: &str,
        page_size: u32,
        page_token: Option<&str>,
    ) -> Result<GameServerLogPage> {
        let mut url = self.base.join(&format!(
            "/server-management/v1/universes/{universe_id}/places/{place_id}\
             /versions/{version}/game-servers/{}/logs",
            encode_query_value(job_id)
        ));
        url.push_str(&format!("?MaxPageSize={}", page_size.min(MAX_PAGE_SIZE)));
        if let Some(token) = page_token {
            url.push_str("&PageToken=");
            url.push_str(&encode_query_value(token));
        }

        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }
}

// Query-value encoding moved to `rbx_core::api::encode_query_value` and is
// tested there: `rbx-ban` pastes pagination tokens into URLs too, and had been
// doing it unencoded.
