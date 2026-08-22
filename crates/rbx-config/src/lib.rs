//! Manage Roblox in-experience live configs via the Open Cloud Configs API.

pub mod api;
mod commands;
pub mod config;
mod ctx;
pub mod diff;
pub mod json;
pub mod lock;
mod places_config;
pub mod value;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};

use rbx_core::GlobalFlags;

use crate::ctx::ConfigCtx;

#[derive(Args, Debug)]
pub struct ConfigCli {
    #[command(subcommand)]
    pub command: ConfigCommands,

    /// Path to rbxconfig.toml.
    #[arg(long, default_value = "rbxconfig.toml")]
    pub config: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Initialize a new rbxconfig.toml with a commented template.
    Init,

    /// Print the live published config (or a single key).
    Get {
        /// Key to retrieve (prints full config if omitted).
        key: Option<String>,

        /// Write the answer to stdout as one JSON document instead of the
        /// bare value.
        ///
        /// `.value` holds the answer when a key was named; without one,
        /// `.entries` carries the whole published config. This says what
        /// Roblox is serving, not whether rbxconfig.toml agrees with it:
        /// that is `rbx config check`. Field names are documented in
        /// docs/config.md.
        #[arg(long)]
        json: bool,
    },

    /// List all published config keys with types and value previews.
    List {
        /// Write the published config to stdout as one JSON document instead
        /// of the listing.
        ///
        /// The same document `get --json` emits without a key: same envelope,
        /// same `.entries`, so one filter reads both. Field names are
        /// documented in docs/config.md.
        #[arg(long)]
        json: bool,
    },

    /// Show the diff between local rbxconfig.toml and live published config.
    ///
    /// Exits 2 when local and live differ, so a CI step can gate on the
    /// status alone; 1 if the check itself could not answer.
    Check,

    /// Push rbxconfig.toml as the canonical state (overwrite draft + publish).
    ///
    /// Keys present in rbxconfig.toml are set; keys absent are removed from live.
    Sync {
        /// Publish message.
        #[arg(long, short)]
        message: Option<String>,

        /// Publish without a message.
        #[arg(long)]
        no_message: bool,

        /// Deployment strategy.
        #[arg(long, short, default_value = "immediate")]
        strategy: Strategy,

        /// Show diff without publishing.
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },

    /// Pull the live config into rbxconfig.toml (live is authoritative).
    Pull {
        /// Overwrite without confirmation.
        #[arg(long, short)]
        yes: bool,
    },

    /// List revision history.
    Versions {
        /// Number of revisions to show.
        #[arg(long, short, default_value = "20")]
        count: usize,

        /// Write the revisions to stdout as one JSON document instead of the
        /// listing.
        ///
        /// Carries the keys each revision changed, which the listing only
        /// counts, and says in the document whether `--count` was reached.
        /// Field names are documented in docs/config.md.
        #[arg(long)]
        json: bool,
    },

    /// Roll back to a previous revision.
    Rollback {
        /// Revision id to roll back to (shows picker if omitted).
        revision_id: Option<String>,

        /// Number of revisions to show in picker.
        #[arg(long, default_value = "10")]
        count: usize,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Strategy {
    /// Propagates in ~5 minutes.
    Immediate,
    /// Rolls out gradually over ~15 minutes.
    GradualRollout,
}

impl Strategy {
    pub fn as_api_str(&self) -> &'static str {
        match self {
            Strategy::Immediate => "Immediate",
            Strategy::GradualRollout => "GradualRollout",
        }
    }

    pub fn eta_minutes(&self) -> u32 {
        match self {
            Strategy::Immediate => 5,
            Strategy::GradualRollout => 15,
        }
    }
}

pub async fn run(cli: ConfigCli, global: &GlobalFlags) -> Result<()> {
    let ctx = ConfigCtx {
        config: cli.config,
        places: global.places.clone(),
        api_key: global.api_key.clone(),
        env: global.env.clone(),
        // The global `--universe-id`, not a local copy. This crate used to
        // declare its own on `ConfigCli`, which shadowed the global one at the
        // parent and stopped it propagating into the subcommands, so the flag
        // worked before the subcommand name and was rejected after it.
        universe_id: global.universe_id,
        #[cfg(test)]
        base_url: None,
    };

    match cli.command {
        ConfigCommands::Init => commands::init::run(&ctx),
        ConfigCommands::Get { key, json } => commands::get::run(&ctx, key.as_deref(), json).await,
        ConfigCommands::List { json } => commands::list::run(&ctx, json).await,
        ConfigCommands::Check => commands::check::run(&ctx).await,
        ConfigCommands::Sync {
            message,
            no_message,
            strategy,
            dry_run,
            yes,
        } => {
            commands::sync::run(
                &ctx,
                message.as_deref(),
                no_message,
                &strategy,
                dry_run,
                yes,
            )
            .await
        }
        ConfigCommands::Pull { yes } => commands::pull::run(&ctx, yes).await,
        ConfigCommands::Versions { count, json } => {
            commands::versions::run(&ctx, count, json).await
        }
        ConfigCommands::Rollback { revision_id, count } => {
            commands::rollback::run(&ctx, revision_id, count).await
        }
    }
}
