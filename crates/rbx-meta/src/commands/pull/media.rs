//! Media download, reached by --accept-remote.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

use crate::api::media::RemoteMedia;
use crate::api::RbxClient;
use crate::config::{Config, MediaConfig};
use crate::lockfile::{Lockfile, MediaLock, DEFAULT_ENV};
use rbx_core::image::{hash_bytes, process_bytes};

pub(super) async fn download_media(
    client: &RbxClient,
    config: &mut Config,
    lockfile: &mut Lockfile,
    env: &str,
    config_dir: &Path,
    lockfile_path: &Path,
) -> Result<()> {
    let media_for_dir = effective_media(config, env);
    let dir_rel = media_for_dir
        .dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("assets"));
    // For named envs we sub-namespace the default media directory so two envs
    // never collide on the same default file name (e.g. assets/icon.png).
    let env_specific = env != DEFAULT_ENV;
    let env_dir_rel = if env_specific {
        dir_rel.join(env)
    } else {
        dir_rel.clone()
    };

    // Snapshot the lockfile media state BEFORE we mutate it. Used for cached
    // checks (image_id match → skip download). Mutations happen on
    // `lockfile.env_mut(env).media` and are persisted after each operation.
    let prior_media = lockfile.env_view(env).media.clone();

    // Icon.
    match client.fetch_icon().await? {
        Some(RemoteMedia {
            target_id,
            image_url,
        }) => {
            let effective = effective_media(config, env);
            // Prefer the env's explicit overlay path; fall back to base only in
            // single-env mode; otherwise default to assets/<env>/icon.png so
            // sibling envs do not overwrite each other.
            let overlay_path = config.envs.get(env).and_then(|o| o.media.icon.clone());
            let icon_rel: PathBuf = if let Some(p) = overlay_path {
                p
            } else if env_specific {
                env_dir_rel.join("icon.png")
            } else {
                effective
                    .icon
                    .clone()
                    .unwrap_or_else(|| dir_rel.join("icon.png"))
            };
            let icon_full = config_dir.join(&icon_rel);

            let cached = prior_media
                .icon
                .as_ref()
                .map(|l| l.image_id == Some(target_id))
                .unwrap_or(false);
            let local_present = icon_full.exists();

            if cached && local_present {
                println!(
                    "\n{} icon unchanged (image_id {}, {})",
                    "·".dimmed(),
                    target_id,
                    icon_full.display()
                );
            } else {
                println!(
                    "\n{} icon (image_id {})...",
                    "Downloading".cyan(),
                    target_id
                );
                let bytes = client.download_bytes(&image_url).await?;
                if let Some(p) = icon_full.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&icon_full, &bytes)
                    .with_context(|| format!("Failed to write {}", icon_full.display()))?;
                let processed = process_bytes(&bytes, effective.bleed)?;
                let hash = hash_bytes(&processed);
                lockfile.env_mut(env).media.icon = Some(MediaLock {
                    hash,
                    image_id: Some(target_id),
                });
                lockfile.save(lockfile_path)?;

                // Named envs always write the path as an overlay so they never
                // hijack the base. Standalone (DEFAULT_ENV) uses the
                // promote-to-base differential.
                if env_specific {
                    config.envs.entry(env.to_string()).or_default().media.icon = Some(icon_rel);
                } else {
                    apply_media_path(
                        &mut config.media.icon,
                        &mut config.envs.entry(env.to_string()).or_default().media.icon,
                        Some(icon_rel),
                    );
                }
                println!("  {} saved to {}", "✓".green(), icon_full.display());
            }
        }
        None => {
            println!("\n{} no icon on Roblox", "✗".dimmed());
            lockfile.env_mut(env).media.icon = None;
            lockfile.save(lockfile_path)?;
        }
    }

    // Thumbnails. Per-index check: reuse if image_id matches and local file exists.
    // After each download (or skip), the lockfile is updated at index i and
    // saved so a crash mid-loop leaves the lockfile consistent with what's been
    // downloaded to disk.
    let remote_thumbs = client.fetch_thumbnails().await?;
    if remote_thumbs.is_empty() {
        lockfile.env_mut(env).media.thumbnails = Vec::new();
        lockfile.save(lockfile_path)?;
    } else {
        println!(
            "\n{} {} thumbnail(s)...",
            "Syncing".cyan(),
            remote_thumbs.len()
        );
        let mut new_paths: Vec<PathBuf> = Vec::with_capacity(remote_thumbs.len());
        let effective = effective_media(config, env);
        let overlay_thumbs = config
            .envs
            .get(env)
            .and_then(|o| o.media.thumbnails.clone());
        for (
            i,
            RemoteMedia {
                target_id,
                image_url,
            },
        ) in remote_thumbs.iter().enumerate()
        {
            // Pick the destination path with the same fallback rules as for
            // the icon: overlay (env) > effective base (single-env) > env-namespaced default.
            let overlay_path = overlay_thumbs.as_ref().and_then(|v| v.get(i)).cloned();
            let path_rel: PathBuf = if let Some(p) = overlay_path {
                p
            } else if env_specific {
                env_dir_rel.join(format!("thumbnail_{:02}.png", i + 1))
            } else {
                effective
                    .thumbnails
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| dir_rel.join(format!("thumbnail_{:02}.png", i + 1)))
            };
            let path_full = config_dir.join(&path_rel);

            let cached = prior_media
                .thumbnails
                .get(i)
                .map(|l| l.image_id == Some(*target_id))
                .unwrap_or(false);
            let local_present = path_full.exists();

            let new_lock = if cached && local_present {
                println!(
                    "  {} {} unchanged (image_id {})",
                    "·".dimmed(),
                    path_full.display(),
                    target_id
                );
                prior_media.thumbnails[i].clone()
            } else {
                let bytes = client.download_bytes(image_url).await?;
                if let Some(p) = path_full.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&path_full, &bytes)
                    .with_context(|| format!("Failed to write {}", path_full.display()))?;
                let processed = process_bytes(&bytes, effective.bleed)?;
                let hash = hash_bytes(&processed);
                println!(
                    "  {} {} (image_id {})",
                    "✓".green(),
                    path_full.display(),
                    target_id
                );
                MediaLock {
                    hash,
                    image_id: Some(*target_id),
                }
            };

            // Insert/replace at index i and persist.
            {
                let media = &mut lockfile.env_mut(env).media;
                if media.thumbnails.len() > i {
                    media.thumbnails[i] = new_lock;
                } else {
                    media.thumbnails.push(new_lock);
                }
            }
            lockfile.save(lockfile_path)?;
            new_paths.push(path_rel);
        }

        // Drop trailing entries that no longer correspond to a remote thumbnail.
        {
            let media = &mut lockfile.env_mut(env).media;
            if media.thumbnails.len() > remote_thumbs.len() {
                media.thumbnails.truncate(remote_thumbs.len());
            }
        }
        lockfile.save(lockfile_path)?;

        // Named envs always write the paths as an overlay so they never hijack
        // the base. Standalone (DEFAULT_ENV) uses the promote-to-base diff.
        if env_specific {
            config
                .envs
                .entry(env.to_string())
                .or_default()
                .media
                .thumbnails = Some(new_paths);
        } else {
            apply_media_thumbnails(
                &mut config.media.thumbnails,
                &mut config
                    .envs
                    .entry(env.to_string())
                    .or_default()
                    .media
                    .thumbnails,
                new_paths,
            );
        }
    }

    Ok(())
}

/// Resolved (base + env overlay) MediaConfig for the current pull state.
pub(super) fn effective_media(config: &Config, env: &str) -> MediaConfig {
    let mut media = config.media.clone();
    if let Some(overlay) = config.envs.get(env) {
        media.apply_overlay(&overlay.media);
    }
    media
}

pub(super) fn apply_media_path(
    base: &mut Option<PathBuf>,
    overlay: &mut Option<PathBuf>,
    remote: Option<PathBuf>,
) {
    let Some(r) = remote else { return };
    match base {
        None => {
            *base = Some(r);
            *overlay = None;
        }
        Some(b) if *b == r => {
            *overlay = None;
        }
        Some(_) => {
            *overlay = Some(r);
        }
    }
}

pub(super) fn apply_media_thumbnails(
    base: &mut Vec<PathBuf>,
    overlay: &mut Option<Vec<PathBuf>>,
    remote: Vec<PathBuf>,
) {
    if base.is_empty() {
        *base = remote;
        *overlay = None;
    } else if *base == remote {
        *overlay = None;
    } else {
        *overlay = Some(remote);
    }
}
