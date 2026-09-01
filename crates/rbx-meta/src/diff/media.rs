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
