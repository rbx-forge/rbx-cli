use std::path::PathBuf;

use anyhow::Result;
use serde_json::{json, Value};

use crate::api::models::ApiSocialLink;
use crate::config::{Game, MediaConfig, ServerFill, SocialLink, Visibility};
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
    config_dir: &std::path::Path,
) -> Result<SyncPlan> {
    Ok(SyncPlan {
        universe_patch: build_universe_patch(game, game_lock),
        place_patch: build_place_patch(game, game_lock),
        place_legacy_patch: build_place_legacy_patch(game, game_lock),
        universe_legacy_patch: build_universe_legacy_patch(game, game_lock),
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

fn build_universe_legacy_patch(game: &Game, lock: &GameLock) -> Option<UniverseLegacyPatch> {
    let mut body = serde_json::Map::new();
    let mut descriptions: Vec<String> = Vec::new();

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

    if body.is_empty() {
        None
    } else {
        Some(UniverseLegacyPatch {
            body: Value::Object(body),
            descriptions,
        })
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
    config_dir: &std::path::Path,
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
    config_dir: &std::path::Path,
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
}
