//! Declaratively manage Roblox game/universe metadata: name, description,
//! icon, thumbnails, devices, social links, private servers, server fill mode,
//! copying permission, visibility, beta mode.

mod api;
mod commands;
pub mod config;
mod ctx;
pub mod diff;
pub mod engine_echo;
pub mod live;
pub mod lockfile;
mod refresh;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

use rbx_core::GlobalFlags;

use crate::ctx::MetaCtx;

#[derive(Args, Debug)]
pub struct MetaCli {
    #[command(subcommand)]
    pub command: MetaCommands,

    /// Path to rbxmeta.toml.
    #[arg(long, default_value = "rbxmeta.toml")]
    pub config: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum MetaCommands {
    /// Initialize a new rbxmeta.toml config file.
    Init {
        /// Populate config from existing remote universe/place state.
        #[arg(long)]
        from_remote: bool,

        /// Universe id (required with --from-remote in standalone mode).
        #[arg(long)]
        universe_id: Option<u64>,

        /// Root place id (required with --from-remote in standalone mode).
        #[arg(long)]
        place_id: Option<u64>,
    },

    /// Sync local config to Roblox.
    ///
    /// Reads Roblox first, so the plan is the config against what Roblox
    /// holds now, a dry run included.
    Sync {
        /// Show what would change without applying.
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,

        /// Replace what was changed on Roblox since the last sync or pull.
        ///
        /// A value the lockfile recorded and Roblox no longer holds was
        /// changed there, on the Creator Hub or from another checkout. A plan
        /// that would write over it is refused: `rbx meta pull` brings those
        /// values in, and this flag replaces them with the config's.
        #[arg(long)]
        overwrite_newer: bool,
    },

    /// Check config validity and diff against lockfile.
    ///
    /// Exits 2 when the config no longer matches the lockfile, so a CI step
    /// can gate on the status alone; 1 if the check itself could not answer.
    Check,

    /// Pull remote state into the config and lockfile.
    Pull {
        /// Show what differs without writing anything.
        #[arg(long)]
        dry_run: bool,

        /// Download remote icon and thumbnails into `media.dir` and update config paths.
        #[arg(long, conflicts_with = "accept_local")]
        accept_remote: bool,

        /// Clear media hashes (next sync re-uploads local icon and thumbnails).
        #[arg(long, conflicts_with = "accept_remote")]
        accept_local: bool,

        /// Skip confirmation prompt before overwriting local files.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
}

pub async fn run(cli: MetaCli, global: &GlobalFlags) -> Result<()> {
    let ctx = MetaCtx {
        config: cli.config,
        global,
    };

    match cli.command {
        MetaCommands::Init {
            from_remote,
            universe_id,
            place_id,
        } => commands::init::run(&ctx, from_remote, universe_id, place_id).await,
        MetaCommands::Sync {
            dry_run,
            yes,
            overwrite_newer,
        } => {
            commands::sync::run(
                &ctx,
                commands::sync::Options {
                    dry_run,
                    yes,
                    overwrite_newer,
                },
            )
            .await
        }
        MetaCommands::Check => commands::check::run(&ctx).await,
        MetaCommands::Pull {
            dry_run,
            accept_remote,
            accept_local,
            yes,
        } => commands::pull::run(&ctx, dry_run, accept_remote, accept_local, yes).await,
    }
}
