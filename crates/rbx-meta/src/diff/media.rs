//! The icon and the thumbnails: which files have to be uploaded, and in what
//! order the result has to be arranged.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::MediaConfig;
use crate::lockfile::MediaLockfile;
use rbx_core::image::{hash_bytes, process_image};

use super::*;

pub(crate) fn build_icon_plan(
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

pub(crate) fn build_thumbnail_plan(
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
                // Lockfile entry without an image_id: treat as new upload.
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

/// The Home Page thumbnails, matched by hash against the lockfile like the
/// experience's thumbnails, each lockfile entry consumed at most once so a
/// duplicated file is uploaded twice rather than listed twice.
pub(crate) fn build_home_thumbnail_plan(
    media: &MediaConfig,
    media_lock: &MediaLockfile,
    config_dir: &Path,
) -> Result<HomeThumbnailPlan> {
    let mut plan = HomeThumbnailPlan::default();
    let mut used = vec![false; media_lock.home_thumbnails.len()];

    for file in &media.home_thumbnails {
        let bytes = process_image(&config_dir.join(file), media.bleed)?;
        let hash = hash_bytes(&bytes);
        let matched = media_lock
            .home_thumbnails
            .iter()
            .enumerate()
            .find(|(i, entry)| !used[*i] && entry.hash == hash);
        match matched {
            Some((i, entry)) => {
                used[i] = true;
                plan.slots.push(HomeSlot::Keep {
                    hash,
                    id: entry.homepage_thumbnail_id.clone(),
                });
            }
            None => {
                plan.slots.push(HomeSlot::New { hash: hash.clone() });
                plan.uploads.push(HomeUpload {
                    bytes,
                    hash,
                    path: file.clone(),
                });
            }
        }
    }

    plan.deletes = media_lock
        .home_thumbnails
        .iter()
        .enumerate()
        .filter(|(i, _)| !used[*i])
        .map(|(_, entry)| entry.homepage_thumbnail_id.clone())
        .collect();

    let kept: Vec<&str> = plan
        .slots
        .iter()
        .filter_map(|slot| match slot {
            HomeSlot::Keep { id, .. } => Some(id.as_str()),
            HomeSlot::New { .. } => None,
        })
        .collect();
    let locked: Vec<&str> = media_lock
        .home_thumbnails
        .iter()
        .map(|entry| entry.homepage_thumbnail_id.as_str())
        .collect();
    plan.needs_update = !plan.uploads.is_empty() || !plan.deletes.is_empty() || kept != locked;

    Ok(plan)
}
