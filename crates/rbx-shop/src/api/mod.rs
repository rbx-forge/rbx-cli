pub mod badges;
pub mod models;
pub mod passes;
pub mod products;

use anyhow::{bail, Result};
use reqwest::Client;

use rbx_core::api::ApiBase;
use rbx_core::owner::OwnerType;

use colored::Colorize;
use models::AssetDeliveryResponse;

/// Named after the `Hosts` field it feeds, because `rbx-spec-drift` resolves
/// which host a `.join(...)` reaches by looking up `const <RECEIVER>_HOST`
/// from the receiver on the same line.
const BADGES_HOST: &str = "https://badges.roblox.com";

/// Named the same way, for the same reason.
const THUMBNAILS_HOST: &str = "https://thumbnails.roblox.com";

/// What `download_icon` asks Roblox for.
///
/// 512x512 rather than the 700x700 the service will also serve. Measured on
/// two real pass icons: upscaling the 150x150 rendition to 700x700 reproduced
/// the real 700x700 exactly for one of them and differed for the other, so the
/// extra resolution is genuine only when the original upload exceeded 512.
/// Asking for the maximum everywhere would bloat every icon whose master is
/// smaller — and these files are committed, then re-uploaded on the next sync.
const ICON_SIZE: &str = "512x512";

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
    /// `thumbnails.roblox.com` — reading an icon back. Public, no key.
    pub(crate) thumbnails: ApiBase,
}

impl Default for Hosts {
    fn default() -> Self {
        Self {
            cloud: ApiBase::default(),
            badges: ApiBase::new(BADGES_HOST),
            thumbnails: ApiBase::new(THUMBNAILS_HOST),
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
            badges: ApiBase::new(url.clone()),
            thumbnails: ApiBase::new(url),
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

    /// Download a resource's icon.
    ///
    /// `asset-delivery` first, because it returns the asset **as stored** —
    /// its true resolution, whatever that is. The thumbnail service is a
    /// fallback rather than an upgrade, and the difference is worth recording
    /// because it is not what the byte counts suggest.
    ///
    /// Measured on two real game pass icons, comparing pixels rather than file
    /// sizes:
    ///
    /// | | asset-delivery vs thumbnail, same size | asset-delivery upscaled vs thumbnail 512 |
    /// | --- | --- | --- |
    /// | pass A | 0.50/255 | **0.04/255** |
    /// | pass B | 3.72/255 | 1.21/255 |
    ///
    /// For pass A the 512x512 thumbnail is a pure upscale of the same 150x150
    /// image: nineteen times the bytes for no extra pixel. PNG is lossless, so
    /// the file-size gap that first suggested "better quality" was only the
    /// cost of storing an enlargement. Preferring it would bloat every icon
    /// whose master is small — and these files are committed, then re-uploaded
    /// on the next sync, which would push an upscale back to Roblox.
    ///
    /// The thumbnail path stays as the fallback: it needs no API key, so it
    /// still works when `legacy-asset:manage` is missing or when Roblox
    /// refuses the delivery endpoint. `legacy-asset:manage` is therefore the
    /// scope to have, and its absence degrades rather than breaks.
    pub async fn download_icon(&self, asset_id: u64) -> Result<Vec<u8>> {
        match self.icon_from_asset_delivery(asset_id).await {
            Ok(bytes) => Ok(bytes),
            Err(delivery_err) => {
                // Say which path was taken: the fallback can hand back an
                // upscaled rendition rather than the stored asset, and that
                // difference gets hashed into the lockfile.
                println!(
                    "  {} asset-delivery unavailable for asset {asset_id} ({delivery_err}); falling back to the thumbnail service, which may return a rescaled rendition",
                    "!".yellow()
                );
                self.icon_from_thumbnails(asset_id).await.map_err(|e| {
                    anyhow::anyhow!(
                        "both icon sources failed for asset {asset_id}. asset-delivery: {delivery_err} (needs `legacy-asset:manage` on the key). thumbnails: {e}"
                    )
                })
            }
        }
    }

    /// The fallback: a rendition at [`ICON_SIZE`], no key required.
    async fn icon_from_thumbnails(&self, asset_id: u64) -> Result<Vec<u8>> {
        let url = {
            let thumbnails = &self.hosts.thumbnails;
            thumbnails.join(&format!(
                "/v1/assets?assetIds={asset_id}&size={ICON_SIZE}&format=Png&isCircular=false"
            ))
        };
        let resp: ThumbnailBatch =
            rbx_core::api::execute_json(|| async { Ok(self.client.get(&url).send().await?) })
                .await?;

        let entry = resp
            .data
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no thumbnail returned"))?;
        // `state` is `Pending` while Roblox renders one for the first time, and
        // `Blocked` when moderation has taken it. Neither is a URL, and writing
        // the empty string as a .png is how a corrupt icon gets hashed into the
        // lockfile.
        let location = match (entry.state.as_deref(), entry.image_url) {
            (Some("Completed"), Some(u)) if !u.is_empty() => u,
            (state, _) => bail!("state is {}", state.unwrap_or("unknown")),
        };
        self.fetch_cdn(asset_id, &location).await
    }

    /// The preferred source: the asset as stored, behind `legacy-asset:manage`.
    async fn icon_from_asset_delivery(&self, asset_id: u64) -> Result<Vec<u8>> {
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
        self.fetch_cdn(asset_id, &resp.location).await
    }

    /// Both sources hand back a CDN url; this is the hop neither can skip.
    async fn fetch_cdn(&self, asset_id: u64, location: &str) -> Result<Vec<u8>> {
        let resp = self.client.get(location).send().await?;
        let status = resp.status();
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
        Ok(resp.bytes().await?.to_vec())
    }
}

/// One entry of `thumbnails/v1/assets`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thumbnail {
    /// `Completed`, `Pending`, `Blocked`. Absent on shapes this has not met.
    state: Option<String>,
    image_url: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ThumbnailBatch {
    data: Vec<Thumbnail>,
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
