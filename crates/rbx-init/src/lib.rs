//! Bootstrap Roblox resources from the CLI: groups, universes, places, and
//! their listing/renaming. All operations are cookie-based (Open Cloud does
//! not yet expose these endpoints).
//!
//! Cookie resolution lives on [`rbx_core::GlobalFlags`]; pass it through to
//! every command handler.

mod api;
mod commands;
/// Recording resources into `rbxplace.toml` without reformatting it.
///
/// Public so `rbx import` can lay an adopted universe down through the same
/// line-level writers `create-universe` uses, rather than growing a second
/// implementation with its own ideas about comments and key order.
pub mod record;

use std::path::PathBuf;

use anyhow::Result;

use clap::{Args, Subcommand};

use rbx_core::GlobalFlags;

#[derive(Args, Debug)]
pub struct InitCli {
    #[command(subcommand)]
    pub command: InitCommands,
}

#[derive(Subcommand, Debug)]
pub enum InitCommands {
    /// Create a new Roblox group. Cookie required.
    CreateGroup {
        /// Group name.
        #[arg(long)]
        name: String,

        /// Group description.
        #[arg(long, default_value = "")]
        description: String,

        /// Make the group publicly joinable (default: invite-only).
        #[arg(long)]
        public: bool,

        /// Path to a PNG icon to upload with the group (required by Roblox).
        #[arg(long)]
        icon: PathBuf,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Create a new universe with a root place. Cookie required.
    ///
    /// Owner is resolved in this priority order:
    ///   --group / --user (explicit) > `[owner]` in rbxplace.toml > your user account
    ///
    /// The new universe is recorded in rbxplace.toml as a new `[<env>]` block:
    /// pass the global `--env <name>` to name it outright, or answer the
    /// prompt. `--place <name>` overrides the default place key (`main`).
    /// Recording is skipped by `--no-record`, by `--yes`, when stdin is not a
    /// terminal, and when rbxplace.toml doesn't exist yet: this command
    /// extends an existing file, it does not create one.
    CreateUniverse {
        /// Group id to create the universe under. Mutually exclusive with --user.
        #[arg(long, conflicts_with = "user")]
        group: Option<u64>,

        /// User id to create the universe under. Mutually exclusive with --group.
        /// Omit both flags to fall back to `[owner]` in rbxplace.toml.
        #[arg(long)]
        user: Option<u64>,

        /// Template place id to clone from (defaults to Roblox's empty baseplate).
        #[arg(long)]
        template_place_id: Option<u64>,

        /// Rename the universe's root place to this name after creation.
        /// Roblox displays the root place name as the universe name.
        /// Prompted for when omitted (unless --yes or non-interactive).
        #[arg(long)]
        name: Option<String>,

        /// Don't record the new universe in rbxplace.toml.
        #[arg(long, conflicts_with_all = ["env", "place"])]
        no_record: bool,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Create a new place inside an existing universe. Cookie required.
    ///
    /// The new place is recorded under the env whose `universe_id` matches
    /// `--universe-id`; pass the global `--place <name>` to name the key, or
    /// answer the prompt. Same skip rules as `create-universe`.
    CreatePlace {
        /// Universe id to create the place in.
        #[arg(long = "universe-id")]
        universe_id: u64,

        /// Template place id to clone from (defaults to Roblox's empty baseplate).
        #[arg(long)]
        template_place_id: Option<u64>,

        /// Rename the new place to this name after creation.
        /// Prompted for when omitted (unless --yes or non-interactive).
        #[arg(long)]
        name: Option<String>,

        /// Don't record the new place in rbxplace.toml.
        #[arg(long, conflicts_with_all = ["env", "place"])]
        no_record: bool,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Rename a place by id. Cookie required.
    RenamePlace {
        /// Place id to rename.
        #[arg(long)]
        place: u64,

        /// New display name.
        #[arg(long)]
        name: String,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Rename a universe by id. Roblox stores the display name on the root
    /// place; this resolves the universe's root place and renames it.
    RenameUniverse {
        /// Universe id.
        #[arg(long = "universe-id")]
        universe_id: u64,

        /// New display name.
        #[arg(long)]
        name: String,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// List the authenticated user's groups. Cookie required.
    ListGroups,

    /// List the universes owned by a group, published or not. No credential needed.
    ///
    /// The listing is unfiltered for any caller: `accessFilter=2` is the
    /// public-only filter, and this sends `1`. A cookie changes no result.
    ListUniverses {
        /// Group id.
        #[arg(long)]
        group: u64,
    },

    /// List the places inside a universe. No credential needed.
    ///
    /// A private universe answers in full to an anonymous caller. What stays
    /// behind a session is a place's content, not its name.
    ListPlaces {
        /// Universe id.
        #[arg(long = "universe-id")]
        universe_id: u64,
    },
}

pub async fn run(cli: InitCli, global: &GlobalFlags) -> Result<()> {
    match cli.command {
        InitCommands::CreateGroup {
            name,
            description,
            public,
            icon,
            yes,
        } => commands::create_group::run(global, &name, &description, public, &icon, yes).await,
        InitCommands::CreateUniverse {
            group,
            user,
            template_place_id,
            name,
            no_record,
            yes,
        } => {
            commands::create_universe::run(
                global,
                group,
                user,
                template_place_id,
                name.as_deref(),
                no_record,
                yes,
            )
            .await
        }
        InitCommands::CreatePlace {
            universe_id,
            template_place_id,
            name,
            no_record,
            yes,
        } => {
            commands::create_place::run(
                global,
                universe_id,
                template_place_id,
                name.as_deref(),
                no_record,
                yes,
            )
            .await
        }
        InitCommands::RenamePlace { place, name, yes } => {
            commands::rename_place::run(global, place, &name, yes).await
        }
        InitCommands::RenameUniverse {
            universe_id,
            name,
            yes,
        } => commands::rename_universe::run(global, universe_id, &name, yes).await,
        InitCommands::ListGroups => commands::list_groups::run(global).await,
        InitCommands::ListUniverses { group } => commands::list_universes::run(global, group).await,
        InitCommands::ListPlaces { universe_id } => {
            commands::list_places::run(global, universe_id).await
        }
    }
}
