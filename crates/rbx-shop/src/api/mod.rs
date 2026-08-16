pub mod badges;
pub mod models;
pub mod passes;
pub mod products;

use anyhow::{bail, Result};
use reqwest::Client;

use rbx_core::api::ApiBase;
use rbx_core::owner::OwnerType;

use models::AssetDeliveryResponse;

/// Named after the `Hosts` field it feeds, because `rbx-spec-drift` resolves
/// which host a `.join(...)` reaches by looking up `const <RECEIVER>_HOST`
/// from the receiver on the same line.
const BADGES_HOST: &str = "https://badges.roblox.com";

/// The two hosts this crate reaches.
///
/// Injectable so the request shaping can run against a mock server. Until this
/// existed the URLs were built inline and nothing here had been tested over
/// HTTP — including the calls that create developer products, which cost real
/// money to get wrong.
#[derive(Debug)]
pub(crate) struct Hosts {
    /// `apis.roblox.com` — passes, products, badge configuration.
    pub(crate) cloud: ApiBase,
    /// `badges.roblox.com` — badge creation and its icon upload.
    pub(crate) badges: ApiBase,
}

impl Default for Hosts {
    fn default() -> Self {
        Self {
            cloud: ApiBase::default(),
            badges: ApiBase::new(BADGES_HOST),
        }
    }
}

#[derive(Debug)]
pub struct RbxClient {
    client: Client,
    api_key: Option<String>,
    universe_id: u64,
    bleed: bool,
    pub(crate) hosts: Hosts,
}

impl RbxClient {
    pub fn new(api_key: Option<String>, universe_id: u64, bleed: bool) -> Self {
        Self {
            client: rbx_core::api::build_client(),
            api_key,
            universe_id,
            bleed,
            hosts: Hosts::default(),
        }
    }

    /// Point both hosts at one server. Tests only.
    ///
    /// `cfg(test)` rather than `pub`: `RbxClient` is reachable inside the
    /// crate only, so outside the test build this would be dead code under
    /// `-D warnings`.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        self.hosts = Hosts {
            cloud: ApiBase::new(url.clone()),
            badges: ApiBase::new(url),
        };
        self
    }

    /// API key header for Open Cloud endpoints.
    pub fn api_key_header(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!("--api-key or RBX_API_KEY env var is required for this operation")
        })
    }

    /// Who owns this universe, according to Roblox.
    ///
    /// The payment source for a badge follows from ownership and nothing else,
    /// so this is the fact that used to be typed into `[creator]` by hand.
    /// Asking removes a whole class of wrong answer: a config that says `user`
    /// for a group-owned game is a badge create Roblox refuses.
    ///
    /// `Ok(None)` rather than an error when Roblox answers without either
    /// field, and the caller treats a failed call as "no answer" too — this is
    /// a convenience over a declaration that still works, not a new hard
    /// requirement. `universe:read` is what it needs.
    pub async fn universe_owner(&self) -> Result<Option<OwnerType>> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!("/cloud/v2/universes/{}", self.universe_id))
        };
        let universe: CloudUniverseOwner = rbx_core::api::execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await?;
        // A universe has one or the other. If Roblox ever sends both, the group
        // is the one that owns the balance.
        Ok(match (universe.user, universe.group) {
            (_, Some(_)) => Some(OwnerType::Group),
            (Some(_), None) => Some(OwnerType::User),
            (None, None) => None,
        })
    }

    /// Download an asset's raw bytes from Roblox via the asset delivery API.
    pub async fn download_asset(&self, asset_id: u64) -> Result<Vec<u8>> {
        let api_key = self.api_key_header()?.to_string();
        let url = {
            let cloud = &self.hosts.cloud;
            cloud.join(&format!("/asset-delivery-api/v1/assetId/{}", asset_id))
        };
        let resp: AssetDeliveryResponse = rbx_core::api::execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await?;

        let cdn_resp = self.client.get(&resp.location).send().await?;
        let status = cdn_resp.status();
        if !status.is_success() {
            // The CDN location can be expired/locked or hit a transient error.
            // Without this guard the error body gets written verbatim as the
            // asset file (e.g. a corrupt .png) and hashed into the lockfile.
            bail!(
                "Asset download failed for {}: CDN returned {}",
                asset_id,
                status
            );
        }
        let bytes = cdn_resp.bytes().await?;
        Ok(bytes.to_vec())
    }
}

/// Only the two fields that say who owns a universe. `rbx-meta` models the
/// rest of this payload; a second full copy here would be two places to update
/// when Roblox adds a field.
#[derive(Debug, serde::Deserialize)]
struct CloudUniverseOwner {
    /// `users/123` when a user owns it.
    user: Option<String>,
    /// `groups/456` when a group does.
    group: Option<String>,
}
