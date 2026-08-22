//! Legacy cookie-based endpoints for fields that Open Cloud v2 does not expose yet
//! (server_fill / customSocialSlotsCount, allow_copying, ...).
//!
//! These hit `https://develop.roblox.com/v2/places/{id}` with an `.ROBLOSECURITY`
//! cookie. The first state-changing request returns 403 with an `x-csrf-token`
//! header; we cache it and retry transparently.

use anyhow::{Context, Result};
use rbx_core::api::roblox_error;
use reqwest::{header, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;

use super::RbxClient;

// Paths, not URLs: the host comes from `RbxClient::legacy_url` so a test can
// point it somewhere else. They were absolute until the visibility ordering in
// `sync` needed a mock server to assert against.
const PLACE_LEGACY_PATH: &str = "/v2/places";
const UNIVERSE_LEGACY_PATH: &str = "/v1/universes";
const UNIVERSE_CONFIG_PATH: &str = "/v2/universes";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlaceLegacy {
    pub id: Option<u64>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub max_player_count: Option<u32>,
    /// "Automatic" | "Empty" | "Custom"
    pub social_slot_type: Option<String>,
    #[serde(alias = "customSocialSlotCount")]
    pub custom_social_slots_count: Option<u32>,
    pub copying_allowed: Option<bool>,
}

/// What `GET /v1/universes/{id}/configuration` answers with, of the fields
/// this tool manages.
///
/// The read is v1 while the write is v2, and the two do not carry the same
/// fields, which is the reason several settings here are write-only. v1 has
/// no `permissions` object and no avatar scales; v2 has both but answers to
/// PATCH only, so there is no request that returns them. Anything absent here
/// is a field `pull` cannot adopt, and the lockfile is the only record of what
/// this tool last wrote. Documented on `config::Permissions`.
/// An enum field on the legacy universe configuration, in whichever of its two
/// spellings arrived.
///
/// **The read and the write do not agree, and this is not a guess.** Measured
/// against a live universe on 2026-08-17, `GET /v1/universes/{id}/configuration`
/// answered `"universeAvatarType":"MorphToR15"` for a value the v2 `PATCH` had
/// been given as `3`. Both spellings are in the vendored spec (the integers as
/// the request type, the names inside the response field's own description)
/// and nothing says which one a future response will use.
///
/// Modelling it as `u8` alone was a real regression, not a theoretical one: the
/// whole `UniverseConfigLegacy` failed to deserialize, so `pull` and `init`
/// silently skipped *every* cookie-only universe field, including
/// `studio_access_to_apis_allowed`, which had been read correctly for months
/// before the avatar work touched this struct.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum LegacyEnum {
    Number(u8),
    Name(String),
}

impl LegacyEnum {
    /// Resolve through whichever parser matches the spelling that arrived.
    ///
    /// Takes both parsers rather than one, because only the caller knows which
    /// enum this field is. Returning `None` for an unrecognised value is the
    /// same contract as the two parsers themselves: a value Roblox has not
    /// documented is not coerced into the first variant.
    pub fn resolve<T>(
        &self,
        from_number: impl Fn(u8) -> Option<T>,
        from_name: impl Fn(&str) -> Option<T>,
    ) -> Option<T> {
        match self {
            LegacyEnum::Number(n) => from_number(*n),
            LegacyEnum::Name(name) => from_name(name),
        }
    }
}

/// What `GET /v1/universes/{id}/configuration` answers with, of the fields
/// this tool manages.
///
/// **The read is v1 and the write is v2, and they do not carry the same
/// fields.** That asymmetry is the reason several settings in `rbxmeta.toml`
/// are write-only: v1 has no `permissions` object, no avatar scales, no
/// `universeAvatarAssetOverrides` and no `engineAvatarSettings`; v2 has all
/// four but answers to `PATCH` only, so there is no request that returns them.
///
/// Anything absent from this struct is therefore a field `pull` cannot adopt
/// and `check` cannot compare, and the lockfile is the only record of what this
/// tool last wrote. Spelled out for users in `docs/meta.md` under "Write-only
/// fields".
///
/// The enum fields arrive as [`LegacyEnum`] because the two endpoints disagree
/// about how to spell them: see that type.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UniverseConfigLegacy {
    pub studio_access_to_apis_allowed: Option<bool>,

    /// `['MorphToR6' = 1, 'PlayerChoice' = 2, 'MorphToR15' = 3]`
    pub universe_avatar_type: Option<LegacyEnum>,
    /// `['Standard' = 1, 'PlayerChoice' = 2]`
    pub universe_animation_type: Option<LegacyEnum>,
    /// `['InnerBox' = 1, 'OuterBox' = 2]`
    pub universe_collision_type: Option<LegacyEnum>,
    /// `['Standard' = 1, 'ArtistIntent' = 2]`
    pub universe_joint_positioning_type: Option<LegacyEnum>,

    /// `['All' = 0, 'Tutorial' = 1, ... 'WildWest' = 14]`
    pub genre: Option<LegacyEnum>,

    pub is_for_sale: Option<bool>,
    pub price: Option<u64>,
}

/// Turn a `GET /v1/universes/{id}/configuration` body into the shared model.
///
/// A free function rather than a block inside the request, so the parsing can
/// be tested against a body Roblox really sent. That matters more here than
/// almost anywhere else in this crate: this read was modelled from the
/// specification instead of from a reply, and got the spelling of five fields
/// wrong. A test that deserialized `UniverseConfigLegacy` directly would have
/// passed the whole time, because nothing ever deserializes *that* from the
/// wire: the shim below is the only path.
///
/// The shim exists for one field. The GET spells it
/// `isStudioAccessToApisAllowed` and the PATCH spells it without the prefix;
/// every other field is spelled the same on both sides.
pub fn parse_v1_universe_config(body: &str) -> Result<UniverseConfigLegacy> {
    #[derive(Deserialize, Default)]
    #[serde(rename_all = "camelCase", default)]
    struct V1ConfigResponse {
        is_studio_access_to_apis_allowed: Option<bool>,
        universe_avatar_type: Option<LegacyEnum>,
        universe_animation_type: Option<LegacyEnum>,
        universe_collision_type: Option<LegacyEnum>,
        universe_joint_positioning_type: Option<LegacyEnum>,
        genre: Option<LegacyEnum>,
        is_for_sale: Option<bool>,
        price: Option<u64>,
    }

    let parsed: V1ConfigResponse = serde_json::from_str(body).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse /v1/configuration: {}
Body: {}",
            e,
            body
        )
    })?;

    Ok(UniverseConfigLegacy {
        studio_access_to_apis_allowed: parsed.is_studio_access_to_apis_allowed,
        universe_avatar_type: parsed.universe_avatar_type,
        universe_animation_type: parsed.universe_animation_type,
        universe_collision_type: parsed.universe_collision_type,
        universe_joint_positioning_type: parsed.universe_joint_positioning_type,
        genre: parsed.genre,
        is_for_sale: parsed.is_for_sale,
        price: parsed.price,
    })
}

impl RbxClient {
    pub async fn get_place_legacy(&self) -> Result<PlaceLegacy> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.legacy_url(&format!("{}/{}", PLACE_LEGACY_PATH, self.place_id));

        let response = self
            .client
            .get(&url)
            .header(header::COOKIE, format!(".ROBLOSECURITY={}", cookie))
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(roblox_error(status, &body));
        }

        let parsed: PlaceLegacy = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Failed to parse PlaceLegacy: {}\nBody: {}", e, body))?;
        Ok(parsed)
    }

    /// PATCH the place via the legacy develop.roblox.com endpoint. Handles CSRF.
    pub async fn patch_place_legacy(&self, body: Value) -> Result<()> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.legacy_url(&format!("{}/{}", PLACE_LEGACY_PATH, self.place_id));
        let cookie_header = format!(".ROBLOSECURITY={}", cookie);

        let build = || {
            self.client
                .patch(&url)
                .header(header::COOKIE, &cookie_header)
                .header(header::CONTENT_TYPE, "application/json")
                .json(&body)
        };

        self.send_with_csrf(build).await
    }

    /// Read the universe configuration via the legacy `/v1/configuration` GET
    /// endpoint (cookie-required). This is what Creator Hub itself uses.
    /// Note: the GET response uses `isStudioAccessToApisAllowed` (with `is`
    /// prefix) whereas the PATCH body uses `studioAccessToApisAllowed` (without).
    pub async fn get_universe_config_legacy(&self) -> Result<UniverseConfigLegacy> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.legacy_url(&format!(
            "{}/{}/configuration",
            UNIVERSE_LEGACY_PATH, self.universe_id
        ));

        let response = self
            .client
            .get(&url)
            .header(header::COOKIE, format!(".ROBLOSECURITY={}", cookie))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(roblox_error(status, &body)
                .context("reading the universe configuration from develop.roblox.com"));
        }

        parse_v1_universe_config(&body)
    }

    /// PATCH the universe-level configuration via the legacy endpoint. Handles
    /// CSRF, and returns the response body.
    ///
    /// The body is returned rather than dropped because this endpoint answers
    /// with the configuration it ended up with: `UniverseSettingsResponseV2`,
    /// `engineAvatarSettings` included. That echo is the only place Roblox
    /// ever says anything about the *inside* of that document, which is
    /// otherwise an opaque string everywhere else in its API. `sync` uses it
    /// to notice a key it sent that Roblox silently dropped. See
    /// `crate::engine_echo`.
    pub async fn patch_universe_config_legacy(&self, body: Value) -> Result<String> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.legacy_url(&format!(
            "{}/{}/configuration",
            UNIVERSE_CONFIG_PATH, self.universe_id
        ));
        let cookie_header = format!(".ROBLOSECURITY={}", cookie);

        let build = || {
            self.client
                .patch(&url)
                .header(header::COOKIE, &cookie_header)
                .header(header::CONTENT_TYPE, "application/json")
                .json(&body)
        };

        self.send_with_csrf_body(build).await
    }

    /// Make the universe public. Legacy endpoint, requires cookie.
    pub async fn activate_universe(&self) -> Result<()> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.legacy_url(&format!(
            "{}/{}/activate",
            UNIVERSE_LEGACY_PATH, self.universe_id
        ));
        let cookie_header = format!(".ROBLOSECURITY={}", cookie);

        let build = || {
            self.client
                .post(&url)
                .header(header::COOKIE, &cookie_header)
                .header(header::CONTENT_LENGTH, "0")
        };

        let universe_id = self.universe_id;
        self.send_with_csrf(build).await.with_context(|| {
            format!(
                "Failed to make universe {} public.\n  \
                 Roblox returns the same 403 for both of:\n    \
                 1. Cookie account lacks 'Configure all places' permission in the owning group.\n    \
                 2. Experience isn't eligible for public access yet (e.g. missing maturity label).\n  \
                 Check Creator Hub > <experience> > Basic Info for prerequisites.",
                universe_id
            )
        })
    }

    /// Make the universe private. Legacy endpoint, requires cookie.
    pub async fn deactivate_universe(&self) -> Result<()> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.legacy_url(&format!(
            "{}/{}/deactivate",
            UNIVERSE_LEGACY_PATH, self.universe_id
        ));
        let cookie_header = format!(".ROBLOSECURITY={}", cookie);

        let build = || {
            self.client
                .post(&url)
                .header(header::COOKIE, &cookie_header)
                .header(header::CONTENT_LENGTH, "0")
        };

        let universe_id = self.universe_id;
        self.send_with_csrf(build).await.with_context(|| {
            format!(
                "Failed to make universe {} private.\n  \
                 Likely cause: the cookie account lacks 'Configure all places' permission in the owning group.",
                universe_id
            )
        })
    }

    /// Send a state-changing request, handling Roblox's CSRF token dance.
    ///
    /// The dance lives in `rbx_core::api::send_with_csrf`. What stays here is
    /// the sentence a second refusal turns into: this crate's writes are the
    /// ones a stale token most often bites, so saying the request was retried
    /// is worth the extra branch.
    /// Like [`Self::send_with_csrf`], but hands back the response body.
    ///
    /// Split rather than folded into the existing helper because every other
    /// caller genuinely has nothing to read: `activate`/`deactivate` answer
    /// with nothing useful, and a `Result<String>` at those call sites would be
    /// a value invented so it could be ignored.
    async fn send_with_csrf_body<F>(&self, build: F) -> Result<String>
    where
        F: Fn() -> RequestBuilder,
    {
        let response = self.send_response_with_csrf(build).await?;
        // A body we cannot read is not a failed write: the write already
        // succeeded. The caller treats an empty string as "no echo to check".
        Ok(response.text().await.unwrap_or_default())
    }

    async fn send_with_csrf<F>(&self, build: F) -> Result<()>
    where
        F: Fn() -> RequestBuilder,
    {
        self.send_response_with_csrf(build).await.map(|_| ())
    }

    async fn send_response_with_csrf<F>(&self, build: F) -> Result<reqwest::Response>
    where
        F: Fn() -> RequestBuilder,
    {
        match rbx_core::api::send_with_csrf(&self.csrf_token, build).await {
            Ok(response) => Ok(response),
            Err(rbx_core::api::CsrfError::Transport(e)) => Err(e),
            Err(rbx_core::api::CsrfError::Refused(r)) if r.retried => Err(roblox_error(
                r.status, &r.body,
            )
            .context("the request was retried with a refreshed CSRF token and failed again")),
            Err(rbx_core::api::CsrfError::Refused(r)) => Err(roblox_error(r.status, &r.body)),
        }
    }
}

#[cfg(test)]
mod tests;
