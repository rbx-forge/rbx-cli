//! Differential pull: writes minimal toml deltas given the base `[game]` /
//! `[media]` and the targeted env. Algorithm (per field):
//!   * if base is unset → write remote to base, remove any env override
//!   * if remote == base → remove any env override
//!   * else → write remote as env override
//!
//! Toml is rewritten with `toml_edit` so user comments are preserved.
//!
//! This module is the run itself: resolve the envs, ask once, fetch each, write
//! each. The five below are the steps it walks through, split out because they
//! are the parts that grow, and along the seams the file already marked with
//! its own section comments.

mod differential;
mod fetch;
mod media;
mod toml_edit_helpers;
mod write;
use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use crate::config::Config;
use crate::ctx::MetaCtx;
use crate::lockfile::{Lockfile, LOCKFILE_NAME, LOCKFILE_VERSION};
use rbx_core::confirm::confirm_always;

use self::{fetch::*, media::*, write::*};

pub async fn run(
    ctx: &MetaCtx<'_>,
    dry_run: bool,
    accept_remote: bool,
    accept_local: bool,
    yes: bool,
) -> Result<()> {
    let mut config = Config::load(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new(".")).to_path_buf();
    let lockfile_path = config_dir.join(LOCKFILE_NAME);
    let mut lockfile = Lockfile::load(&lockfile_path)?;

    let targets = ctx.resolve_targets(&config)?;
    // The header appears only when there is more than one env to separate, so
    // a single-env run prints exactly what it always has.
    let many = targets.len() > 1;

    // Read pass: every env's remote state, folded into one in-memory config.
    // Nothing local is written here, which is what lets the confirmation below
    // cover the whole run rather than being asked once per env.
    let mut pending: Vec<PendingPull> = Vec::new();
    for (env_name, universe_id, place_id) in &targets {
        if many {
            println!("\n{} {}", "env:".bold(), env_name.bold());
        }
        let (client, confirmed) = fetch_env(
            ctx,
            &mut config,
            &lockfile,
            env_name,
            *universe_id,
            *place_id,
        )
        .await?;
        // Taken here, not after the loop: see `PendingPull::resolved_game`.
        let (resolved_game, _) = config.resolve_env(Some(env_name));
        pending.push(PendingPull {
            env: env_name.clone(),
            universe_id: *universe_id,
            place_id: *place_id,
            client,
            confirmed,
            resolved_game,
        });
    }

    if !dry_run {
        // Confirm BEFORE any local file mutation. The pull rewrites
        // rbxmeta.toml + lockfile (and may overwrite media/) so this guards
        // against accidental clobbering of in-progress local edits. Asked once
        // for the whole run: a prompt inside the write loop is reached after
        // the earlier envs have already been overwritten, which is no longer a
        // question the user can answer no to.
        let envs: Vec<&str> = pending.iter().map(|p| p.env.as_str()).collect();
        confirm_always(&overwrite_prompt(&envs), yes)?;
    }

    for p in &pending {
        // No header here. The read pass already printed one per env, and a
        // second set makes a two-env transcript read as four envs visited.
        // What this pass prints (media lines, notes) falls under the env it
        // belongs to.

        // For dry-run, prepare/save nothing on disk. Otherwise we ensure the env
        // entry exists in the lockfile up front so download_media can persist after
        // each successful operation (crash-safe).
        let had_media_before = {
            let el = lockfile.env_view(&p.env);
            el.media.icon.is_some() || !el.media.thumbnails.is_empty()
        };
        if !dry_run {
            lockfile.version = LOCKFILE_VERSION;
            let el = lockfile.env_mut(&p.env);
            el.universe_id = p.universe_id;
            el.place_id = p.place_id;
            lockfile.save(&lockfile_path)?;
        }

        if accept_remote && !dry_run {
            download_media(
                &p.client,
                &mut config,
                &mut lockfile,
                &p.env,
                &config_dir,
                &lockfile_path,
            )
            .await?;
        } else if accept_local && !dry_run {
            println!(
                "\n{} clearing media hashes for env '{}' (next sync will re-upload).",
                "--accept-local:".yellow(),
                p.env
            );
            let el = lockfile.env_mut(&p.env);
            el.media.icon = None;
            el.media.thumbnails = Vec::new();
            lockfile.save(&lockfile_path)?;
        } else if !accept_remote && !accept_local && had_media_before {
            println!(
                "\n{} media not pulled. Use --accept-remote to download from Roblox or \
                 --accept-local to re-upload local on next sync.",
                "note:".dimmed()
            );
        }
    }

    if dry_run {
        println!("\n{}", "(dry-run: nothing written)".dimmed());
        return Ok(());
    }

    // -----------------------------------------------------------------------
    // Persist the resolved game state for every env in the lockfile.
    // -----------------------------------------------------------------------
    // Each entry from the snapshot that env's own read produced, never from the
    // config as it stands now. The reasoning, and the silent divergence the
    // other choice causes, are on `PendingPull::resolved_game`.
    for p in &pending {
        let mut new_game_lock = crate::diff::config_to_lock(&p.resolved_game);
        reconcile_lock(
            &mut new_game_lock,
            &lockfile.env_view(&p.env).game,
            &p.confirmed,
        );
        lockfile.env_mut(&p.env).game = new_game_lock;
    }
    lockfile.save(&lockfile_path)?;
    println!(
        "\n{} {}",
        "Updated".green().bold(),
        lockfile_path.display().to_string().cyan()
    );

    // -----------------------------------------------------------------------
    // Save config back to the toml (preserves comments via toml_edit)
    // -----------------------------------------------------------------------
    write_config_toml(&ctx.config, &config)?;
    println!(
        "{} {}",
        "Updated".green().bold(),
        ctx.config.display().to_string().cyan()
    );

    Ok(())
}

/// One env already read from Roblox, waiting for the local write-back.
struct PendingPull {
    env: String,
    universe_id: u64,
    place_id: u64,
    /// Kept rather than rebuilt: `--accept-remote` downloads media through the
    /// same client the read pass used, aimed at the same universe and place.
    client: RbxClient,
    confirmed: ConfirmedReads,
    /// This env's resolved `[game]`, captured the instant its own differential
    /// finished and **before any later env had a turn**.
    ///
    /// That instant is the only correct one, and getting it wrong is a silent
    /// permanent divergence rather than an error. The differential promotes a
    /// field to the base `[game]` the first time an env has a value for it, so
    /// reading the config back after the whole loop hands one env a value
    /// another env's remote supplied. Concretely: Roblox returns no
    /// `private_server` for `dev` (it has none) and 100 for `prod`, so prod's
    /// pass promotes 100 to base, and a post-loop read records 100 as dev's
    /// *confirmed remote state*. `check --env dev` then reports agreement and
    /// `sync --env dev` sends nothing, forever, over a universe that has no
    /// private servers. That is exactly the failure
    /// [`reconcile_lock`]'s doc forbids, reached through a door its field list
    /// cannot close: `private_server` is deliberately absent from
    /// `ConfirmedReads` because a failed read fails the command, which is true
    /// for one env and false across several.
    ///
    /// Captured per env, the entry is what a sequential `pull --env dev` then
    /// `pull --env prod` would have written, which is the behaviour a fan-out
    /// owes its user.
    resolved_game: crate::config::Game,
}

/// The question a pull asks once, before it overwrites anything local.
///
/// One env keeps the wording it has always had; several are named as a list,
/// because the single confirmation covers every one of them.
fn overwrite_prompt(envs: &[&str]) -> String {
    match envs {
        [env] => format!(
            "Overwrite local rbxmeta.toml and lockfile for env '{}'?",
            env
        ),
        _ => format!(
            "Overwrite local rbxmeta.toml and lockfile for env(s): [{}]?",
            envs.join(", ")
        ),
    }
}

/// Read one env's remote state and fold it into `config`.
///
/// Returns the client the later passes reuse, and what this env's reads
/// confirmed: `reconcile_lock` needs the confirmed values, and once the
/// differential apply has run the config no longer distinguishes what was read
#[cfg(test)]
mod prompt_tests {
    use super::*;

    /// One question for the whole run. The single-env rendering is the wording
    /// this command has always used, which is what keeps a one-env pull
    /// unchanged; several envs are named in one prompt rather than one prompt
    /// each, because the second would be asked after the first env's files had
    /// already been overwritten.
    #[test]
    fn the_overwrite_prompt_names_every_env_it_covers() {
        assert_eq!(
            overwrite_prompt(&["dev"]),
            "Overwrite local rbxmeta.toml and lockfile for env 'dev'?"
        );
        assert_eq!(
            overwrite_prompt(&["dev", "staging"]),
            "Overwrite local rbxmeta.toml and lockfile for env(s): [dev, staging]?"
        );
    }
}
