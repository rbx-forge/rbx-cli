use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::config::{Game, MediaConfig, Visibility};
use crate::lockfile::{GameLock, MediaLockfile};

mod media;
mod place;
mod show;
mod universe;

#[cfg(test)]
mod tests;

// Re-exported so the four modules reach each other through `super::*`, and so
// every `crate::diff::build_plan` outside stays where it was.
pub(crate) use self::{media::*, place::*, show::*, universe::*};
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
    /// it is now asked twice (once to refuse early, once inside `apply_plan`
    /// where the guarantee has to hold whatever the caller did) and two copies
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
/// No updateMask: the legacy API accepts any subset of fields.
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
        // the hash is of a file it has no path to resolve: `sync` writes it
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
// place. They are pure (config + lockfile in, patch out) so they are cheap
// to pin exactly, and every assertion below is on the exact body and mask
// rather than on the patch merely existing.
// ---------------------------------------------------------------------------
