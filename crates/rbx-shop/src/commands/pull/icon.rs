//! Which icon wins for an env: the local file, the remote one, or a conflict
//! the operator has to settle.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::ResourceKind;

use super::*;

pub(super) enum IconResolution {
    SetNone,
    PreserveOld,
    PendingDownload,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_icon(
    env: &str,
    kind: ResourceKind,
    name: &str,
    resource_id: u64,
    old_icon_id: Option<&u64>,
    new_icon_id: &Option<u64>,
    local_icon: Option<&PathBuf>,
    config_dir: &Path,
    icon_dir: &Path,
    accept_remote: bool,
    accept_local: bool,
    conflicts: &mut Vec<IconConflict>,
    downloads: &mut Vec<PendingDownload>,
) -> Result<IconResolution> {
    let icon_changed = match (old_icon_id, new_icon_id.as_ref()) {
        (Some(old), Some(new)) => old != new,
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => false,
    };

    if !icon_changed {
        return Ok(IconResolution::PreserveOld);
    }

    if accept_remote {
        if let Some(&asset_id) = new_icon_id.as_ref() {
            let save_path = if let Some(local_path) = local_icon {
                config_dir.join(local_path)
            } else {
                // Same rule as `shop init`: the display name is Roblox's, and
                // Roblox allows characters Windows refuses in a filename. See
                // `rbx_core::fs_name`.
                let safe = rbx_core::fs_name::safe_component(name);
                let stem = if safe.is_empty() {
                    format!("{kind}-{resource_id}")
                } else {
                    format!("{kind}-{resource_id}-{safe}")
                };
                config_dir.join(format!("{}/{stem}.png", icon_dir.display()))
            };
            downloads.push(PendingDownload {
                env: env.to_string(),
                kind,
                name: name.to_string(),
                asset_id,
                save_path,
            });
            return Ok(IconResolution::PendingDownload);
        } else {
            return Ok(IconResolution::SetNone);
        }
    }

    if accept_local {
        return Ok(IconResolution::SetNone);
    }

    let Some(local_path) = local_icon else {
        return Ok(IconResolution::SetNone);
    };

    let full_path = config_dir.join(local_path);
    let local_hash = hash_file(&full_path)?;

    conflicts.push(IconConflict {
        env: env.to_string(),
        kind,
        name: name.to_string(),
        local_path: local_path.display().to_string(),
        local_hash,
        remote_asset_id: new_icon_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string()),
    });
    Ok(IconResolution::SetNone)
}

pub(super) fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(rbx_core::image::hash_bytes(&bytes))
}
