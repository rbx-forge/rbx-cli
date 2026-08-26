//! `rbx rtbf`: declare which data store keys hold a user's data, so Roblox can
//! delete them when a right-to-be-forgotten request arrives.
//!
//! # Where the state lives
//!
//! `rbxrtbf.toml`, committed, reconciled against the `DataStoresConfig`
//! repository of the Open Cloud Configs API. Declarative like `rbx config`,
//! `rbx meta` and `rbx shop`: you write the desired state and this pushes it.
//!
//! There is **no lockfile**, for the reason `rbx config` has none: the published
//! config is readable in full, so the remote state is a fetch rather than
//! something that has to be remembered. `check` compares the file against what
//! Roblox is actually serving.
//!
//! # The templates are not per env
//!
//! `rbxrtbf.toml` declares the templates once, with no `[envs.*]` overlays,
//! because a key naming scheme is a property of your codebase rather than of an
//! environment: `PlayerInventory` is called that in dev and in prod. `--env`
//! selects which universe to publish to, and `--env all` or a group publishes
//! the same templates to several, which is the shape you want when the same
//! code runs in each.
//!
//! # Why a crate rather than `rbx config --repository DataStoresConfig`
//!
//! That invocation works and will keep working. What it cannot do is check the
//! templates, because its entry model holds an opaque value, and every mistake
//! in this file is one Roblox *accepts* and then silently never matches. See
//! [`model`] for the rules and why each one is invisible without a tool.

pub mod commands;
pub mod config;
pub mod model;
pub mod stores;

use anyhow::Result;
use clap::{Args, Subcommand};

use rbx_core::GlobalFlags;

/// How many days Roblox gives you to have honoured a request.
///
/// Printed by `check` rather than left in the docs: the deadline is the reason
/// a silently inert template matters, and it is not a number anyone remembers.
pub const DEADLINE_DAYS: u32 = 30;

#[derive(Args, Debug)]
pub struct RtbfCli {
    #[command(subcommand)]
    command: Command,

    /// Path to rbxrtbf.toml.
    #[arg(long, default_value = config::FILE)]
    pub config: std::path::PathBuf,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write a commented rbxrtbf.toml to start from
    Init,

    /// Show the declared templates and what they would match
    ///
    /// Local only, no network. Every template is validated and printed with the
    /// sample key Roblox would look for, which is the form you can compare
    /// against your Luau.
    Show {
        /// User id to substitute into the samples.
        #[arg(long, default_value_t = 1234567890)]
        user_id: u64,
    },

    /// Compare rbxrtbf.toml against the published templates
    ///
    /// Exits 2 when they differ, so a CI step can gate on the status alone;
    /// 1 if the check itself could not answer.
    Check,

    /// Publish rbxrtbf.toml as the canonical set of templates
    Sync {
        /// Publish message, recorded in the revision history.
        #[arg(long, short)]
        message: Option<String>,

        /// Publish without a message.
        #[arg(long)]
        no_message: bool,

        /// Show what would be published and stop.
        #[arg(long)]
        dry_run: bool,

        /// Skip the confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },

    /// Pull the published templates into rbxrtbf.toml
    Pull {
        /// Overwrite without confirmation.
        #[arg(long, short)]
        yes: bool,
    },

    /// Check every template against the data stores that actually exist
    ///
    /// The question `check` cannot answer. A template that names a store you do
    /// not have, or a key pattern nothing matches, is accepted by Roblox and
    /// deletes nothing: this is what finds that before a real request does.
    Verify {
        /// Also list the live stores no template covers.
        #[arg(long)]
        uncovered: bool,
    },
}

pub async fn run(cli: RtbfCli, global: &GlobalFlags) -> Result<()> {
    let ctx = commands::RtbfCtx {
        global,
        config: cli.config,
        base_url: cli.base_url,
    };

    match cli.command {
        Command::Init => commands::init::run(&ctx),
        Command::Show { user_id } => commands::show::run(&ctx, user_id),
        Command::Check => commands::check::run(&ctx).await,
        Command::Sync {
            message,
            no_message,
            dry_run,
            yes,
        } => commands::sync::run(&ctx, message.as_deref(), no_message, dry_run, yes).await,
        Command::Pull { yes } => commands::pull::run(&ctx, yes).await,
        Command::Verify { uncovered } => commands::verify::run(&ctx, uncovered).await,
    }
}
