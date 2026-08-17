use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::api::models::ApiSocialLink;
use crate::config::{
    AvatarScales, Game, MediaConfig, PaidAccess, Permissions, ServerFill, SocialLink, Visibility,
};
use crate::lockfile::{GameLock, MediaLockfile};
use rbx_core::image::{hash_bytes, process_image};

#[derive(Debug, Default)]
pub struct SyncPlan {
    pub universe_patch: Option<UniversePatch>,
    pub place_patch: Option<PlacePatch>,
    pub place_legacy_patch: Option<PlaceLegacyPatch>,
    pub universe_legacy_patch: Option<UniverseLegacyPatch>,
    pub visibility_change: Option<Visibility>,
    pub beta_mode_change: Option<bool>,
    pub icon: IconPlan,
    pub thumbnails: ThumbnailPlan,
}

impl SyncPlan {
    pub fn is_empty(&self) -> bool {
        self.universe_patch.is_none()
            && self.place_patch.is_none()
            && self.place_legacy_patch.is_none()
            && self.universe_legacy_patch.is_none()
            && self.visibility_change.is_none()
            && self.beta_mode_change.is_none()
            && matches!(self.icon, IconPlan::None)
            && self.thumbnails.is_empty()
    }

    /// Whether applying this plan sends anything the cookie authenticates.
    ///
    /// The four cookie-only pieces of a sync: the two legacy patches, the
    /// visibility flip, and beta mode. Everything else goes to Open Cloud with
    /// the API key.
    ///
    /// A method rather than the expression it replaced in `sync::run`, because
    /// it is now asked twice — once to refuse early, once inside `apply_plan`
    /// where the guarantee has to hold whatever the caller did — and two copies
    /// of a list of four fields is one field away from disagreeing.
    pub fn needs_cookie(&self) -> bool {
        self.place_legacy_patch.is_some()
            || self.universe_legacy_patch.is_some()
            || self.visibility_change.is_some()
            || self.beta_mode_change.is_some()
    }
}

#[derive(Debug)]
pub struct UniversePatch {
    pub body: Value,
    pub mask: Vec<&'static str>,
    pub descriptions: Vec<String>,
}

#[derive(Debug)]
pub struct PlacePatch {
    pub body: Value,
    pub mask: Vec<&'static str>,
    pub descriptions: Vec<String>,
}

/// Patch for legacy `develop.roblox.com/v2/places/{id}` (cookie-only).
/// No updateMask — the legacy API accepts any subset of fields.
#[derive(Debug)]
pub struct PlaceLegacyPatch {
    pub body: Value,
    pub descriptions: Vec<String>,
}

/// Patch for legacy `develop.roblox.com/v2/universes/{id}/configuration` (cookie-only).
#[derive(Debug)]
pub struct UniverseLegacyPatch {
    pub body: Value,
    pub descriptions: Vec<String>,
    /// Hash of the `engineAvatarSettings` document this patch carries, for the
    /// lockfile to record once the call has landed.
    ///
    /// Carried on the patch rather than recomputed by the caller because the
    /// hash has to be of the exact bytes that were sent. Recomputing it after
    /// the fact would re-read a file that may have changed in between, and
    /// write a hash for a document Roblox never saw.
    pub engine_avatar_settings_hash: Option<String>,
}

#[derive(Debug, Default)]
pub enum IconPlan {
    #[default]
    None,
    Upload {
        bytes: Vec<u8>,
        hash: String,
        path: PathBuf,
    },
}

#[derive(Debug, Default)]
pub struct ThumbnailPlan {
    /// Image IDs to delete (from lockfile entries that are no longer in config).
    pub deletes: Vec<u64>,
    /// New uploads, in the order they should appear in config.
    pub uploads: Vec<ThumbUpload>,
    /// For each config thumbnail, an entry indicating whether it's kept (image_id known)
    /// or a new upload (image_id resolved post-upload).
    pub slots: Vec<ThumbSlot>,
    /// True when the existing thumbnails just need reordering (no adds/deletes).
    /// Without this, a pure reorder would make `is_empty()` true and be skipped.
    pub needs_reorder: bool,
}

impl ThumbnailPlan {
    pub fn is_empty(&self) -> bool {
        self.deletes.is_empty() && self.uploads.is_empty() && !self.needs_reorder
    }
}

#[derive(Debug)]
pub struct ThumbUpload {
    pub bytes: Vec<u8>,
    pub hash: String,
    pub path: PathBuf,
    /// Index into ThumbnailPlan::slots that this upload will fill.
    pub slot_index: usize,
}

#[derive(Debug, Clone)]
// `hash` is kept on both variants for readability/debugging; only `image_id`
// is consumed when building the reorder plan.
#[allow(dead_code)]
pub enum ThumbSlot {
    /// Reuse an existing image already on Roblox (matched by hash).
    Keep { hash: String, image_id: u64 },
    /// Will be filled by a new upload (image ID resolved after the upload call).
    NewUpload { hash: String },
}

pub fn build_plan(
    game: &Game,
    media: &MediaConfig,
    game_lock: &GameLock,
    media_lock: &MediaLockfile,
    config_dir: &Path,
) -> Result<SyncPlan> {
    Ok(SyncPlan {
        universe_patch: build_universe_patch(game, game_lock),
        place_patch: build_place_patch(game, game_lock),
        place_legacy_patch: build_place_legacy_patch(game, game_lock),
        universe_legacy_patch: build_universe_legacy_patch(game, game_lock, config_dir)?,
        visibility_change: build_visibility_change(game, game_lock),
        beta_mode_change: build_beta_mode_change(game, game_lock),
        icon: build_icon_plan(media, media_lock, config_dir)?,
        thumbnails: build_thumbnail_plan(media, media_lock, config_dir)?,
    })
}

fn build_beta_mode_change(game: &Game, lock: &GameLock) -> Option<bool> {
    let desired = game.beta_mode?;
    if lock.beta_mode == Some(desired) {
        return None;
    }
    Some(desired)
}

fn build_universe_legacy_patch(
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
/// should not be forced to grow one that is not — which is the objection this
/// answers.
///
/// Both land on the same `serde_json::Value` before hashing, so the two formats
/// are interchangeable: rewriting `avatar.json` as `avatar.toml` with the same
/// content produces the same hash and sends nothing.
///
/// TOML has no `null`. Nothing in the documents Roblox accepts here uses one —
/// they are numbers, booleans, arrays and tables all the way down — but a
/// document that needed one would have to be the JSON form.
fn read_engine_avatar_settings(path: &Path) -> Result<(String, String)> {
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

fn short_hash(hash: &str) -> String {
    hash.chars().take(8).collect::<String>() + "..."
}

fn short_hash_opt(hash: Option<&str>) -> String {
    match hash {
        Some(h) => short_hash(h),
        None => "(unset)".to_string(),
    }
}

fn build_visibility_change(game: &Game, lock: &GameLock) -> Option<Visibility> {
    let desired = game.visibility?;
    if lock.visibility == Some(desired) {
        return None;
    }
    Some(desired)
}

fn build_place_legacy_patch(game: &Game, lock: &GameLock) -> Option<PlaceLegacyPatch> {
    let mut body = serde_json::Map::new();
    let mut descriptions: Vec<String> = Vec::new();

    if let Some(desired) = &game.server_fill {
        if lock.server_fill.as_ref() != Some(desired) {
            body.insert(
                "socialSlotType".to_string(),
                json!(desired.social_slot_type()),
            );
            if let Some(count) = desired.custom_count() {
                body.insert("customSocialSlotsCount".to_string(), json!(count));
            }
            descriptions.push(format!(
                "server_fill: {} → {:?}",
                lock.server_fill
                    .as_ref()
                    .map(format_server_fill)
                    .unwrap_or_else(|| "(unset)".to_string()),
                format_server_fill(desired)
            ));
        }
    }

    if let Some(desired) = game.allow_copying {
        if lock.allow_copying != Some(desired) {
            body.insert("copyingAllowed".to_string(), json!(desired));
            descriptions.push(format!(
                "allow_copying: {} → {}",
                show_opt(lock.allow_copying),
                desired
            ));
        }
    }

    if body.is_empty() {
        None
    } else {
        Some(PlaceLegacyPatch {
            body: Value::Object(body),
            descriptions,
        })
    }
}

fn format_server_fill(sf: &ServerFill) -> String {
    match sf {
        ServerFill::Automatic => "automatic".to_string(),
        ServerFill::Empty => "empty".to_string(),
        ServerFill::Custom { reserved_slots } => format!("custom(reserved={})", reserved_slots),
    }
}

fn build_universe_patch(game: &Game, lock: &GameLock) -> Option<UniversePatch> {
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

fn push_device(
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

fn push_social(
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

fn build_place_patch(game: &Game, lock: &GameLock) -> Option<PlacePatch> {
    let mut body = serde_json::Map::new();
    let mut mask: Vec<&'static str> = Vec::new();
    let mut descriptions: Vec<String> = Vec::new();

    if let Some(name) = &game.name {
        if lock.name.as_ref() != Some(name) {
            body.insert("displayName".to_string(), json!(name));
            mask.push("displayName");
            descriptions.push(format!(
                "name: {} → {}",
                lock.name.as_deref().unwrap_or("(unset)"),
                name
            ));
        }
    }

    if let Some(description) = &game.description {
        if lock.description.as_ref() != Some(description) {
            body.insert("description".to_string(), json!(description));
            mask.push("description");
            descriptions.push(format!(
                "description: {} → {}",
                preview(lock.description.as_deref().unwrap_or("(unset)")),
                preview(description)
            ));
        }
    }

    if let Some(size) = game.server_size {
        if lock.server_size != Some(size) {
            body.insert("serverSize".to_string(), json!(size));
            mask.push("serverSize");
            descriptions.push(format!(
                "server size: {} → {}",
                show_opt(lock.server_size),
                size
            ));
        }
    }

    if mask.is_empty() {
        None
    } else {
        Some(PlacePatch {
            body: Value::Object(body),
            mask,
            descriptions,
        })
    }
}

fn build_icon_plan(
    media: &MediaConfig,
    media_lock: &MediaLockfile,
    config_dir: &Path,
) -> Result<IconPlan> {
    let Some(icon) = &media.icon else {
        return Ok(IconPlan::None);
    };

    let path = config_dir.join(icon);
    let bytes = process_image(&path, media.bleed)?;
    let hash = hash_bytes(&bytes);

    let lock_hash = media_lock.icon.as_ref().map(|i| i.hash.as_str());
    if lock_hash == Some(hash.as_str()) {
        return Ok(IconPlan::None);
    }

    Ok(IconPlan::Upload {
        bytes,
        hash,
        path: icon.clone(),
    })
}

fn build_thumbnail_plan(
    media: &MediaConfig,
    media_lock: &MediaLockfile,
    config_dir: &Path,
) -> Result<ThumbnailPlan> {
    let mut plan = ThumbnailPlan::default();

    // Compute hash for each local thumbnail.
    let mut local: Vec<(PathBuf, Vec<u8>, String)> = Vec::new();
    for thumb in &media.thumbnails {
        let path = config_dir.join(thumb);
        let bytes = process_image(&path, media.bleed)?;
        let hash = hash_bytes(&bytes);
        local.push((thumb.clone(), bytes, hash));
    }

    // Match each local hash against a lockfile entry; consume each lockfile entry at most once.
    let mut used = vec![false; media_lock.thumbnails.len()];

    for (idx, (path, bytes, hash)) in local.iter().enumerate() {
        let mut matched = None;
        for (i, lock_entry) in media_lock.thumbnails.iter().enumerate() {
            if used[i] {
                continue;
            }
            if lock_entry.hash == *hash {
                matched = Some((i, lock_entry.image_id));
                break;
            }
        }

        match matched {
            Some((lock_idx, Some(image_id))) => {
                used[lock_idx] = true;
                plan.slots.push(ThumbSlot::Keep {
                    hash: hash.clone(),
                    image_id,
                });
            }
            Some((lock_idx, None)) => {
                // Lockfile entry without an image_id — treat as new upload.
                used[lock_idx] = true;
                plan.uploads.push(ThumbUpload {
                    bytes: bytes.clone(),
                    hash: hash.clone(),
                    path: path.clone(),
                    slot_index: idx,
                });
                plan.slots.push(ThumbSlot::NewUpload { hash: hash.clone() });
            }
            None => {
                plan.uploads.push(ThumbUpload {
                    bytes: bytes.clone(),
                    hash: hash.clone(),
                    path: path.clone(),
                    slot_index: idx,
                });
                plan.slots.push(ThumbSlot::NewUpload { hash: hash.clone() });
            }
        }
    }

    // Any lockfile entries not consumed → delete.
    for (i, lock_entry) in media_lock.thumbnails.iter().enumerate() {
        if !used[i] {
            if let Some(image_id) = lock_entry.image_id {
                plan.deletes.push(image_id);
            }
        }
    }

    // Detect a pure reorder: the same images stay, only their order changed.
    // With no uploads or deletes, `is_empty()` would otherwise hide this and
    // the reorder would never be sent to Roblox.
    if plan.deletes.is_empty() && plan.uploads.is_empty() {
        let want: Vec<u64> = plan
            .slots
            .iter()
            .filter_map(|s| match s {
                ThumbSlot::Keep { image_id, .. } => Some(*image_id),
                ThumbSlot::NewUpload { .. } => None,
            })
            .collect();
        let have: Vec<u64> = media_lock
            .thumbnails
            .iter()
            .filter_map(|m| m.image_id)
            .collect();
        plan.needs_reorder = want != have;
    }

    Ok(plan)
}

/// Helpers
fn show_opt<T: std::fmt::Display>(v: Option<T>) -> String {
    match v {
        Some(x) => x.to_string(),
        None => "(unset)".to_string(),
    }
}

/// `{:?}` for a value that has one, `(unset)` for none. The enums here are
/// small and their `Debug` spelling is the config spelling with different
/// casing, which is close enough to read in a plan.
fn show_debug_opt<T: std::fmt::Debug>(value: Option<T>) -> String {
    match value {
        Some(v) => format!("{v:?}"),
        None => "(unset)".to_string(),
    }
}

fn show_permissions(p: Option<&Permissions>) -> String {
    match p {
        Some(p) => format!(
            "teleport={} asset={} purchase={} client={}",
            p.third_party_teleport, p.third_party_asset, p.third_party_purchase, p.client_teleport
        ),
        None => "(unset)".to_string(),
    }
}

fn show_paid_access(p: Option<&PaidAccess>) -> String {
    match p {
        Some(PaidAccess::Free) => "free".to_string(),
        Some(PaidAccess::Paid { price }) => format!("{price} Robux"),
        None => "(unset)".to_string(),
    }
}

/// The scale object Roblox takes.
///
/// **Five of the six fields.** `Roblox.Web.Responses.Avatar.ScaleModel` also
/// declares `depth`, and this omits it — which sits awkwardly beside the reason
/// [`AvatarScales`] requires all of its own fields: that Roblox reads the
/// object whole, so a key left out is a key it may read as zero.
///
/// It is omitted on precedent rather than on principle. Mantle's
/// `ExperienceAvatarScales` carries the same five and not `depth`, and Mantle
/// wrote avatar scales against real experiences for years. That is evidence,
/// not proof, and it is the strongest available: nothing here has sent this
/// object to Roblox, and `depth` appears in no avatar scaling UI to compare
/// against.
///
/// **If a synced experience comes back with squashed avatars, this is the first
/// place to look** — add `depth` to [`AvatarScales`] as a sixth required field
/// and it will travel with the rest.
fn scales_json(s: &AvatarScales) -> Value {
    json!({
        "height": s.height,
        "width": s.width,
        "head": s.head,
        "bodyType": s.body_type,
        "proportion": s.proportion,
    })
}

fn preview(s: &str) -> String {
    const MAX: usize = 60;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let trimmed: String = s.chars().take(MAX).collect();
        format!("{}…", trimmed)
    }
}

/// Build a GameLock mirroring a (resolved) Game struct.
pub fn config_to_lock(game: &Game) -> GameLock {
    GameLock {
        name: game.name.clone(),
        description: game.description.clone(),
        server_size: game.server_size,
        voice_chat: game.voice_chat,
        allow_copying: game.allow_copying,
        visibility: game.visibility,
        studio_access_to_apis_allowed: game.studio_access_to_apis_allowed,
        beta_mode: game.beta_mode,
        private_server: game.private_server.clone(),
        devices: game.devices.clone(),
        social_links: game.social_links.clone(),
        server_fill: game.server_fill.clone(),
        permissions: game.permissions,
        avatar: game.avatar,
        paid_access: game.paid_access.clone(),
        genre: game.genre,
        // Deliberately not derived here. This function takes only a `Game`, and
        // the hash is of a file it has no path to resolve — `sync` writes it
        // from the patch that carried it, which is also the only moment the
        // hash is known to describe something Roblox actually received.
        engine_avatar_settings_hash: None,
    }
}

/// The desired final order of image IDs based on local config order.
pub fn desired_order(plan: &ThumbnailPlan, new_image_ids: &[Option<u64>]) -> Vec<u64> {
    let mut order: Vec<u64> = Vec::new();
    let upload_id_for_slot = |slot_idx: usize| -> Option<u64> {
        for (upload_idx, upload) in plan.uploads.iter().enumerate() {
            if upload.slot_index == slot_idx {
                return new_image_ids.get(upload_idx).copied().flatten();
            }
        }
        None
    };

    for (idx, slot) in plan.slots.iter().enumerate() {
        match slot {
            ThumbSlot::Keep { image_id, .. } => order.push(*image_id),
            ThumbSlot::NewUpload { .. } => {
                if let Some(id) = upload_id_for_slot(idx) {
                    order.push(id);
                }
            }
        }
    }

    order
}

// ---------------------------------------------------------------------------
// Tests
//
// These functions decide what gets written to a live universe and a live
// place. They are pure — config + lockfile in, patch out — so they are cheap
// to pin exactly, and every assertion below is on the exact body and mask
// rather than on the patch merely existing.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Devices, MediaConfig, PrivateServer};
    use crate::lockfile::MediaLock;
    use serde_json::json;
    use std::path::Path;

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    fn game() -> Game {
        Game::default()
    }

    fn lock() -> GameLock {
        GameLock::default()
    }

    fn link(title: &str, url: &str) -> SocialLink {
        SocialLink {
            title: title.to_string(),
            url: url.to_string(),
        }
    }

    /// `build_plan` with no media, so nothing touches the filesystem.
    fn plan_for(game: &Game, lock: &GameLock) -> SyncPlan {
        build_plan(
            game,
            &MediaConfig::default(),
            lock,
            &MediaLockfile::default(),
            Path::new("."),
        )
        .expect("a plan with no media never reads a file")
    }

    // -----------------------------------------------------------------------
    // build_plan: the entry point
    // -----------------------------------------------------------------------

    mod build_plan {
        use super::*;

        #[test]
        fn a_config_that_matches_the_lock_produces_an_empty_plan() {
            let mut g = game();
            g.name = Some("Game".into());
            g.server_size = Some(50);
            g.voice_chat = Some(true);
            g.visibility = Some(Visibility::Public);
            g.beta_mode = Some(false);
            g.allow_copying = Some(true);
            g.studio_access_to_apis_allowed = Some(true);
            g.private_server = Some(PrivateServer { price: 100 });
            g.devices.desktop = Some(true);
            g.social_links.discord = Some(link("Discord", "https://d.test"));
            g.server_fill = Some(ServerFill::Empty);

            let plan = plan_for(&g, &config_to_lock(&g));

            assert!(
                plan.is_empty(),
                "a lock built from the config itself leaves nothing to do: {plan:?}"
            );
        }

        /// An empty config is not a request to clear everything. Every field is
        /// `Option`, and `None` means "not declared here", not "set to nothing".
        #[test]
        fn an_empty_config_against_a_populated_lock_asks_for_nothing() {
            let mut locked = lock();
            locked.name = Some("Remote name".into());
            locked.server_size = Some(30);
            locked.voice_chat = Some(true);
            locked.devices.desktop = Some(true);
            locked.visibility = Some(Visibility::Public);
            locked.beta_mode = Some(true);
            locked.allow_copying = Some(true);
            locked.studio_access_to_apis_allowed = Some(true);
            locked.server_fill = Some(ServerFill::Automatic);

            let plan = plan_for(&game(), &locked);

            assert!(plan.universe_patch.is_none());
            assert!(plan.place_patch.is_none());
            assert!(plan.place_legacy_patch.is_none());
            assert!(plan.universe_legacy_patch.is_none());
            assert!(plan.visibility_change.is_none());
            assert!(plan.beta_mode_change.is_none());
            assert!(plan.is_empty());
        }

        /// The one exception to the rule above: a private server that the lock
        /// has and the config does not is an explicit "disable", because
        /// removing the `[game.private_server]` table is how you disable it.
        #[test]
        fn a_dropped_private_server_table_is_an_explicit_disable() {
            let mut locked = lock();
            locked.private_server = Some(PrivateServer { price: 100 });

            let plan = plan_for(&game(), &locked);
            let patch = plan.universe_patch.expect("disabling is a change");

            assert_eq!(patch.body, json!({ "privateServerPriceRobux": null }));
            assert_eq!(patch.mask, vec!["privateServerPriceRobux"]);
        }

        /// Each field lands in the patch its own API demands. Getting this
        /// wrong sends a cookie-only field to Open Cloud, which ignores it.
        #[test]
        fn each_field_is_routed_to_the_api_that_owns_it() {
            let mut g = game();
            g.name = Some("Name".into()); // place
            g.voice_chat = Some(true); // universe
            g.allow_copying = Some(true); // place, legacy
            g.studio_access_to_apis_allowed = Some(true); // universe config, legacy
            g.visibility = Some(Visibility::Public); // activate/deactivate
            g.beta_mode = Some(true); // experience releases

            let plan = plan_for(&g, &lock());

            assert_eq!(
                plan.universe_patch.expect("universe").body,
                json!({ "voiceChatEnabled": true })
            );
            assert_eq!(
                plan.place_patch.expect("place").body,
                json!({ "displayName": "Name" })
            );
            assert_eq!(
                plan.place_legacy_patch.expect("place legacy").body,
                json!({ "copyingAllowed": true })
            );
            assert_eq!(
                plan.universe_legacy_patch.expect("universe legacy").body,
                json!({ "studioAccessToApisAllowed": true })
            );
            assert_eq!(plan.visibility_change, Some(Visibility::Public));
            assert_eq!(plan.beta_mode_change, Some(true));
        }
    }

    // -----------------------------------------------------------------------
    // build_universe_patch
    // -----------------------------------------------------------------------

    mod universe_patch {
        use super::*;

        #[test]
        fn no_change_means_no_patch() {
            let mut g = game();
            g.voice_chat = Some(true);
            let mut l = lock();
            l.voice_chat = Some(true);

            assert!(plan_for(&g, &l).universe_patch.is_none());
        }

        #[test]
        fn every_device_toggle_has_its_own_api_field_and_mask_entry() {
            let mut g = game();
            g.devices = Devices {
                desktop: Some(true),
                mobile: Some(false),
                tablet: Some(true),
                console: Some(false),
                vr: Some(true),
            };

            let patch = plan_for(&g, &lock()).universe_patch.expect("patch");

            assert_eq!(
                patch.body,
                json!({
                    "desktopEnabled": true,
                    "mobileEnabled": false,
                    "tabletEnabled": true,
                    "consoleEnabled": false,
                    "vrEnabled": true,
                })
            );
            assert_eq!(
                patch.mask,
                vec![
                    "desktopEnabled",
                    "mobileEnabled",
                    "tabletEnabled",
                    "consoleEnabled",
                    "vrEnabled"
                ]
            );
        }

        /// Only the devices that actually differ are sent. A mask naming a
        /// field the body does not carry is how you clear a field by accident.
        #[test]
        fn only_the_devices_that_differ_are_sent() {
            let mut g = game();
            g.devices.desktop = Some(true);
            g.devices.mobile = Some(true);
            let mut l = lock();
            l.devices.desktop = Some(true);

            let patch = plan_for(&g, &l).universe_patch.expect("patch");

            assert_eq!(patch.body, json!({ "mobileEnabled": true }));
            assert_eq!(patch.mask, vec!["mobileEnabled"]);
        }

        #[test]
        fn a_social_link_is_sent_as_the_api_shape() {
            let mut g = game();
            g.social_links.discord = Some(link("Join us", "https://discord.gg/x"));

            let patch = plan_for(&g, &lock()).universe_patch.expect("patch");

            assert_eq!(
                patch.body,
                json!({
                    "discordSocialLink": { "title": "Join us", "uri": "https://discord.gg/x" }
                })
            );
            assert_eq!(patch.mask, vec!["discordSocialLink"]);
        }

        /// A link in the lock and not in the config is a removal, sent as an
        /// explicit null. This is the one place where "absent" does mean
        /// "clear it", because a social link has no other way to be removed.
        #[test]
        fn a_link_dropped_from_the_config_is_removed_with_an_explicit_null() {
            let mut l = lock();
            l.social_links.twitter = Some(link("Old", "https://x.test"));

            let patch = plan_for(&game(), &l).universe_patch.expect("patch");

            assert_eq!(patch.body, json!({ "twitterSocialLink": null }));
            assert_eq!(patch.mask, vec!["twitterSocialLink"]);
        }

        #[test]
        fn an_unchanged_link_is_left_alone() {
            let mut g = game();
            g.social_links.youtube = Some(link("Yt", "https://yt.test"));
            let mut l = lock();
            l.social_links.youtube = Some(link("Yt", "https://yt.test"));

            assert!(plan_for(&g, &l).universe_patch.is_none());
        }

        #[test]
        fn retitling_a_link_at_the_same_url_still_patches() {
            let mut g = game();
            g.social_links.youtube = Some(link("New title", "https://yt.test"));
            let mut l = lock();
            l.social_links.youtube = Some(link("Old title", "https://yt.test"));

            let patch = plan_for(&g, &l).universe_patch.expect("patch");

            assert_eq!(
                patch.body,
                json!({
                    "youtubeSocialLink": { "title": "New title", "uri": "https://yt.test" }
                })
            );
        }

        #[test]
        fn a_private_server_price_change_carries_the_new_price() {
            let mut g = game();
            g.private_server = Some(PrivateServer { price: 250 });
            let mut l = lock();
            l.private_server = Some(PrivateServer { price: 100 });

            let patch = plan_for(&g, &l).universe_patch.expect("patch");

            assert_eq!(patch.body, json!({ "privateServerPriceRobux": 250 }));
            assert_eq!(patch.mask, vec!["privateServerPriceRobux"]);
        }

        /// Free (price 0) and disabled (no table) are different states, and
        /// `Some(0) != None` has to survive the round trip through the patch.
        #[test]
        fn free_private_servers_are_not_the_same_as_disabled_ones() {
            let mut g = game();
            g.private_server = Some(PrivateServer { price: 0 });

            let patch = plan_for(&g, &lock()).universe_patch.expect("patch");

            assert_eq!(patch.body, json!({ "privateServerPriceRobux": 0 }));
        }
    }

    // -----------------------------------------------------------------------
    // build_place_patch
    // -----------------------------------------------------------------------

    mod place_patch {
        use super::*;

        #[test]
        fn name_description_and_server_size_map_to_their_api_fields() {
            let mut g = game();
            g.name = Some("My Game".into());
            g.description = Some("Fun".into());
            g.server_size = Some(50);

            let patch = plan_for(&g, &lock()).place_patch.expect("patch");

            assert_eq!(
                patch.body,
                json!({ "displayName": "My Game", "description": "Fun", "serverSize": 50 })
            );
            assert_eq!(patch.mask, vec!["displayName", "description", "serverSize"]);
        }

        #[test]
        fn only_the_fields_that_differ_are_sent() {
            let mut g = game();
            g.name = Some("Same".into());
            g.server_size = Some(60);
            let mut l = lock();
            l.name = Some("Same".into());
            l.server_size = Some(50);

            let patch = plan_for(&g, &l).place_patch.expect("patch");

            assert_eq!(patch.body, json!({ "serverSize": 60 }));
            assert_eq!(patch.mask, vec!["serverSize"]);
        }

        #[test]
        fn an_empty_string_is_a_value_not_an_absence() {
            let mut g = game();
            g.description = Some(String::new());
            let mut l = lock();
            l.description = Some("something".into());

            let patch = plan_for(&g, &l).place_patch.expect("patch");

            assert_eq!(patch.body, json!({ "description": "" }));
        }
    }

    // -----------------------------------------------------------------------
    // Legacy (cookie-only) patches
    // -----------------------------------------------------------------------

    mod legacy_patches {
        use super::*;

        #[test]
        fn a_custom_server_fill_carries_its_reserved_slot_count() {
            let mut g = game();
            g.server_fill = Some(ServerFill::Custom { reserved_slots: 5 });

            let patch = plan_for(&g, &lock()).place_legacy_patch.expect("patch");

            assert_eq!(
                patch.body,
                json!({ "socialSlotType": "Custom", "customSocialSlotsCount": 5 })
            );
        }

        /// Non-custom modes must not carry a slot count: the legacy API takes
        /// the field literally, and a stale count on `Automatic` sticks.
        #[test]
        fn a_non_custom_server_fill_sends_no_slot_count() {
            let mut g = game();
            g.server_fill = Some(ServerFill::Automatic);
            let mut l = lock();
            l.server_fill = Some(ServerFill::Custom { reserved_slots: 5 });

            let patch = plan_for(&g, &l).place_legacy_patch.expect("patch");

            assert_eq!(patch.body, json!({ "socialSlotType": "Automatic" }));
        }

        #[test]
        fn changing_only_the_reserved_slot_count_is_still_a_change() {
            let mut g = game();
            g.server_fill = Some(ServerFill::Custom { reserved_slots: 8 });
            let mut l = lock();
            l.server_fill = Some(ServerFill::Custom { reserved_slots: 5 });

            let patch = plan_for(&g, &l).place_legacy_patch.expect("patch");

            assert_eq!(
                patch.body,
                json!({ "socialSlotType": "Custom", "customSocialSlotsCount": 8 })
            );
        }

        #[test]
        fn allow_copying_and_server_fill_share_one_legacy_patch() {
            let mut g = game();
            g.allow_copying = Some(false);
            g.server_fill = Some(ServerFill::Empty);

            let patch = plan_for(&g, &lock()).place_legacy_patch.expect("patch");

            assert_eq!(
                patch.body,
                json!({ "socialSlotType": "Empty", "copyingAllowed": false })
            );
        }

        #[test]
        fn studio_api_access_goes_to_the_universe_config_endpoint() {
            let mut g = game();
            g.studio_access_to_apis_allowed = Some(false);
            let mut l = lock();
            l.studio_access_to_apis_allowed = Some(true);

            let patch = plan_for(&g, &l).universe_legacy_patch.expect("patch");

            assert_eq!(patch.body, json!({ "studioAccessToApisAllowed": false }));
        }
    }

    // -----------------------------------------------------------------------
    // visibility and beta_mode
    //
    // Neither is a patch body: both are separate endpoints, and visibility in
    // particular decides the *order* the rest of the plan is applied in.
    // `commands::sync` activates before every other call when going public
    // (a paid private server cannot be set on a private universe) and
    // deactivates after them all when going private.
    // -----------------------------------------------------------------------

    mod visibility_and_beta {
        use super::*;

        #[test]
        fn going_from_private_to_public_is_a_change() {
            let mut g = game();
            g.visibility = Some(Visibility::Public);
            let mut l = lock();
            l.visibility = Some(Visibility::Private);

            let plan = plan_for(&g, &l);

            assert_eq!(plan.visibility_change, Some(Visibility::Public));
            assert!(
                plan.visibility_change.is_some_and(|v| v.is_public()),
                "sync keys the activate-first branch off is_public()"
            );
        }

        #[test]
        fn going_from_public_to_private_is_a_change() {
            let mut g = game();
            g.visibility = Some(Visibility::Private);
            let mut l = lock();
            l.visibility = Some(Visibility::Public);

            let plan = plan_for(&g, &l);

            assert_eq!(plan.visibility_change, Some(Visibility::Private));
            assert!(
                plan.visibility_change.is_some_and(|v| !v.is_public()),
                "sync keys the deactivate-last branch off !is_public()"
            );
        }

        /// A universe that has never been synced has no locked visibility.
        /// Declaring `public` must still activate it, or every other patch in
        /// the same run is sent against a private universe.
        #[test]
        fn an_unlocked_visibility_still_produces_a_change() {
            let mut g = game();
            g.visibility = Some(Visibility::Public);

            assert_eq!(
                plan_for(&g, &lock()).visibility_change,
                Some(Visibility::Public)
            );
        }

        #[test]
        fn an_already_matching_visibility_is_not_a_change() {
            let mut g = game();
            g.visibility = Some(Visibility::Public);
            let mut l = lock();
            l.visibility = Some(Visibility::Public);

            assert_eq!(plan_for(&g, &l).visibility_change, None);
        }

        /// Going public while enabling a paid private server is the
        /// combination the ordering exists for: the price patch is rejected by
        /// Roblox unless the universe is already public. Both must be in the
        /// same plan for sync to be able to order them.
        #[test]
        fn going_public_and_pricing_a_private_server_land_in_the_same_plan() {
            let mut g = game();
            g.visibility = Some(Visibility::Public);
            g.private_server = Some(PrivateServer { price: 100 });
            let mut l = lock();
            l.visibility = Some(Visibility::Private);

            let plan = plan_for(&g, &l);

            assert_eq!(plan.visibility_change, Some(Visibility::Public));
            assert_eq!(
                plan.universe_patch.expect("price patch").body,
                json!({ "privateServerPriceRobux": 100 })
            );
        }

        #[test]
        fn beta_mode_toggles_only_when_it_differs() {
            let mut g = game();
            g.beta_mode = Some(true);
            assert_eq!(plan_for(&g, &lock()).beta_mode_change, Some(true));

            let mut l = lock();
            l.beta_mode = Some(true);
            assert_eq!(plan_for(&g, &l).beta_mode_change, None);

            l.beta_mode = Some(false);
            assert_eq!(plan_for(&g, &l).beta_mode_change, Some(true));
        }
    }

    // -----------------------------------------------------------------------
    // Media: icon and thumbnails
    // -----------------------------------------------------------------------

    /// 1x1 PNGs, one per colour, so each has a distinct hash. Written to a
    /// temp dir per test rather than committed as fixture files.
    const RED: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415478da63f8cfc0f01f00050001ff56c72f0d0000000049454e44ae426082";
    const GREEN: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415478da6360f8cff01f00040101ffaeb555f50000000049454e44ae426082";
    const BLUE: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415478da636060f8ff1f00030201ff392919be0000000049454e44ae426082";
    const WHITE: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000b4944415478da63f80f040009fb03fd68fa1ccc0000000049454e44ae426082";

    fn from_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
            .collect()
    }

    /// Write `name` into `dir` with the given PNG, and return the hash
    /// `build_*_plan` will compute for it. Computed rather than hardcoded: the
    /// hash is of the *re-encoded* bytes, so it is the image pipeline's to own.
    fn put_image(dir: &Path, name: &str, hex: &str) -> String {
        std::fs::write(dir.join(name), from_hex(hex)).expect("write png");
        hash_bytes(&process_image(&dir.join(name), true).expect("process png"))
    }

    fn media_with(thumbnails: &[&str]) -> MediaConfig {
        MediaConfig {
            thumbnails: thumbnails.iter().map(PathBuf::from).collect(),
            ..MediaConfig::default()
        }
    }

    fn locked(entries: &[(&str, Option<u64>)]) -> MediaLockfile {
        MediaLockfile {
            icon: None,
            thumbnails: entries
                .iter()
                .map(|(hash, image_id)| MediaLock {
                    hash: (*hash).to_string(),
                    image_id: *image_id,
                })
                .collect(),
        }
    }

    fn thumbnail_plan(
        dir: &Path,
        media: &MediaConfig,
        media_lock: &MediaLockfile,
    ) -> ThumbnailPlan {
        build_plan(&game(), media, &lock(), media_lock, dir)
            .expect("plan")
            .thumbnails
    }

    fn slot_ids(plan: &ThumbnailPlan) -> Vec<Option<u64>> {
        plan.slots
            .iter()
            .map(|s| match s {
                ThumbSlot::Keep { image_id, .. } => Some(*image_id),
                ThumbSlot::NewUpload { .. } => None,
            })
            .collect()
    }

    mod icon {
        use super::*;

        #[test]
        fn an_unchanged_icon_is_not_re_uploaded() {
            let dir = tempfile::tempdir().expect("tempdir");
            let hash = put_image(dir.path(), "icon.png", RED);

            let media = MediaConfig {
                icon: Some(PathBuf::from("icon.png")),
                ..MediaConfig::default()
            };
            let media_lock = MediaLockfile {
                icon: Some(MediaLock {
                    hash,
                    image_id: Some(1),
                }),
                thumbnails: Vec::new(),
            };

            let plan = build_plan(&game(), &media, &lock(), &media_lock, dir.path()).expect("plan");

            assert!(matches!(plan.icon, IconPlan::None));
        }

        #[test]
        fn a_changed_icon_is_uploaded_with_its_config_relative_path() {
            let dir = tempfile::tempdir().expect("tempdir");
            put_image(dir.path(), "icon.png", RED);
            let stale = put_image(dir.path(), "other.png", GREEN);

            let media = MediaConfig {
                icon: Some(PathBuf::from("icon.png")),
                ..MediaConfig::default()
            };
            let media_lock = MediaLockfile {
                icon: Some(MediaLock {
                    hash: stale,
                    image_id: Some(1),
                }),
                thumbnails: Vec::new(),
            };

            let plan = build_plan(&game(), &media, &lock(), &media_lock, dir.path()).expect("plan");

            match plan.icon {
                IconPlan::Upload { path, .. } => assert_eq!(path, PathBuf::from("icon.png")),
                IconPlan::None => panic!("a changed icon must be uploaded"),
            }
        }

        #[test]
        fn no_icon_in_the_config_is_not_a_deletion() {
            let dir = tempfile::tempdir().expect("tempdir");
            let media_lock = MediaLockfile {
                icon: Some(MediaLock {
                    hash: "whatever".into(),
                    image_id: Some(1),
                }),
                thumbnails: Vec::new(),
            };

            let plan = build_plan(
                &game(),
                &MediaConfig::default(),
                &lock(),
                &media_lock,
                dir.path(),
            )
            .expect("plan");

            assert!(matches!(plan.icon, IconPlan::None));
        }
    }

    // -----------------------------------------------------------------------
    // build_thumbnail_plan
    //
    // The delete / upload / reorder reconciliation. Thumbnails are matched to
    // lockfile entries by content hash, not by position, so the interesting
    // cases are all about which lock entry a local file claims and what is
    // left over.
    // -----------------------------------------------------------------------

    mod thumbnails {
        use super::*;

        #[test]
        fn a_first_sync_uploads_everything_in_config_order() {
            let dir = tempfile::tempdir().expect("tempdir");
            put_image(dir.path(), "a.png", RED);
            put_image(dir.path(), "b.png", GREEN);

            let plan = thumbnail_plan(dir.path(), &media_with(&["a.png", "b.png"]), &locked(&[]));

            assert!(plan.deletes.is_empty());
            assert_eq!(
                plan.uploads
                    .iter()
                    .map(|u| (u.path.clone(), u.slot_index))
                    .collect::<Vec<_>>(),
                vec![(PathBuf::from("a.png"), 0), (PathBuf::from("b.png"), 1)]
            );
            assert_eq!(slot_ids(&plan), vec![None, None]);
            assert!(!plan.needs_reorder, "there is nothing to reorder yet");
        }

        #[test]
        fn thumbnails_already_uploaded_in_the_same_order_are_left_alone() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);
            let b = put_image(dir.path(), "b.png", GREEN);

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&["a.png", "b.png"]),
                &locked(&[(&a, Some(10)), (&b, Some(20))]),
            );

            assert!(plan.is_empty(), "nothing to do: {plan:?}");
            assert_eq!(slot_ids(&plan), vec![Some(10), Some(20)]);
        }

        #[test]
        fn a_thumbnail_dropped_from_the_config_is_deleted() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);
            let b = put_image(dir.path(), "b.png", GREEN);

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&["a.png"]),
                &locked(&[(&a, Some(10)), (&b, Some(20))]),
            );

            assert_eq!(plan.deletes, vec![20]);
            assert!(plan.uploads.is_empty());
            assert_eq!(slot_ids(&plan), vec![Some(10)]);
        }

        #[test]
        fn clearing_the_config_deletes_every_tracked_thumbnail() {
            let dir = tempfile::tempdir().expect("tempdir");

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&[]),
                &locked(&[("h1", Some(10)), ("h2", Some(20))]),
            );

            assert_eq!(plan.deletes, vec![10, 20]);
            assert!(plan.slots.is_empty());
        }

        /// A lock entry with no `image_id` is one whose upload response never
        /// came back. There is nothing to delete on Roblox and nothing to
        /// reuse, so the local file has to be uploaded again.
        #[test]
        fn a_lock_entry_without_an_image_id_is_re_uploaded_not_deleted() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);

            let plan = thumbnail_plan(dir.path(), &media_with(&["a.png"]), &locked(&[(&a, None)]));

            assert!(
                plan.deletes.is_empty(),
                "there is no remote image to delete"
            );
            assert_eq!(plan.uploads.len(), 1);
            assert_eq!(slot_ids(&plan), vec![None]);
        }

        /// An orphaned entry with no `image_id` is no remote work: there is
        /// nothing to delete and nothing to reorder, so the plan is empty and
        /// stays empty. That is correct here and is deliberately not where the
        /// entry gets cleaned up.
        ///
        /// It used to be nowhere. `sync` only dropped lockfile entries inside
        /// its deletes loop, which is keyed on `image_id`, so an entry without
        /// one survived every run forever (#22). `sync` now prunes those as
        /// bookkeeping, before its own "nothing to do" exit — which it has to
        /// be, because this plan being empty is exactly what that exit tests.
        #[test]
        fn an_orphaned_lock_entry_without_an_image_id_is_no_remote_work() {
            let dir = tempfile::tempdir().expect("tempdir");

            let plan = thumbnail_plan(dir.path(), &media_with(&[]), &locked(&[("gone", None)]));

            assert!(plan.deletes.is_empty());
            assert!(plan.is_empty());
        }

        /// Reordering the same files is invisible to a delete/upload diff:
        /// nothing is added and nothing is removed. Without `needs_reorder`
        /// the plan would look empty and the new order would never be sent.
        #[test]
        fn a_pure_reorder_is_detected_even_though_nothing_is_added_or_removed() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);
            let b = put_image(dir.path(), "b.png", GREEN);

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&["b.png", "a.png"]),
                &locked(&[(&a, Some(10)), (&b, Some(20))]),
            );

            assert!(plan.deletes.is_empty());
            assert!(plan.uploads.is_empty());
            assert!(plan.needs_reorder, "the order changed and must be sent");
            assert!(!plan.is_empty(), "a pure reorder is not an empty plan");
            assert_eq!(slot_ids(&plan), vec![Some(20), Some(10)]);
        }

        /// `needs_reorder` is only computed when there is nothing else to do,
        /// because uploads and deletes change the remote order anyway and the
        /// reorder is derived from the post-op state instead.
        #[test]
        fn a_reorder_alongside_an_upload_is_left_to_the_post_upload_order() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);
            put_image(dir.path(), "c.png", BLUE);

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&["c.png", "a.png"]),
                &locked(&[(&a, Some(10))]),
            );

            assert_eq!(plan.uploads.len(), 1);
            assert!(
                !plan.needs_reorder,
                "with an upload pending, the order comes from desired_order instead"
            );
            assert!(!plan.is_empty());
        }

        /// Two identical images at different positions must consume two
        /// distinct lock entries. Matching one entry twice would leave the
        /// other unconsumed, and it would be deleted out from under a slot
        /// that still points at it.
        #[test]
        fn duplicate_images_consume_one_lock_entry_each() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);
            std::fs::copy(dir.path().join("a.png"), dir.path().join("copy.png")).expect("copy");

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&["a.png", "copy.png"]),
                &locked(&[(&a, Some(10)), (&a, Some(11))]),
            );

            assert!(
                plan.deletes.is_empty(),
                "both lock entries are claimed, so neither is deleted: {plan:?}"
            );
            assert!(plan.uploads.is_empty());
            assert_eq!(slot_ids(&plan), vec![Some(10), Some(11)]);
        }

        /// The same image twice locally against a single lock entry: the first
        /// slot reuses it, the second has to be a fresh upload.
        #[test]
        fn a_duplicate_beyond_the_locked_copies_is_uploaded_again() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);
            std::fs::copy(dir.path().join("a.png"), dir.path().join("copy.png")).expect("copy");

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&["a.png", "copy.png"]),
                &locked(&[(&a, Some(10))]),
            );

            assert_eq!(slot_ids(&plan), vec![Some(10), None]);
            assert_eq!(plan.uploads.len(), 1);
            assert_eq!(plan.uploads[0].slot_index, 1);
            assert!(plan.deletes.is_empty());
        }

        /// Replacing the middle of a list: one delete, one upload, and the
        /// untouched neighbours keep their image IDs in place.
        #[test]
        fn replacing_one_of_three_touches_only_that_slot() {
            let dir = tempfile::tempdir().expect("tempdir");
            let a = put_image(dir.path(), "a.png", RED);
            let b = put_image(dir.path(), "b.png", GREEN);
            let c = put_image(dir.path(), "c.png", BLUE);
            put_image(dir.path(), "d.png", WHITE);

            let plan = thumbnail_plan(
                dir.path(),
                &media_with(&["a.png", "d.png", "c.png"]),
                &locked(&[(&a, Some(10)), (&b, Some(20)), (&c, Some(30))]),
            );

            assert_eq!(plan.deletes, vec![20]);
            assert_eq!(plan.uploads.len(), 1);
            assert_eq!(plan.uploads[0].slot_index, 1);
            assert_eq!(slot_ids(&plan), vec![Some(10), None, Some(30)]);
        }

        #[test]
        fn upload_bytes_are_the_processed_image_not_the_file_on_disk() {
            let dir = tempfile::tempdir().expect("tempdir");
            let expected_hash = put_image(dir.path(), "a.png", RED);

            let plan = thumbnail_plan(dir.path(), &media_with(&["a.png"]), &locked(&[]));

            assert_eq!(plan.uploads[0].hash, expected_hash);
            assert_eq!(hash_bytes(&plan.uploads[0].bytes), expected_hash);
        }

        #[test]
        fn a_missing_thumbnail_file_is_an_error_not_a_silent_skip() {
            let dir = tempfile::tempdir().expect("tempdir");

            let err = build_plan(
                &game(),
                &media_with(&["absent.png"]),
                &lock(),
                &MediaLockfile::default(),
                dir.path(),
            )
            .expect_err("a config naming a file that is not there must fail");

            assert!(format!("{err:#}").contains("absent.png"), "{err:#}");
        }
    }

    // -----------------------------------------------------------------------
    // desired_order
    //
    // The final image-ID order sent to Roblox. Kept images already have their
    // IDs; new uploads only get theirs once the upload call returns, and the
    // sync loop appends them to the lockfile in upload order. So the config
    // order and the post-upload order are different lists, and this function
    // is what reconciles them.
    // -----------------------------------------------------------------------

    mod ordering {
        use super::*;

        fn plan(slots: Vec<ThumbSlot>, uploads: Vec<usize>) -> ThumbnailPlan {
            ThumbnailPlan {
                deletes: Vec::new(),
                uploads: uploads
                    .into_iter()
                    .map(|slot_index| ThumbUpload {
                        bytes: Vec::new(),
                        hash: String::new(),
                        path: PathBuf::new(),
                        slot_index,
                    })
                    .collect(),
                slots,
                needs_reorder: false,
            }
        }

        fn keep(image_id: u64) -> ThumbSlot {
            ThumbSlot::Keep {
                hash: String::new(),
                image_id,
            }
        }

        fn new_upload() -> ThumbSlot {
            ThumbSlot::NewUpload {
                hash: String::new(),
            }
        }

        #[test]
        fn kept_images_come_back_in_config_order() {
            let p = plan(vec![keep(30), keep(10), keep(20)], vec![]);

            assert_eq!(desired_order(&p, &[]), vec![30, 10, 20]);
        }

        #[test]
        fn a_fresh_upload_takes_the_id_the_upload_returned() {
            let p = plan(vec![new_upload()], vec![0]);

            assert_eq!(desired_order(&p, &[Some(99)]), vec![99]);
        }

        /// The case the reorder exists for. Uploads are appended to the end of
        /// the remote list, but the config wants the new image first — so the
        /// desired order is not the post-upload order, and sending it is the
        /// only thing that puts the new thumbnail where the user asked.
        #[test]
        fn a_new_upload_is_placed_where_the_config_asks_not_where_it_landed() {
            let p = plan(vec![new_upload(), keep(10), keep(20)], vec![0]);

            assert_eq!(
                desired_order(&p, &[Some(99)]),
                vec![99, 10, 20],
                "the new image goes first, ahead of the images that already existed"
            );
        }

        #[test]
        fn uploads_interleaved_with_kept_images_land_in_their_own_slots() {
            let p = plan(
                vec![keep(10), new_upload(), keep(20), new_upload()],
                vec![1, 3],
            );

            assert_eq!(
                desired_order(&p, &[Some(98), Some(99)]),
                vec![10, 98, 20, 99]
            );
        }

        /// Uploads are indexed by their position in `uploads`, not by slot, so
        /// a plan whose uploads are not in slot order still maps correctly.
        #[test]
        fn upload_ids_are_matched_by_upload_index_not_by_slot_position() {
            let p = plan(vec![new_upload(), new_upload()], vec![1, 0]);

            assert_eq!(
                desired_order(&p, &[Some(98), Some(99)]),
                vec![99, 98],
                "uploads[0] fills slot 1, so its id belongs second"
            );
        }

        /// An upload whose response carried no image ID is skipped rather than
        /// poisoning the order with a placeholder. The rest still reorder.
        #[test]
        fn an_upload_with_no_returned_id_is_left_out_of_the_order() {
            let p = plan(vec![keep(10), new_upload(), keep(20)], vec![1]);

            assert_eq!(desired_order(&p, &[None]), vec![10, 20]);
        }

        #[test]
        fn a_short_id_list_does_not_panic() {
            let p = plan(vec![new_upload(), new_upload()], vec![0, 1]);

            assert_eq!(desired_order(&p, &[Some(98)]), vec![98]);
        }

        #[test]
        fn an_empty_plan_has_no_order_to_send() {
            assert!(desired_order(&plan(vec![], vec![]), &[]).is_empty());
        }
    }

    // -----------------------------------------------------------------------
    // The universe configuration fields that only the cookie can write
    // -----------------------------------------------------------------------

    mod universe_legacy {
        use super::*;
        use crate::config::{
            AnimationType, AssetOverride, AssetOverrides, Avatar, AvatarScales, AvatarType,
            CollisionType, Genre, JointPositioningType, PaidAccess, Permissions,
        };

        fn body(g: &Game, l: &GameLock) -> Value {
            plan_for(g, l)
                .universe_legacy_patch
                .expect("a legacy patch")
                .body
        }

        fn perms() -> Permissions {
            Permissions {
                third_party_teleport: true,
                third_party_asset: false,
                third_party_purchase: true,
                client_teleport: false,
            }
        }

        /// The object goes out whole, with the PascalCase keys Roblox uses.
        /// Sending a subset would leave the unstated flags at values nothing
        /// here can read back.
        #[test]
        fn permissions_are_sent_as_one_object() {
            let mut g = game();
            g.permissions = Some(perms());

            assert_eq!(
                body(&g, &lock()),
                json!({ "permissions": {
                    "IsThirdPartyTeleportAllowed": true,
                    "IsThirdPartyAssetAllowed": false,
                    "IsThirdPartyPurchaseAllowed": true,
                    "IsClientTeleportAllowed": false,
                }})
            );
        }

        #[test]
        fn permissions_matching_the_lock_send_nothing() {
            let mut g = game();
            g.permissions = Some(perms());

            assert!(plan_for(&g, &config_to_lock(&g))
                .universe_legacy_patch
                .is_none());
        }

        /// The three avatar modes map to integers Roblox chose, and the
        /// mapping is not alphabetical: `player_choice` sits between the two
        /// rigs. Asserting the numbers is the only way this stays right.
        #[test]
        fn avatar_modes_map_to_their_legacy_integers() {
            let mut g = game();
            g.avatar = Avatar {
                kind: Some(AvatarType::R15),
                animation: Some(AnimationType::PlayerChoice),
                collision: Some(CollisionType::OuterBox),
                joint_positioning: Some(JointPositioningType::ArtistIntent),
                min_scale: None,
                max_scale: None,
                asset_overrides: None,
            };

            assert_eq!(
                body(&g, &lock()),
                json!({
                    "universeAvatarType": 3,
                    "universeAnimationType": 2,
                    "universeCollisionType": 2,
                    "universeJointPositioningType": 2,
                })
            );
        }

        #[test]
        fn r6_is_one_and_player_choice_is_two() {
            let mut g = game();
            g.avatar.kind = Some(AvatarType::R6);
            assert_eq!(body(&g, &lock()), json!({ "universeAvatarType": 1 }));

            g.avatar.kind = Some(AvatarType::PlayerChoice);
            assert_eq!(body(&g, &lock()), json!({ "universeAvatarType": 2 }));
        }

        #[test]
        fn a_scale_table_is_sent_with_roblox_key_casing() {
            let mut g = game();
            g.avatar.min_scale = Some(AvatarScales {
                height: 0.9,
                width: 0.7,
                head: 0.95,
                body_type: 0.0,
                proportion: 0.0,
            });

            assert_eq!(
                body(&g, &lock()),
                json!({ "universeAvatarMinScales": {
                    "height": 0.9,
                    "width": 0.7,
                    "head": 0.95,
                    "bodyType": 0.0,
                    "proportion": 0.0,
                }})
            );
        }

        /// A price without `isForSale` is a price Roblox ignores, and
        /// `isForSale` without a price is a paid experience that is free by
        /// accident. They travel together.
        #[test]
        fn a_paid_experience_sends_both_the_flag_and_the_price() {
            let mut g = game();
            g.paid_access = Some(PaidAccess::Paid { price: 25 });

            assert_eq!(body(&g, &lock()), json!({ "isForSale": true, "price": 25 }));
        }

        /// `free` is an instruction, not an absence: it turns paid access off.
        /// No price goes with it, because there is nothing to charge.
        #[test]
        fn free_sends_the_flag_and_no_price() {
            let mut g = game();
            g.paid_access = Some(PaidAccess::Free);

            assert_eq!(body(&g, &lock()), json!({ "isForSale": false }));
        }

        /// Omitting the table entirely means "not managed", which is a third
        /// state and has to stay distinct from `free`.
        #[test]
        fn an_absent_paid_access_table_sends_nothing() {
            let mut g = game();
            g.paid_access = None;
            g.genre = None;

            assert!(plan_for(&g, &lock()).universe_legacy_patch.is_none());
        }

        #[test]
        fn genres_map_to_their_legacy_integers() {
            let mut g = game();
            g.genre = Some(Genre::All);
            assert_eq!(body(&g, &lock()), json!({ "genre": 0 }));

            g.genre = Some(Genre::WildWest);
            assert_eq!(body(&g, &lock()), json!({ "genre": 14 }));
        }

        /// Every legacy integer Roblox documents round-trips. A mapping that
        /// was wrong in one direction only would otherwise survive a pull and
        /// a sync, and change the setting on the third run.
        #[test]
        fn every_legacy_mapping_round_trips() {
            for v in 1..=3 {
                assert_eq!(
                    AvatarType::from_legacy(v).map(AvatarType::to_legacy),
                    Some(v)
                );
            }
            for v in 1..=2 {
                assert_eq!(
                    AnimationType::from_legacy(v).map(AnimationType::to_legacy),
                    Some(v)
                );
                assert_eq!(
                    CollisionType::from_legacy(v).map(CollisionType::to_legacy),
                    Some(v)
                );
                assert_eq!(
                    JointPositioningType::from_legacy(v).map(JointPositioningType::to_legacy),
                    Some(v)
                );
            }
            for v in 0..=14 {
                assert_eq!(Genre::from_legacy(v).map(Genre::to_legacy), Some(v));
            }
        }

        /// An integer Roblox has not defined is not silently coerced to the
        /// first variant: a pull that met one would otherwise write a wrong
        /// value into the config, and a sync would then apply it.
        #[test]
        fn an_unknown_legacy_integer_is_rejected() {
            assert!(AvatarType::from_legacy(0).is_none());
            assert!(AvatarType::from_legacy(4).is_none());
            assert!(AnimationType::from_legacy(3).is_none());
            assert!(Genre::from_legacy(15).is_none());
        }

        /// These are cookie-only settings. A plan that carried them without
        /// saying so would be refused at apply time with a confusing message.
        #[test]
        fn the_new_fields_make_the_plan_need_a_cookie() {
            let mut g = game();
            g.permissions = Some(perms());

            assert!(plan_for(&g, &lock()).needs_cookie());
        }

        // -------------------------------------------------------------------
        // Avatar slot overrides
        // -------------------------------------------------------------------

        fn choice() -> AssetOverride {
            AssetOverride::PlayerChoice(crate::config::PlayerChoiceMarker::PlayerChoice)
        }

        fn slots() -> AssetOverrides {
            let c = choice();
            AssetOverrides {
                face: c,
                head: c,
                torso: c,
                left_arm: c,
                right_arm: c,
                left_leg: c,
                right_leg: c,
                t_shirt: c,
                shirt: c,
                pants: c,
            }
        }

        /// The `assetTypeID` numbers are Roblox's global asset-type numbering
        /// and are neither contiguous nor in the order the slots read — `Head`
        /// is 17 while `Torso` is 27, and `TShirt` is 2 while `Shirt` is 11.
        /// Nothing but an assertion keeps that table right.
        #[test]
        fn every_slot_maps_to_its_roblox_asset_type_id() {
            let mut g = game();
            g.avatar.asset_overrides = Some(slots());

            let sent = body(&g, &lock());
            let array = sent["universeAvatarAssetOverrides"].as_array().unwrap();

            let ids: Vec<i64> = array
                .iter()
                .map(|slot| slot["assetTypeID"].as_i64().unwrap())
                .collect();
            assert_eq!(ids, vec![18, 17, 27, 29, 28, 30, 31, 2, 11, 12]);
        }

        /// `player_choice` is `isPlayerChoice: true` with a zero id, and an
        /// override is the reverse. Sending an id alongside `isPlayerChoice:
        /// true` would be asking for two different things at once.
        #[test]
        fn a_forced_asset_and_a_player_choice_slot_differ_on_both_fields() {
            let mut g = game();
            g.avatar.asset_overrides = Some(AssetOverrides {
                pants: AssetOverride::Asset(12345),
                ..slots()
            });

            let sent = body(&g, &lock());
            let array = sent["universeAvatarAssetOverrides"].as_array().unwrap();

            let pants = array
                .iter()
                .find(|slot| slot["assetTypeID"] == 12)
                .expect("the pants slot");
            assert_eq!(pants["isPlayerChoice"], json!(false));
            assert_eq!(pants["assetID"], json!(12345));

            let shirt = array
                .iter()
                .find(|slot| slot["assetTypeID"] == 11)
                .expect("the shirt slot");
            assert_eq!(shirt["isPlayerChoice"], json!(true));
            assert_eq!(shirt["assetID"], json!(0));
        }

        /// All ten slots always travel, because Roblox replaces the array
        /// rather than merging into it.
        #[test]
        fn all_ten_slots_are_always_sent() {
            let mut g = game();
            g.avatar.asset_overrides = Some(slots());

            let sent = body(&g, &lock());
            assert_eq!(
                sent["universeAvatarAssetOverrides"]
                    .as_array()
                    .unwrap()
                    .len(),
                10
            );
        }

        #[test]
        fn slots_matching_the_lock_send_nothing() {
            let mut g = game();
            g.avatar.asset_overrides = Some(slots());

            assert!(plan_for(&g, &config_to_lock(&g))
                .universe_legacy_patch
                .is_none());
        }

        // -------------------------------------------------------------------
        // engineAvatarSettings
        // -------------------------------------------------------------------

        /// `build_plan` with an `engine_avatar_settings` file on disk.
        ///
        /// Needs a real directory, unlike every other test here, because the
        /// whole point of the field is that the document comes from a file.
        fn plan_with_engine_settings(
            lock: &GameLock,
            contents: &str,
        ) -> (tempfile::TempDir, Result<SyncPlan>) {
            plan_with_engine_file(lock, "avatar.json", contents)
        }

        fn plan_with_engine_file(
            lock: &GameLock,
            name: &str,
            contents: &str,
        ) -> (tempfile::TempDir, Result<SyncPlan>) {
            let dir = tempfile::tempdir().expect("tempdir");
            std::fs::write(dir.path().join(name), contents).expect("write");
            let mut g = game();
            g.engine_avatar_settings = Some(PathBuf::from(name));
            let plan = build_plan(
                &g,
                &MediaConfig::default(),
                lock,
                &MediaLockfile::default(),
                dir.path(),
            );
            (dir, plan)
        }

        /// The document this sends, whatever format it came from.
        fn engine_document(plan: Result<SyncPlan>) -> String {
            plan.unwrap().universe_legacy_patch.expect("a patch").body["engineAvatarSettings"]
                .as_str()
                .expect("a JSON string")
                .to_string()
        }

        /// The field is typed as a JSON *string* on the API, so the document is
        /// serialised and handed over as text rather than nested as an object.
        #[test]
        fn the_document_is_sent_as_a_json_string() {
            let (_dir, plan) =
                plan_with_engine_settings(&lock(), r#"{"AvatarRules":{"AvatarType":1}}"#);
            let patch = plan.unwrap().universe_legacy_patch.expect("a patch");

            let sent = patch.body["engineAvatarSettings"]
                .as_str()
                .expect("a string, not an object");
            assert_eq!(sent, r#"{"AvatarRules":{"AvatarType":1}}"#);
        }

        /// Reindenting the file is not a change to re-send. The hash is of the
        /// canonical serialisation, so whitespace never reaches the wire.
        #[test]
        fn reformatting_the_file_is_not_a_change() {
            let (_dir, plan) = plan_with_engine_settings(&lock(), r#"{"a":1}"#);
            let hash = plan
                .unwrap()
                .universe_legacy_patch
                .expect("a patch")
                .engine_avatar_settings_hash
                .expect("a hash");

            let mut locked = lock();
            locked.engine_avatar_settings_hash = Some(hash);

            let (_dir2, plan2) = plan_with_engine_settings(&locked, "{\n    \"a\" :   1\n}\n");
            assert!(
                plan2.unwrap().universe_legacy_patch.is_none(),
                "the same document, formatted differently, must not re-send"
            );
        }

        /// A real edit does re-send.
        #[test]
        fn a_changed_document_is_sent() {
            let (_dir, plan) = plan_with_engine_settings(&lock(), r#"{"a":1}"#);
            let hash = plan
                .unwrap()
                .universe_legacy_patch
                .expect("a patch")
                .engine_avatar_settings_hash
                .expect("a hash");

            let mut locked = lock();
            locked.engine_avatar_settings_hash = Some(hash);

            let (_dir2, plan2) = plan_with_engine_settings(&locked, r#"{"a":2}"#);
            assert!(plan2.unwrap().universe_legacy_patch.is_some());
        }

        /// Caught locally, before a cookie-authenticated write that would come
        /// back as an opaque 400.
        #[test]
        fn a_file_that_is_not_json_is_refused_by_name() {
            let (_dir, plan) = plan_with_engine_settings(&lock(), "AvatarType = 1");
            let error = format!("{:#}", plan.unwrap_err());

            assert!(error.contains("avatar.json"), "{error}");
            assert!(error.contains("not valid JSON"), "{error}");
        }

        #[test]
        fn a_missing_file_is_refused_by_name() {
            let dir = tempfile::tempdir().unwrap();
            let mut g = game();
            g.engine_avatar_settings = Some(PathBuf::from("nowhere.json"));

            let error = build_plan(
                &g,
                &MediaConfig::default(),
                &lock(),
                &MediaLockfile::default(),
                dir.path(),
            )
            .unwrap_err();

            assert!(format!("{error:#}").contains("nowhere.json"));
        }

        /// `{}` is how Roblox documents clearing the settings, so it has to
        /// reach the wire rather than being treated as "nothing to send".
        #[test]
        fn an_empty_object_is_a_real_instruction() {
            let (_dir, plan) = plan_with_engine_settings(&lock(), "{}");
            let patch = plan.unwrap().universe_legacy_patch.expect("a patch");

            assert_eq!(patch.body["engineAvatarSettings"], json!("{}"));
        }

        /// A TOML document reaches Roblox as the JSON its field is typed as.
        /// The point of accepting TOML is that the project keeps one config
        /// language, not that Roblox learns a second one.
        #[test]
        fn a_toml_document_is_sent_as_json() {
            let (_dir, plan) =
                plan_with_engine_file(&lock(), "avatar.toml", "[AvatarRules]\nAvatarType = 1\n");

            assert_eq!(engine_document(plan), r#"{"AvatarRules":{"AvatarType":1}}"#);
        }

        /// The two formats are interchangeable, which is the property that
        /// makes converting a file a no-op rather than a spurious re-send.
        #[test]
        fn the_same_document_hashes_the_same_in_either_format() {
            let (_dir, json_plan) = plan_with_engine_file(
                &lock(),
                "avatar.json",
                r#"{"AvatarRules": {"AvatarType": 1}, "version": 1}"#,
            );
            let (_dir2, toml_plan) = plan_with_engine_file(
                &lock(),
                "avatar.toml",
                "version = 1\n\n[AvatarRules]\nAvatarType = 1\n",
            );

            assert_eq!(engine_document(json_plan), engine_document(toml_plan));
        }

        /// Numbers have to survive the conversion with their type intact:
        /// `AvatarType = 1` is an integer to Roblox, and a `1.0` would be a
        /// different value.
        #[test]
        fn toml_integers_and_floats_keep_their_types() {
            let (_dir, plan) = plan_with_engine_file(
                &lock(),
                "avatar.toml",
                "mode = 1\nscale = 1.5\nenabled = true\nbounds = [0, 0, 0]\n",
            );
            let sent = engine_document(plan);

            assert!(sent.contains(r#""mode":1"#), "{sent}");
            assert!(sent.contains(r#""scale":1.5"#), "{sent}");
            assert!(sent.contains(r#""enabled":true"#), "{sent}");
            assert!(sent.contains(r#""bounds":[0,0,0]"#), "{sent}");
        }

        #[test]
        fn a_file_that_is_not_toml_is_refused_by_name() {
            let (_dir, plan) = plan_with_engine_file(&lock(), "avatar.toml", "{\"not\": \"toml\"}");
            let error = format!("{:#}", plan.unwrap_err());

            assert!(error.contains("avatar.toml"), "{error}");
            assert!(error.contains("not valid TOML"), "{error}");
        }

        /// Sniffing the content instead would let a `.txt` through, and turn a
        /// typo in the path into a silent success.
        #[test]
        fn an_unrecognised_extension_is_refused_rather_than_sniffed() {
            let (_dir, plan) = plan_with_engine_file(&lock(), "avatar.yaml", "{}");
            let error = format!("{:#}", plan.unwrap_err());

            assert!(error.contains(".toml or .json"), "{error}");
            assert!(error.contains("avatar.yaml"), "{error}");
        }
    }
}
