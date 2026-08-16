//! Public games / develop endpoints used for universe → owner resolution.

use anyhow::{bail, Result};
use rbx_core::owner::OwnerType;
use reqwest::header;
use serde::Deserialize;

use crate::lock::UniverseOwner;

use super::RbxApiKeyClient;

#[derive(Debug, Deserialize)]
struct GamesResponse {
    #[serde(default)]
    data: Vec<GameEntry>,
}

#[derive(Debug, Deserialize)]
struct GameEntry {
    #[serde(default)]
    creator: Option<GameCreator>,
}

#[derive(Debug, Deserialize)]
struct GameCreator {
    #[serde(default, rename = "type")]
    ty: String,
    #[serde(default)]
    id: u64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DevelopUniverseInfo {
    creator_type: String,
    creator_target_id: u64,
}

impl RbxApiKeyClient {
    pub async fn fetch_universe_owner(&self, universe_id: u64) -> Result<UniverseOwner> {
        // Public games API first (no auth required for public universes).
        let public_url = format!(
            "https://games.roblox.com/v1/games?universeIds={}",
            universe_id
        );
        let resp = self.client.get(&public_url).send().await?;
        if resp.status().is_success() {
            let body = resp.text().await?;
            if let Ok(data) = serde_json::from_str::<GamesResponse>(&body) {
                if let Some(game) = data.data.into_iter().next() {
                    if let Some(creator) = game.creator {
                        if creator.id != 0 {
                            let owner_type = if creator.ty.eq_ignore_ascii_case("Group") {
                                OwnerType::Group
                            } else {
                                OwnerType::User
                            };
                            return Ok(UniverseOwner {
                                universe_id,
                                owner_type,
                                owner_id: creator.id,
                            });
                        }
                    }
                }
            }
        }

        // Private universe → develop API with cookie.
        let cookie = self.cookie_header()?;
        let url = format!("https://develop.roblox.com/v1/universes/{}", universe_id);
        let resp = self
            .client
            .get(&url)
            .header(header::COOKIE, &cookie)
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!(
                "universe {}: not found publicly and develop API returned {}",
                universe_id,
                status
            );
        }
        let info: DevelopUniverseInfo = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!(
                "universe {}: failed to parse develop response: {}\nBody: {}",
                universe_id,
                e,
                body
            )
        })?;
        if info.creator_target_id == 0 {
            bail!(
                "universe {}: develop API response missing creatorTargetId",
                universe_id
            );
        }
        let owner_type = if info.creator_type.eq_ignore_ascii_case("Group") {
            OwnerType::Group
        } else {
            OwnerType::User
        };
        Ok(UniverseOwner {
            universe_id,
            owner_type,
            owner_id: info.creator_target_id,
        })
    }
}
