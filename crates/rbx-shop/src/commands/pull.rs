//! `rbx shop pull`: adopt what the live catalogue says into the config and the
//! lockfile.
//!
//! This file is the run: resolve the envs, ask once, walk them. Each step it
//! walks through is a module beside it, split along the section banners this
//! file used to carry.

mod env;
mod icon;
mod overlay;
mod print;

// Re-exported so `tests.rs`, which is a sibling of those modules rather than a
// child, still reaches them through `super::` as it always did.
use self::{env::*, icon::*, overlay::*, print::*};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use colored::Colorize;

use crate::config::{Config, ResourceKind};
use crate::ctx::ShopCtx;
use crate::lockfile::{Lockfile, LOCKFILE_NAME, LOCKFILE_VERSION};
use rbx_core::confirm::confirm_always;

struct IconConflict {
    env: String,
    kind: ResourceKind,
    name: String,
    local_path: String,
    local_hash: String,
    remote_asset_id: String,
}

/// An icon queued for download by `--accept-remote`.
///
/// `kind` was a `&'static str` even though `ResourceKind` already existed and
/// was used in the same expression, which meant every consumer needed a `_`
/// arm that silently did nothing.
struct PendingDownload {
    env: String,
    kind: ResourceKind,
    name: String,
    asset_id: u64,
    save_path: PathBuf,
}

pub async fn run(
    ctx: &ShopCtx<'_>,
    dry_run: bool,
    accept_remote: bool,
    accept_local: bool,
    yes: bool,
) -> Result<()> {
    let mut files = Config::load_all(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new("."));
    let lockfile_path = config_dir.join(LOCKFILE_NAME);

    let old_lockfile = Lockfile::load(&lockfile_path)?;
    let mut new_lockfile = old_lockfile.clone();
    new_lockfile.version = LOCKFILE_VERSION;

    let envs = ctx.resolve_envs(&files[0].config)?;

    let mut all_conflicts = Vec::new();
    let mut all_downloads = Vec::new();
    let mut had_any_changes = false;

    for env_target in &envs {
        if envs.len() > 1 {
            println!("\n{} {}", "env:".bold(), env_target.name.bold());
        }
        // Recomputed every iteration so a `--env all` run sees mutations
        // applied by a prior env in the same loop (matches the old
        // single-`Config` behavior, where all envs shared one mutable view).
        let merged = Config::merge_loaded(&files)?;
        let changed = pull_one_env(
            ctx,
            &merged,
            &mut files,
            config_dir,
            &old_lockfile,
            &mut new_lockfile,
            env_target,
            accept_remote,
            accept_local,
            dry_run,
            &mut all_conflicts,
            &mut all_downloads,
        )
        .await?;
        had_any_changes |= changed;
    }

    // Icon-only differences don't show up in the config/lockfile diffs, but
    // the queued downloads and conflicts below still rewrite local files:
    // count them as changes so dry-run reporting and the confirmation gate
    // see them.
    had_any_changes |= !all_conflicts.is_empty() || !all_downloads.is_empty();

    if dry_run {
        if !had_any_changes {
            println!("{} Already up to date with remote (all envs).", "✓".green());
        } else {
            println!("\nDry run: no changes applied.");
        }
        return Ok(());
    }

    // Report icon conflicts before asking for confirmation: they abort the
    // pull, so prompting first would make the user confirm a no-op.
    if !all_conflicts.is_empty() {
        println!();
        for c in &all_conflicts {
            println!(
                "{} [{}] {} '{}': icon differs from remote",
                "!".yellow(),
                c.env,
                c.kind,
                c.name.bold()
            );
            println!(
                "  Local:  {} (blake3: {}...)",
                c.local_path,
                &c.local_hash[..12.min(c.local_hash.len())]
            );
            println!("  Remote: asset {}", c.remote_asset_id);
        }
        println!();
        bail!(
            "Icon conflicts detected.\n  \
             Use --accept-remote to keep remote icons\n  \
             Use --accept-local to re-upload local icons on next sync"
        );
    }

    // Confirm before any local write (icon downloads, config rewrite,
    // lockfile rewrite). Only ask when there's something to apply; pure
    // already-up-to-date pulls fall through silently.
    if had_any_changes {
        let env_list: String = envs
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        confirm_always(
            &format!(
                "Overwrite local rbxshop.toml and lockfile for env(s) [{}]?",
                env_list
            ),
            yes,
        )?;
    }

    // Download remote icons (--accept-remote)
    let client_cache = build_client_cache(ctx, &envs, files[0].config.icons.bleed);
    for dl in &all_downloads {
        println!(
            "  {} [{}] Downloading {} '{}' icon...",
            "↓".cyan(),
            dl.env,
            dl.kind,
            dl.name
        );
        let client = client_cache
            .get(&dl.env)
            .expect("client should exist for env");
        let bytes = client.download_icon(dl.asset_id).await?;
        if let Some(parent) = dl.save_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dl.save_path, &bytes)?;
        let hash = rbx_core::image::hash_bytes(&bytes);

        let relative_icon = dl
            .save_path
            .strip_prefix(config_dir)
            .unwrap_or(&dl.save_path)
            .to_path_buf();

        // Persist the downloaded icon hash into the lockfile for this env.
        if let Some(env_lock) = new_lockfile.envs.get_mut(&dl.env) {
            if let Some(slot) = lock_icon_hash_mut(env_lock, dl.kind, &dl.name) {
                *slot = Some(hash.clone());
            }
        }

        // Persist the icon path wherever the resource currently lives: base
        // if env is default and the resource lives in base; otherwise the
        // env overlay wherever it lives, falling back to the base's file
        // when no overlay exists yet for this env.
        persist_icon_path(&mut files, dl.kind, &dl.env, &dl.name, &relative_icon);

        println!("  {} Saved to {}", "✓".green(), dl.save_path.display());
    }

    new_lockfile.save(&lockfile_path)?;
    // Written back through toml_edit, not reserialised: the user's comments,
    // key order, and any key rbx does not model all have to survive a pull.
    for file in &files {
        file.config.save_in_place(&file.path)?;
    }

    println!("{} Pull complete.", "✓".green());

    Ok(())
}

#[cfg(test)]
mod tests;
