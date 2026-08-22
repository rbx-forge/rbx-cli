//! Read `rbxplace.toml`: the shared env map every other subcommand resolves
//! `--env` against. This crate never talks to Roblox: it only reports what the
//! file says, using the exact same resolution rules as `rbx_core::places`, so
//! `rbx env get place-id --env prod` always prints the id that
//! `rbx place upload --env prod` would target.
//!
//! Four verbs: `list`/`get` mirror `rbx config`:
//! - `list`: human-readable dump of the whole file (or one env via `--env`),
//!   plus the two bare listings `--names` and `--place-names` that scripts and
//!   the generated shell completions read.
//! - `get`: one bare value on stdout, for `$(...)` capture in scripts.
//! - `gen-module`: the same data as a Luau/Lua/JSON/TS module for game code.
//! - `rm`: take an env out of every file that mentions it.
//!
//! `rm` is the only one that writes, and it writes only local files: this
//! crate still never opens a connection. It lives here rather than in
//! `rbx-shop` or `rbx-meta` because an env is not owned by either: it is
//! declared in `rbxplace.toml` and referenced from both, so removing one is a
//! question about this file's contents that happens to reach into its
//! neighbours. See `commands::rm` for why it is not called `destroy`.

pub mod commands;
pub mod json;

use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};

use rbx_core::GlobalFlags;

/// Re-exported so `rbx check` can run the same comparison `gen-module --check`
/// runs, without going through the command, which prints as it goes, and
/// stdout belongs to the document under `--json`. Visibility only: neither
/// function's body changed.
pub use commands::gen_module::{
    render as render_env_module, resolve_out as resolve_env_module_out,
};
pub use commands::get::resolve_field;

#[derive(Args, Debug)]
pub struct EnvCli {
    #[command(subcommand)]
    pub command: EnvCommands,
}

#[derive(Subcommand, Debug)]
pub enum EnvCommands {
    /// Show the envs defined in rbxplace.toml.
    ///
    /// Prints every env by default; pass the global `--env <name>` to show
    /// just one. Output mirrors the file's own TOML shape so it maps back
    /// onto what you'd edit.
    List {
        /// Print env names only, one per line: no colors, no decoration.
        /// Useful for scripts and shell-completion helpers.
        #[arg(long)]
        names: bool,

        /// Print place names only, one per line: the `--place` counterpart
        /// of `--names`.
        ///
        /// Without `--env`, the sorted union of every env's place names, so
        /// the answer exists before an env has been chosen. With `--env
        /// <name>`, only that env's places.
        ///
        /// Spelled `--place-names` and not `--places`, which is already the
        /// global flag naming the file to read.
        #[arg(long, conflicts_with = "names")]
        place_names: bool,

        /// Write the envs to stdout as one JSON document instead of the
        /// listing.
        ///
        /// Rejected alongside --names rather than overriding it: the JSON
        /// document already carries every name, so asking for both is a
        /// mistake worth reporting. Field names are documented in docs/env.md.
        #[arg(long, conflicts_with_all = ["names", "place_names"])]
        json: bool,
    },

    /// Print a single resolved value from rbxplace.toml.
    ///
    /// The value goes to stdout bare (no label, no color) so it can be
    /// captured directly:
    ///
    ///   UNIVERSE=$(rbx env get universe-id --env prod)
    ///
    /// With `--env all`, one `<env><TAB><value>` line is printed per env.
    Get {
        /// Field to print.
        #[arg(value_enum)]
        field: Field,

        /// Write the answer to stdout as one JSON document instead of the bare
        /// value.
        ///
        /// `.value` holds the answer for a single env; `--env all` fills
        /// `.results` instead. Field names are documented in docs/env.md.
        #[arg(long)]
        json: bool,
    },

    /// Generate a module of env ids for your game code to import.
    ///
    /// The output format follows the extension: `.luau` (typed), `.lua`,
    /// `.json`, or `.ts`. Envs and places are emitted in name order, so
    /// regenerating an unchanged rbxplace.toml produces an identical file.
    GenModule {
        /// Output file path (`.lua`, `.luau`, `.json`, or `.ts`).
        ///
        /// Optional when `[codegen].output` is set in rbxplace.toml, which is
        /// the form to prefer in a hook or a CI job, so the generator and the
        /// checker cannot be pointed at different files.
        #[arg(long, short)]
        out: Option<String>,

        /// Compare the existing file against what would be generated instead
        /// of writing it. Exits 2 if they differ, so a git hook or CI job can
        /// prove the committed module was never hand-edited.
        #[arg(long)]
        check: bool,
    },

    /// Remove an env from rbxplace.toml and every file keyed by it.
    ///
    /// Local only. Nothing is deleted on Roblox, and nothing could be: a game
    /// pass or a developer product cannot be deleted there at all, only taken
    /// off sale, and a badge can only be disabled. This removes the env
    /// itself: its block in rbxplace.toml, its overlay in rbxmeta.toml and
    /// rbxshop.toml, its section in both lockfiles, and the per-env module
    /// `rbx shop codegen` wrote for it.
    ///
    /// Prints what it will touch and asks before writing. Comments and key
    /// order in the files it edits are preserved.
    Rm {
        /// Env to remove, as it is named in rbxplace.toml.
        ///
        /// Positional rather than read from the global `--env`: this is the
        /// one command where naming the wrong env deletes something, and
        /// `--env` is a flag people leave set in a shell for a whole session.
        name: String,

        /// List what would be removed without writing anything.
        #[arg(long)]
        dry_run: bool,

        /// Skip the confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
}

/// A readable field of `rbxplace.toml`. Names are kebab-case to match CLI
/// convention, with the file's own snake_case keys accepted as aliases so
/// people can type what they see in the TOML.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Field {
    /// `universe_id` of the target env. Requires `--env`.
    #[value(alias = "universe_id")]
    UniverseId,

    /// Place id from `[<env>.places]`. Requires `--env`; honors `--place`,
    /// and otherwise defaults to `main` (or the only entry).
    #[value(alias = "place_id")]
    PlaceId,

    /// Owner id: per-env `[<env>.owner]` if set, else top-level `[owner]`.
    #[value(alias = "owner_id")]
    OwnerId,

    /// Owner type (`user` or `group`), resolved like `owner-id`.
    #[value(alias = "owner_type")]
    OwnerType,
}

impl Field {
    /// The canonical CLI spelling, for error messages that suggest a fix.
    pub fn as_str(self) -> &'static str {
        match self {
            Field::UniverseId => "universe-id",
            Field::PlaceId => "place-id",
            Field::OwnerId => "owner-id",
            Field::OwnerType => "owner-type",
        }
    }

    /// Owner fields resolve from the top-level `[owner]` block, so they can
    /// answer without an env; the id fields cannot.
    pub fn needs_env(self) -> bool {
        matches!(self, Field::UniverseId | Field::PlaceId)
    }
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub async fn run(cli: EnvCli, global: &GlobalFlags) -> Result<()> {
    match cli.command {
        EnvCommands::List {
            names,
            place_names,
            json,
        } => commands::list::run(global, commands::list::Mode::new(names, place_names, json)),
        EnvCommands::Get { field, json } => commands::get::run(global, field, json),
        EnvCommands::GenModule { out, check } => {
            commands::gen_module::run(&global.places, out.as_deref(), check)
        }
        EnvCommands::Rm { name, dry_run, yes } => {
            commands::rm::run(&global.places, &name, dry_run, yes)
        }
    }
}
