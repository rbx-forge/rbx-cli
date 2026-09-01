//! What changes at the universe level, and how it has to be spelled.
//!
//! Two patches, because Roblox splits the surface: a modern one that takes a
//! field mask, and a legacy one that does not. Both are built here so a setting
//! that moved between them is visible as a move.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::api::models::ApiSocialLink;
use crate::config::{Game, SocialLink, Visibility};
use crate::lockfile::GameLock;
use rbx_core::image::hash_bytes;

use super::*;

pub(crate) fn build_beta_mode_change(game: &Game, lock: &GameLock) -> Option<bool> {
    let desired = game.beta_mode?;
    if lock.beta_mode == Some(desired) {
        return None;
    }
    Some(desired)
}

pub(crate) fn build_universe_legacy_patch(
    game: &Game,
    lock: &GameLock,
    config_dir: &Path,
) -> Result<Option<UniverseLegacyPatch>> {
    let mut body = serde_json::Map::new();
    let mut descriptions: Vec<String> = Vec::new();
    let mut engine_avatar_settings_hash = None;

    if let Some(desired) = game.studio_access_to_apis_allowed {
        if lock.studio_access_to_apis_allowed != Some(desired) {
            body.insert("studioAccessToApisAllowed".to_string(), json!(desired));
            descriptions.push(format!(
                "studio_access_to_apis_allowed: {} → {}",
                show_opt(lock.studio_access_to_apis_allowed),
                desired
            ));
        }
    }

    if let Some(desired) = game.permissions {
        if lock.permissions != Some(desired) {
            // Sent whole, because Roblox reads it whole. `config::Permissions`
            // requires all four fields for the same reason.
            body.insert(
                "permissions".to_string(),
                json!({
                    "IsThirdPartyTeleportAllowed": desired.third_party_teleport,
                    "IsThirdPartyAssetAllowed": desired.third_party_asset,
                    "IsThirdPartyPurchaseAllowed": desired.third_party_purchase,
                    "IsClientTeleportAllowed": desired.client_teleport,
                }),
            );
            descriptions.push(format!(
                "permissions: {} → {}",
                show_permissions(lock.permissions.as_ref()),
                show_permissions(Some(&desired))
            ));
        }
    }

    if let Some(desired) = game.avatar.kind {
        if lock.avatar.kind != Some(desired) {
            body.insert("universeAvatarType".to_string(), json!(desired.to_legacy()));
            descriptions.push(format!(
                "avatar.type: {} → {:?}",
                show_debug_opt(lock.avatar.kind),
                desired
            ));
        }
    }

    if let Some(desired) = game.avatar.animation {
        if lock.avatar.animation != Some(desired) {
            body.insert(
                "universeAnimationType".to_string(),
                json!(desired.to_legacy()),
            );
            descriptions.push(format!(
                "avatar.animation: {} → {:?}",
                show_debug_opt(lock.avatar.animation),
                desired
            ));
        }
    }

    if let Some(desired) = game.avatar.collision {
        if lock.avatar.collision != Some(desired) {
            body.insert(
                "universeCollisionType".to_string(),
                json!(desired.to_legacy()),
            );
            descriptions.push(format!(
                "avatar.collision: {} → {:?}",
                show_debug_opt(lock.avatar.collision),
                desired
            ));
        }
    }

    if let Some(desired) = game.avatar.joint_positioning {
        if lock.avatar.joint_positioning != Some(desired) {
            body.insert(
                "universeJointPositioningType".to_string(),
                json!(desired.to_legacy()),
            );
            descriptions.push(format!(
                "avatar.joint_positioning: {} → {:?}",
                show_debug_opt(lock.avatar.joint_positioning),
                desired
            ));
        }
    }

    if let Some(desired) = game.avatar.min_scale {
        if lock.avatar.min_scale != Some(desired) {
            body.insert("universeAvatarMinScales".to_string(), scales_json(&desired));
            descriptions.push("avatar.min_scale changed".to_string());
        }
    }

    if let Some(desired) = game.avatar.max_scale {
        if lock.avatar.max_scale != Some(desired) {
            body.insert("universeAvatarMaxScales".to_string(), scales_json(&desired));
            descriptions.push("avatar.max_scale changed".to_string());
        }
    }

    if let Some(desired) = &game.paid_access {
        if lock.paid_access.as_ref() != Some(desired) {
            // `isForSale` and `price` travel together: Roblox rejects a price
            // on an experience that is not for sale, and an experience turned
            // on for sale with no price is free by accident.
            body.insert("isForSale".to_string(), json!(desired.is_for_sale()));
            if let Some(price) = desired.price() {
                body.insert("price".to_string(), json!(price));
            }
            descriptions.push(format!(
                "paid_access: {} → {}",
                show_paid_access(lock.paid_access.as_ref()),
                show_paid_access(Some(desired))
            ));
        }
    }

    if let Some(desired) = game.genre {
        if lock.genre != Some(desired) {
            body.insert("genre".to_string(), json!(desired.to_legacy()));
            descriptions.push(format!(
                "genre: {} → {:?}",
                show_debug_opt(lock.genre),
                desired
            ));
        }
    }

    if let Some(desired) = &game.avatar.asset_overrides {
        if lock.avatar.asset_overrides.as_ref() != Some(desired) {
            // Sent whole, like the scales and the permissions: Roblox replaces
            // the array rather than merging into it.
            let slots: Vec<Value> = desired
                .to_legacy()
                .into_iter()
                .map(|(type_id, is_player_choice, asset_id)| {
                    json!({
                        "assetTypeID": type_id,
                        "isPlayerChoice": is_player_choice,
                        "assetID": asset_id,
                    })
                })
                .collect();
            body.insert("universeAvatarAssetOverrides".to_string(), json!(slots));
            descriptions.push("avatar.asset_overrides changed".to_string());
        }
    }

    if let Some(path) = &game.engine_avatar_settings {
        let (document, hash) = read_engine_avatar_settings(&config_dir.join(path))?;
        refuse_overlapping_avatar_writes(&body, &document, path)?;
        if lock.engine_avatar_settings_hash.as_deref() != Some(hash.as_str()) {
            // A JSON *string*, not a JSON object: the field is typed that way,
            // so the document is serialised and handed over as text.
            body.insert("engineAvatarSettings".to_string(), json!(document));
            descriptions.push(format!(
                "engine_avatar_settings: {} → {} ({})",
                short_hash_opt(lock.engine_avatar_settings_hash.as_deref()),
                short_hash(&hash),
                path.display()
            ));
            engine_avatar_settings_hash = Some(hash);
        }
    }

    Ok(if body.is_empty() {
        None
    } else {
        Some(UniverseLegacyPatch {
            body: Value::Object(body),
            descriptions,
            engine_avatar_settings_hash,
        })
    })
}

/// Read the engine avatar settings document, returning `(compact JSON, hash)`.
///
/// Parsed and re-serialised rather than passed through verbatim, for two
/// reasons that both matter. A file that is not JSON is caught here, before a
/// cookie-authenticated write, instead of coming back as an opaque 400. And
/// the hash is then of the canonical form, so reindenting the file or moving a
/// key does not read as a change to be re-sent.
///
/// The keys inside are not inspected. See `config::Game::engine_avatar_settings`
/// for why modelling them would be inventing a contract Roblox has not offered.
///
/// ## Why the extension decides the format
///
/// TOML *and* JSON, because the two ways this document arrives are different.
/// Roblox's field is a JSON string, and anything dumped out of Studio or copied
/// from somebody's example is JSON, so refusing it would mean hand-converting a
/// hundred and fifty keys. But a project whose every other config file is TOML
/// should not be forced to grow one that is not, which is the objection this
/// answers.
///
/// Both land on the same `serde_json::Value` before hashing, so the two formats
/// are interchangeable: rewriting `avatar.json` as `avatar.toml` with the same
/// content produces the same hash and sends nothing.
///
/// TOML has no `null`. Nothing in the documents Roblox accepts here uses one
/// (they are numbers, booleans, arrays and tables all the way down) but a
/// document that needed one would have to be the JSON form.
pub(crate) fn read_engine_avatar_settings(path: &Path) -> Result<(String, String)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading engine_avatar_settings from {}", path.display()))?;

    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let parsed: Value = match extension.as_str() {
        "json" => serde_json::from_str(&text).with_context(|| {
            format!(
                "{} is set as engine_avatar_settings but is not valid JSON",
                path.display()
            )
        })?,
        "toml" => {
            let table: toml::Value = toml::from_str(&text).with_context(|| {
                format!(
                    "{} is set as engine_avatar_settings but is not valid TOML",
                    path.display()
                )
            })?;
            serde_json::to_value(table).with_context(|| {
                format!("converting {} to the JSON Roblox expects", path.display())
            })?
        }
        // Named rather than guessed. Sniffing the content would let a `.txt`
        // through and turn a typo in the path into a silent success.
        other => bail!(
            "engine_avatar_settings must be a .toml or .json file; {} has {}",
            path.display(),
            if other.is_empty() {
                "no extension".to_string()
            } else {
                format!("a .{other} extension")
            }
        ),
    };

    let document = serde_json::to_string(&parsed)?;
    let hash = hash_bytes(document.as_bytes());
    Ok((document, hash))
}

pub(crate) fn short_hash(hash: &str) -> String {
    hash.chars().take(8).collect::<String>() + "..."
}

pub(crate) fn short_hash_opt(hash: Option<&str>) -> String {
    match hash {
        Some(h) => short_hash(h),
        None => "(unset)".to_string(),
    }
}

pub(crate) fn build_visibility_change(game: &Game, lock: &GameLock) -> Option<Visibility> {
    let desired = game.visibility?;
    if lock.visibility == Some(desired) {
        return None;
    }
    Some(desired)
}

pub(crate) fn build_universe_patch(game: &Game, lock: &GameLock) -> Option<UniversePatch> {
    let mut body = serde_json::Map::new();
    let mut mask: Vec<&'static str> = Vec::new();
    let mut descriptions: Vec<String> = Vec::new();

    // voice_chat
    if let Some(desired) = game.voice_chat {
        if lock.voice_chat != Some(desired) {
            body.insert("voiceChatEnabled".to_string(), json!(desired));
            mask.push("voiceChatEnabled");
            descriptions.push(format!(
                "voice chat: {} → {}",
                show_opt(lock.voice_chat),
                desired
            ));
        }
    }

    // private_server.price
    let desired_pps = game.private_server.as_ref().map(|p| p.price);
    let lock_pps = lock.private_server.as_ref().map(|p| p.price);
    if desired_pps != lock_pps {
        match desired_pps {
            Some(price) => {
                body.insert("privateServerPriceRobux".to_string(), json!(price));
                descriptions.push(format!(
                    "private server price: {} → {}",
                    show_opt(lock_pps),
                    price
                ));
            }
            None => {
                body.insert("privateServerPriceRobux".to_string(), Value::Null);
                descriptions.push(format!("private server: {} → disabled", show_opt(lock_pps)));
            }
        }
        mask.push("privateServerPriceRobux");
    }

    // devices
    push_device(
        &game.devices.desktop,
        lock.devices.desktop,
        "desktopEnabled",
        "desktop",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_device(
        &game.devices.mobile,
        lock.devices.mobile,
        "mobileEnabled",
        "mobile",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_device(
        &game.devices.tablet,
        lock.devices.tablet,
        "tabletEnabled",
        "tablet",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_device(
        &game.devices.console,
        lock.devices.console,
        "consoleEnabled",
        "console",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_device(
        &game.devices.vr,
        lock.devices.vr,
        "vrEnabled",
        "vr",
        &mut body,
        &mut mask,
        &mut descriptions,
    );

    // social links
    push_social(
        &game.social_links.facebook,
        &lock.social_links.facebook,
        "facebookSocialLink",
        "facebook",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_social(
        &game.social_links.twitter,
        &lock.social_links.twitter,
        "twitterSocialLink",
        "twitter",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_social(
        &game.social_links.youtube,
        &lock.social_links.youtube,
        "youtubeSocialLink",
        "youtube",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_social(
        &game.social_links.twitch,
        &lock.social_links.twitch,
        "twitchSocialLink",
        "twitch",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_social(
        &game.social_links.discord,
        &lock.social_links.discord,
        "discordSocialLink",
        "discord",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_social(
        &game.social_links.roblox_group,
        &lock.social_links.roblox_group,
        "robloxGroupSocialLink",
        "roblox_group",
        &mut body,
        &mut mask,
        &mut descriptions,
    );
    push_social(
        &game.social_links.guilded,
        &lock.social_links.guilded,
        "guildedSocialLink",
        "guilded",
        &mut body,
        &mut mask,
        &mut descriptions,
    );

    if mask.is_empty() {
        None
    } else {
        Some(UniversePatch {
            body: Value::Object(body),
            mask,
            descriptions,
        })
    }
}

pub(crate) fn push_device(
    desired: &Option<bool>,
    locked: Option<bool>,
    api_field: &'static str,
    label: &str,
    body: &mut serde_json::Map<String, Value>,
    mask: &mut Vec<&'static str>,
    descriptions: &mut Vec<String>,
) {
    if let Some(v) = desired {
        if locked != Some(*v) {
            body.insert(api_field.to_string(), json!(*v));
            mask.push(api_field);
            descriptions.push(format!("device.{}: {} → {}", label, show_opt(locked), v));
        }
    }
}

pub(crate) fn push_social(
    desired: &Option<SocialLink>,
    locked: &Option<SocialLink>,
    api_field: &'static str,
    label: &str,
    body: &mut serde_json::Map<String, Value>,
    mask: &mut Vec<&'static str>,
    descriptions: &mut Vec<String>,
) {
    match (desired, locked) {
        (Some(d), Some(l)) if d == l => {}
        (Some(d), _) => {
            let api_link = ApiSocialLink::from(d);
            body.insert(
                api_field.to_string(),
                serde_json::to_value(&api_link).unwrap(),
            );
            mask.push(api_field);
            descriptions.push(format!("social.{}: set → '{}' ({})", label, d.title, d.url));
        }
        (None, Some(_)) => {
            body.insert(api_field.to_string(), Value::Null);
            mask.push(api_field);
            descriptions.push(format!("social.{}: remove", label));
        }
        (None, None) => {}
    }
}
