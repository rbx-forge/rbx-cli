//! Declaratively manage Roblox game/universe metadata: name, description,
//! icon, thumbnails, devices, social links, private servers, server fill mode,
//! copying permission, visibility, beta mode.

mod api;
mod commands;
pub mod config;
mod ctx;
pub mod diff;
pub mod engine_echo;
pub mod lockfile;

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
    Sync {
        /// Show what would change without applying.
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
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
        MetaCommands::Sync { dry_run, yes } => commands::sync::run(&ctx, dry_run, yes).await,
        MetaCommands::Check => commands::check::run(&ctx).await,
        MetaCommands::Pull {
            dry_run,
            accept_remote,
            accept_local,
            yes,
        } => commands::pull::run(&ctx, dry_run, accept_remote, accept_local, yes).await,
    }
}
