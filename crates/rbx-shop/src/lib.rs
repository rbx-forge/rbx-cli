//! Declaratively manage Roblox game passes, badges, and developer products.

pub mod api;
pub mod codegen;
mod collision;
pub mod commands;
pub mod config;
mod ctx;
pub mod diff;
pub mod gifts;
pub mod json;
pub mod lockfile;
mod toml_write;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};

use rbx_core::GlobalFlags;

use crate::ctx::ShopCtx;

#[derive(Args, Debug)]
pub struct ShopCli {
    #[command(subcommand)]
    pub command: ShopCommands,

    /// Path to rbxshop.toml. Works before or after the subcommand.
    #[arg(long, global = true, default_value = "rbxshop.toml")]
    pub config: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum ShopCommands {
    /// Initialize a new rbxshop.toml config file.
    Init {
        /// Populate config from existing remote resources.
        #[arg(long)]
        from_remote: bool,

        /// Universe id (standalone mode; requires --from-remote).
        #[arg(long)]
        universe_id: Option<u64>,

        /// Detect pre-existing gift-twin developer products among the
        /// imported resources and mark `create_gift = true` on their source
        /// automatically (requires --from-remote). Value is the label prefix
        /// used to recognize them, e.g. "GIFT - ".
        #[arg(long, requires = "from_remote")]
        gift_label: Option<String>,

        /// Preview what would be imported without writing any files.
        #[arg(long, requires = "from_remote")]
        dry_run: bool,
    },

    /// Sync local config to Roblox.
    Sync {
        /// Show what would change without applying.
        #[arg(long)]
        dry_run: bool,

        /// Only sync specific resource types (comma-separated).
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<config::ResourceKind>>,

        /// Expected cost in Robux when creating a badge (default: 0).
        #[arg(long, default_value_t = 0)]
        badge_cost: u64,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// List remote resources (passes, badges, products).
    List {
        /// Resource type to list.
        resource: config::ResourceKind,

        /// Write the remote resources to stdout as one JSON document instead
        /// of a table.
        ///
        /// Every field Roblox returns, including the ids the table truncates
        /// nothing of but the description column crowds out. This says what
        /// Roblox has; `shop show --json` says what the repo declares, and
        /// neither says whether the two agree — that is `rbx check --json`.
        /// Field names are documented in docs/shop.md.
        #[arg(long)]
        json: bool,
    },

    /// Check config validity and diff against lockfile.
    ///
    /// Exits 2 when an env is out of sync, so a CI step can gate on the
    /// status alone; 1 if the check itself could not answer.
    Check,

    /// Regenerate the codegen folder from rbxshop.toml + rbxshop.lock.
    ///
    /// Offline — reads local files only, never contacts Roblox. `sync` already
    /// does this at the end of a successful run; use this to rebuild after a
    /// `git pull` without credentials, or with `--check` from a git hook or CD
    /// to prove the committed modules were not hand-edited.
    Codegen {
        /// Compare the folder against what would be generated instead of
        /// writing it. Exits 2 if anything differs.
        #[arg(long)]
        check: bool,
    },

    /// Pull remote state into lockfile.
    Pull {
        /// Show what remote state differs without writing anything.
        #[arg(long)]
        dry_run: bool,

        /// Keep remote icons (set local hash so next sync skips re-upload).
        #[arg(long, conflicts_with = "accept_local")]
        accept_remote: bool,

        /// Re-upload local icons on next sync.
        #[arg(long, conflicts_with = "accept_remote")]
        accept_local: bool,

        /// Skip confirmation prompt before overwriting local files.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Pretty-print the local rbxshop.toml (read-only, with defaults filled in).
    Show {
        /// Sort order within each section.
        #[arg(long, value_enum, default_value_t = ShowSort::Name)]
        sort: ShowSort,

        /// Merge all resource types into one list sorted globally (instead of
        /// grouping by section), with a type column.
        #[arg(long)]
        flat: bool,

        /// Write the declared resources to stdout as one JSON document
        /// instead of the tables.
        ///
        /// Rejected alongside --sort and --flat rather than overriding them:
        /// both are layout choices over a listing, and the document is an
        /// object keyed by TOML key, which has neither an order to pick nor a
        /// flat variant to ask for. Field names are documented in
        /// docs/shop.md.
        #[arg(long, conflicts_with_all = ["sort", "flat"])]
        json: bool,
    },

    /// Rename a resource key in config and lockfile (across all envs).
    Rename {
        /// Resource type.
        resource: config::ResourceKind,
        /// Current key name.
        old_key: String,
        /// New key name.
        new_key: String,
    },
}

/// `ResourceKind` as a command-line argument.
///
/// There used to be a second enum here, `ResourceType`, with the plural
/// variants the CLI spells and a `kind()` converting to
/// [`config::ResourceKind`] at the boundary. It existed only to carry
/// `#[derive(ValueEnum)]`, because putting that derive on the domain type
/// would pull `clap` into `config.rs` and everything that dispatches on it.
///
/// Implementing the trait by hand instead removes the duplicate without
/// paying that price: `ValueEnum` is a foreign trait on a local type, so the
/// impl is allowed here, in the one module that already knows about `clap`.
/// The domain modules are unchanged and still see a plain enum.
///
/// The strings are the argument spelling and are deliberately plural — they
/// read as a list on a command line, where `label()` is the singular word used
/// in prose about one resource. Keeping them here, next to the parser, is also
/// what makes it obvious that changing one changes the CLI.
impl ValueEnum for config::ResourceKind {
    fn value_variants<'a>() -> &'a [Self] {
        &config::ResourceKind::ALL
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(match self {
            config::ResourceKind::Pass => "passes",
            config::ResourceKind::Badge => "badges",
            config::ResourceKind::Product => "products",
        }))
    }
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum ShowSort {
    /// By display name (alphabetical, case-insensitive).
    Name,
    /// By price ascending; entries without a price sort last.
    Price,
    /// By TOML key.
    Key,
}

pub async fn run(cli: ShopCli, global: &GlobalFlags) -> Result<()> {
    let ctx = ShopCtx {
        config: cli.config,
        global,
        #[cfg(test)]
        base_url: None,
    };

    match cli.command {
        ShopCommands::Init {
            from_remote,
            universe_id,
            gift_label,
            dry_run,
        } => commands::init::run(&ctx, from_remote, universe_id, gift_label, dry_run).await,
        ShopCommands::Sync {
            dry_run,
            only,
            badge_cost,
            yes,
        } => commands::sync::run(&ctx, dry_run, only, badge_cost, yes).await,
        ShopCommands::List { resource, json } => commands::list::run(&ctx, resource, json).await,
        ShopCommands::Check => commands::check::run(&ctx).await,
        ShopCommands::Codegen { check } => commands::codegen::run(&ctx, check),
        ShopCommands::Pull {
            dry_run,
            accept_remote,
            accept_local,
            yes,
        } => commands::pull::run(&ctx, dry_run, accept_remote, accept_local, yes).await,
        ShopCommands::Show { sort, flat, json } => commands::show::run(&ctx, sort, flat, json),
        ShopCommands::Rename {
            resource,
            old_key,
            new_key,
        } => commands::rename::run(&ctx, resource, &old_key, &new_key),
    }
}
