//! Manage Roblox in-experience live configs via the Open Cloud Configs API.

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

use rbx_core::api::Repository;
use rbx_core::GlobalFlags;

use crate::ctx::ConfigCtx;

#[derive(Args, Debug)]
pub struct ConfigCli {
    #[command(subcommand)]
    pub command: ConfigCommands,

    /// Path to rbxconfig.toml.
    #[arg(long, default_value = "rbxconfig.toml")]
    pub config: PathBuf,

    /// Configs repository to address.
    ///
    /// Defaults to `InExperienceConfig`, the live config `ConfigService`
    /// reads, which is the only repository this command addressed before the
    /// flag existed. `rbxconfig.toml` may name one in its `repository` field,
    /// and it is the source of truth for the commands that read it: a flag
    /// naming a different one is refused rather than allowed to win, because
    /// a publish into the wrong repository replaces a live config wholesale
    /// and cannot be undone from here.
    ///
    /// Case-insensitive. An unknown name lists the eight the API exposes.
    #[arg(long)]
    pub repository: Option<String>,
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
        ///
        /// Required off a terminal, where there is nobody to show the picker
        /// to. `rbx config versions` lists the ids.
        revision_id: Option<String>,

        /// Number of revisions to show in picker.
        #[arg(long, default_value = "10")]
        count: usize,

        /// Publish message for the version the rollback creates.
        #[arg(long, short)]
        message: Option<String>,

        /// Publish without a message.
        #[arg(long)]
        no_message: bool,

        /// Skip confirmation prompt.
        ///
        /// Answers the publish message too, exactly as on `sync`: a flag that
        /// skipped the confirmation and then stopped on a second question
        /// would fail a pipeline on a message about a terminal.
        #[arg(long, short)]
        yes: bool,
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

/// The `--repository` flag, parsed.
///
/// Parsed here rather than by a clap `value_parser` because
/// `Repository::from_str` answers with an `anyhow::Error` carrying the list of
/// available names, and clap's parser wants a `std::error::Error`, which
/// `anyhow::Error` is not. It runs before any request, so a typo costs one
/// error and no HTTP.
fn parse_repository(flag: Option<&str>) -> Result<Option<Repository>> {
    flag.map(|name| name.parse::<Repository>()).transpose()
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
        repository: parse_repository(cli.repository.as_deref())?,
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
        ConfigCommands::Rollback {
            revision_id,
            count,
            message,
            no_message,
            yes,
        } => {
            commands::rollback::run(
                &ctx,
                revision_id,
                count,
                message.as_deref(),
                no_message,
                yes,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// `ConfigCli` is an `Args`, so it reaches clap through the binary's
    /// parser. This is that parser, minus everything else the binary has.
    #[derive(Parser, Debug)]
    struct Harness {
        #[command(flatten)]
        cli: ConfigCli,
    }

    fn parse(args: &[&str]) -> ConfigCli {
        Harness::try_parse_from(args).expect("parse").cli
    }

    /// The default is the absence of a flag, not a spelling of the default:
    /// `resolve_repository` has to be able to tell the two apart.
    #[test]
    fn without_the_flag_no_repository_is_named() {
        let cli = parse(&["rbx-config", "init"]);

        assert_eq!(cli.repository, None);
        assert_eq!(parse_repository(cli.repository.as_deref()).unwrap(), None);
    }

    #[test]
    fn the_flag_is_parsed_case_insensitively_into_the_canonical_spelling() {
        let cli = parse(&["rbx-config", "--repository", "datastoresconfig", "init"]);

        assert_eq!(
            parse_repository(cli.repository.as_deref()).unwrap(),
            Some(Repository::DataStoresConfig)
        );
    }

    /// A name that is not one of the eight is only actionable next to the
    /// eight, and it costs nothing to say so: the parse runs before any
    /// request.
    #[test]
    fn an_unknown_repository_lists_the_eight() {
        let cli = parse(&["rbx-config", "--repository", "InExperience", "init"]);

        let err = parse_repository(cli.repository.as_deref())
            .expect_err("'InExperience' is not a repository")
            .to_string();
        for repository in Repository::ALL {
            assert!(err.contains(repository.as_str()), "{err}");
        }
        assert_eq!(Repository::ALL.len(), 8);
    }
}
