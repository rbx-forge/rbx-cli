//! The command tree of `rbx data`, and nothing else.
//!
//! Split out of `lib.rs` because clap declarations and the code that acts on
//! them grow for different reasons. Most of what follows is documentation: the
//! long-form help is where the safety model of each subcommand is written down,
//! and it belongs next to the flags it describes rather than interleaved with
//! the dispatch that reads them.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::{backup, ordered, DEFAULT_SCOPE};

/// The flags every subcommand shares, and the subcommand itself.
///
/// The fields are `pub(crate)` rather than private because the dispatch that
/// reads them lives in `lib.rs` now. They stay crate-visible and no wider: a
/// caller outside this crate hands the whole `DataCli` to [`crate::run`] and
/// has no business reaching into it.
#[derive(Args, Debug)]
pub struct DataCli {
    #[command(subcommand)]
    pub(crate) command: Command,

    /// Data store name, as the game passes to `GetDataStore`.
    #[arg(long, global = true)]
    pub(crate) datastore: Option<String>,

    /// Data store scope.
    #[arg(long, global = true, default_value = DEFAULT_SCOPE)]
    pub(crate) scope: String,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    pub(crate) base_url: Option<String>,
}

impl DataCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Print one entry
    Get {
        /// Entry key, e.g. `Player_156`.
        entry: String,

        /// Write the value to a file instead of printing it.
        #[arg(long)]
        out: Option<PathBuf>,

        /// Write the result to stdout as one JSON document.
        ///
        /// The stored value nested under `value`, plus the revision and
        /// whether the entry is soft-deleted. Nothing else about the player:
        /// the entry's user association and attributes stay out, as they do
        /// in the human form. stdout carries the document and nothing else;
        /// diagnostics stay on stderr. Field names are documented in
        /// docs/ops/data.md.
        #[arg(long)]
        json: bool,
    },

    /// Put a player back to a fresh profile
    ///
    /// The same write as `set`, named for what it is used for. Deleting is the
    /// obvious way to reset somebody and the wrong one: it leaves the game
    /// reading nothing, which behaves however that game's own wrapper decides,
    /// and it erases nothing anyway since the old value stays readable through
    /// its revision. Writing the template you ship with is immediate and
    /// unambiguous.
    Reset {
        /// Entry key.
        entry: String,

        /// The fresh profile to write. Defaults to `playerdata.template.json`
        /// in the working directory.
        #[arg(long)]
        template: Option<PathBuf>,

        /// Write the current value here first. Defaults to a timestamped file
        /// in `.rbx/backups/<env>/`, beside `rbxplace.toml`.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable (after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days) or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// What happened rather than what it looks like: the action, whether
        /// it was applied or was a dry run, whether the entry existed, the
        /// revision it is at now, and where the backup went. stdout carries
        /// the document and nothing else. Field names are documented in
        /// docs/ops/data.md.
        ///
        /// Requires `--yes`. `OutputFormat::Json` refuses to prompt and every
        /// write here asks through `confirm_always`, so the pair would either
        /// draw a prompt into a pipe or quietly skip a confirmation. Clap
        /// refuses the combination instead, which keeps that guarantee
        /// structural rather than moving it into a check at run time.
        #[arg(long, requires = "yes")]
        json: bool,
    },

    /// Force the next write to every key to keep a backup
    ///
    /// The answer to the warning the rest of this crate carries. Normally an
    /// overwrite is unrecoverable: Roblox keeps the current revision and drops
    /// the one it replaced. After a snapshot, the *next* write to every key in
    /// the experience creates a versioned backup of the previous value first,
    /// and that backup is guaranteed readable for 30 days. So the state as of
    /// the snapshot survives one overwrite per key.
    ///
    /// Run it before a migration or a bulk edit, not on a schedule: Roblox
    /// allows one per experience per UTC day. A second call the same day is a
    /// no-op that reports the standing snapshot's time rather than failing.
    ///
    /// This is why it needs `--apply` despite only ever *adding*
    /// recoverability. Spending the day's snapshot early is not free: a
    /// snapshot at 09:00 protects the values as of 09:00, and a key written at
    /// 10:00 and again at 17:00 keeps the 09:00 value, not the 10:00 one. When
    /// you want the state right before a risky write, you want the snapshot
    /// right before it too.
    ///
    /// Experience-wide, so it needs neither `--datastore` nor `--scope`.
    /// Needs `universe-datastores.control:snapshot`.
    Snapshot {
        /// Actually take it.
        #[arg(long)]
        apply: bool,
    },

    /// Remove an entry, the way `RemoveAsync` does
    ///
    /// The counterpart to `reset`, and the gentler of the two despite the
    /// name. A normal read then answers 404, so a game that builds a fresh
    /// profile when it finds nothing builds one, from its own template rather
    /// than from a copy of it that has to be kept in step.
    ///
    /// And the value survives: Roblox soft-deletes, so the entry stays in a
    /// listing with `--show-deleted` and its last value stays readable through
    /// `data revisions` for thirty days. `set` and `reset` destroy it at once.
    ///
    /// The local copy is still written first, because thirty days is a window
    /// and a file is not.
    ///
    /// One ordering matters: a live session that holds this profile in memory
    /// writes it back when it ends, undoing this. Delete while nobody is in
    /// the experience, or end the session first from inside the game.
    ///
    /// Named `delete-key` beside `delete-store`, which removes the whole store
    /// this entry lives in. The level is in the name on both, so neither reads
    /// as the default: `delete profile_123` and `delete-store PlayerData` would
    /// be one glance apart in a shell history.
    ///
    /// The bare `delete` was kept as an alias when the suffix was introduced,
    /// and dropped in 0.9.0. It was the exact spelling the rename existed to
    /// remove, held open for callers who did not exist yet, on the one
    /// subcommand here where reading the wrong level destroys the wrong thing.
    #[command(name = "delete-key")]
    DeleteKey {
        /// Entry key.
        entry: String,

        /// Write the current value here first. Defaults to a timestamped file
        /// in `.rbx/backups/<env>/`, beside `rbxplace.toml`.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Actually remove it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// What happened rather than what it looks like: the action, whether
        /// it was applied or was a dry run, whether the entry existed, the
        /// revision it is at now, and where the backup went. stdout carries
        /// the document and nothing else. Field names are documented in
        /// docs/ops/data.md.
        ///
        /// Requires `--yes`. `OutputFormat::Json` refuses to prompt and every
        /// write here asks through `confirm_always`, so the pair would either
        /// draw a prompt into a pipe or quietly skip a confirmation. Clap
        /// refuses the combination instead, which keeps that guarantee
        /// structural rather than moving it into a check at run time.
        #[arg(long, requires = "yes")]
        json: bool,
    },

    /// List the data stores in the experience
    ///
    /// The one subcommand you can run without knowing a store name, which is
    /// what makes it the entry point to every other one: its output is what
    /// `--datastore` takes.
    ///
    /// Expect stores nobody wrote by hand. A game running in Studio writes to
    /// whatever name its own wrapper builds, so a `-studio` twin of the live
    /// store is normal, and a wrapper library keeps its bookkeeping in a store
    /// of its own next to the data it manages.
    ///
    /// A store exists from its first write, not from the first `GetDataStore`,
    /// so a name absent here is a store the game has never written to.
    ///
    /// Experience-wide, so it needs neither `--datastore` nor `--scope`.
    /// Needs `universe-datastores.control:list`.
    Stores {
        /// Include stores that have been deleted but not yet purged.
        #[arg(long)]
        show_deleted: bool,

        /// Maximum stores to fetch.
        #[arg(long, default_value_t = 100)]
        limit: u32,

        /// Write the stores to stdout as one JSON document.
        ///
        /// The names, plus what the human form prints around them: the count,
        /// and whether the run stopped at --limit rather than at the end of
        /// the experience. stdout carries the document and nothing else. Field
        /// names are documented in docs/ops/data.md.
        #[arg(long)]
        json: bool,
    },

    /// Remove a whole data store, with every entry in it
    ///
    /// The counterpart of `delete-key` one level up, and the answer to an
    /// asymmetry: a store comes into existence from the first write to a name
    /// nobody created, so this tool could make one by accident and could not
    /// remove one at all. Until now the only way back was the Creator Hub.
    ///
    /// Soft, like `delete-key`: the store stays in `data stores
    /// --show-deleted` and `restore-store` brings it back. How long that lasts
    /// is not documented by Roblox and is not claimed here.
    ///
    /// Asks for the store's name typed back rather than for a `y`. The mistake
    /// worth catching is not running this, it is running it on the wrong store,
    /// and a name that arrived from a shell history answers `y` just as
    /// readily as one that was read off `data stores`.
    ///
    /// Needs `universe-datastores.control:delete`.
    DeleteStore {
        /// Store name, from `data stores`.
        ///
        /// Positional rather than taken from `--datastore`, though that flag
        /// names a store too. `--datastore` is the store the other subcommands
        /// happen to be pointed at, often from a config file or a shell alias;
        /// a store being destroyed should be named in the command that destroys
        /// it.
        store: String,

        /// Actually remove it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// Requires `--yes`, the rule every write here follows: `--json`
        /// refuses to prompt, so the pair would either draw a prompt into a
        /// pipe or quietly skip a confirmation.
        #[arg(long, requires = "yes")]
        json: bool,
    },

    /// Bring back a store removed with `delete-store`
    ///
    /// The undo, and the reason `delete-store` can be soft about it. Works
    /// while Roblox still holds the store, which `data stores --show-deleted`
    /// is how you check: a store absent from that listing is past whatever
    /// window Roblox keeps, and this cannot reach it.
    ///
    /// Needs `universe-datastores.control:delete`, the same scope as
    /// `delete-store`. Roblox files `:undelete` under the delete scope rather
    /// than giving it one of its own, so a key that can remove a store can
    /// always put it back and there is no narrower grant for only the undo.
    RestoreStore {
        /// Store name, from `data stores --show-deleted`.
        store: String,

        /// Actually restore it.
        #[arg(long)]
        apply: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// No `--yes` here, unlike its sibling: restoring a store takes
        /// nothing away, so there is no confirmation for `--json` to collide
        /// with.
        #[arg(long)]
        json: bool,
    },

    /// List entry keys
    List {
        /// Only keys starting with this.
        #[arg(long)]
        prefix: Option<String>,

        /// Include entries that have been deleted but not yet purged.
        #[arg(long)]
        show_deleted: bool,

        /// Maximum keys to fetch.
        #[arg(long, default_value_t = 100)]
        limit: u32,

        /// Write the keys to stdout as one JSON document.
        ///
        /// The keys, plus what the human form prints around them: the filter
        /// in force, the count, and whether the run stopped at --limit rather
        /// than at the end of the store. stdout carries the document and
        /// nothing else. Field names are documented in docs/ops/data.md.
        #[arg(long)]
        json: bool,
    },

    /// List the revisions of one entry
    ///
    /// Expect fewer than you wrote. Roblox keeps the current revision of a
    /// live entry and discards the ones an overwrite replaced, so this is
    /// mostly useful after a delete, where the value from before the delete
    /// survives. To undo an overwrite, use the backup file the write left
    /// behind.
    ///
    /// Needs `universe-datastores.versions:list`, and reading one needs
    /// `versions:read`, both separate from a plain read.
    Revisions {
        /// Entry key.
        entry: String,

        /// Print this revision's value instead of the list.
        #[arg(long)]
        revision: Option<String>,

        /// Write the result to stdout as one JSON document.
        ///
        /// Two documents, and --revision is what picks between them: without
        /// it, the revision list; with it, that revision's value. Field names
        /// are documented in docs/ops/data.md.
        #[arg(long)]
        json: bool,
    },

    /// Put a past revision back as the current value
    ///
    /// Works when there is a past revision to put back, which after a delete
    /// there is. After an overwrite there usually is not: see `data revisions`.
    ///
    /// Named `restore-key` beside `restore-store`, for the reason `delete-key`
    /// is, and the bare `restore` went the same way in 0.9.0.
    #[command(name = "restore-key")]
    RestoreKey {
        /// Entry key.
        entry: String,

        /// Revision id, from `data revisions`.
        #[arg(long)]
        revision: String,

        /// Write the current value here first. Defaults to a timestamped file
        /// in `.rbx/backups/<env>/`, beside `rbxplace.toml`.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable (after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days) or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// What happened rather than what it looks like: the action, whether
        /// it was applied or was a dry run, whether the entry existed, the
        /// revision it is at now, and where the backup went. stdout carries
        /// the document and nothing else. Field names are documented in
        /// docs/ops/data.md.
        ///
        /// Requires `--yes`. `OutputFormat::Json` refuses to prompt and every
        /// write here asks through `confirm_always`, so the pair would either
        /// draw a prompt into a pipe or quietly skip a confirmation. Clap
        /// refuses the combination instead, which keeps that guarantee
        /// structural rather than moving it into a check at run time.
        #[arg(long, requires = "yes")]
        json: bool,
    },

    /// Copy an entry to another env, another key, or both
    ///
    /// The thing a single-universe tool cannot do: take a profile from
    /// production into staging to reproduce a bug on real data, or onto a test
    /// account. Source and destination are named explicitly; neither defaults
    /// to `--env`, so no copy happens because a flag was forgotten.
    Copy {
        /// Entry key to read.
        entry: String,

        /// Env to read from.
        #[arg(long)]
        from: String,

        /// Env to write to. May be the same as `--from`.
        #[arg(long)]
        to: String,

        /// Key to write. Defaults to the same key.
        #[arg(long)]
        to_entry: Option<String>,

        /// Write the destination's current value here first. Defaults to a
        /// timestamped file in `.rbx/backups/<to>/`, the destination env.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable (after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days) or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Add to a numeric entry, atomically
    ///
    /// Not the same as reading and writing back: two people granting currency
    /// at the same time both land, where `set` would lose one of them.
    Increment {
        /// Entry key.
        entry: String,

        /// Amount to add. Negative subtracts.
        #[arg(long, allow_negative_numbers = true)]
        by: i64,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Compare two revisions, or the same key across two envs
    ///
    /// Writes both sides to files and hands them to a diff tool rather than
    /// rendering a diff here. `code --diff`, `git diff --no-index` and anything
    /// else already knows how to show this better than a terminal can.
    Diff {
        /// Entry key.
        entry: String,

        /// Two revision ids, comma separated.
        #[arg(long, conflicts_with = "between")]
        revisions: Option<String>,

        /// Two env names, comma separated.
        #[arg(long)]
        between: Option<String>,

        /// Open the pair in a diff tool.
        ///
        /// Uses `$RBX_DIFF_TOOL` if set, otherwise `code --diff`, otherwise
        /// `git diff --no-index`. Without this the two file paths are printed
        /// for you to open however you like.
        #[arg(long)]
        open: bool,

        /// Write the two paths to stdout as one JSON document.
        ///
        /// The paths and what each side is, never the two values: they are on
        /// disk, which is where the human form leaves them too. Rejected
        /// together with --open, which hands stdout to a diff tool and would
        /// write somebody else's output into the document. Field names are
        /// documented in docs/ops/data.md.
        #[arg(long, conflicts_with = "open")]
        json: bool,
    },

    /// Overwrite one entry with an arbitrary value
    Set {
        /// Entry key.
        entry: String,

        /// New value, as JSON on the command line.
        #[arg(long, conflicts_with = "file")]
        value: Option<String>,

        /// New value, read from a file. Easier than quoting JSON in a shell.
        #[arg(long)]
        file: Option<PathBuf>,

        /// Write the current value here before overwriting it.
        ///
        /// Defaults to a timestamped file in `.rbx/backups/<env>/`, beside
        /// `rbxplace.toml`. Roblox keeps revisions too, but a local copy needs
        /// no API call and no scope to read back.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        ///
        /// The oldest beyond this are deleted after the new one lands. Only
        /// this entry's own backups are counted, and only in the default
        /// directory: `--backup <path>` writes where it is told and prunes
        /// nothing.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable (after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days) or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Do not keep the entry's `users` and `attributes`.
        ///
        /// They are preserved by default. `users` is the association Roblox
        /// uses to answer a player's data request, so dropping it should be a
        /// decision rather than a side effect.
        #[arg(long)]
        drop_metadata: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// What happened rather than what it looks like: the action, whether
        /// it was applied or was a dry run, whether the entry existed, the
        /// revision it is at now, and where the backup went. stdout carries
        /// the document and nothing else. Field names are documented in
        /// docs/ops/data.md.
        ///
        /// Requires `--yes`. `OutputFormat::Json` refuses to prompt and every
        /// write here asks through `confirm_always`, so the pair would either
        /// draw a prompt into a pipe or quietly skip a confirmation. Clap
        /// refuses the combination instead, which keeps that guarantee
        /// structural rather than moving it into a check at run time.
        #[arg(long, requires = "yes")]
        json: bool,
    },

    /// Ordered data stores: the leaderboard resource.
    ///
    /// A different Open Cloud resource from the verbs above, not a mode of
    /// them: integer values, server-side ordering, and no revision history at
    /// all. `--datastore` names it the same way, and `--scope` applies.
    ///
    /// Nothing here writes a backup file, because there is nothing to back
    /// up: an ordered entry is one integer with no revision history behind it,
    /// so there would be nothing to reconstruct.
    Ordered {
        #[command(subcommand)]
        command: ordered::OrderedCommand,
    },
}

#[cfg(test)]
mod json_flag_tests {
    use super::*;
    use rbx_core::output::OutputFormat;

    #[derive(clap::Parser)]
    struct Wrapper {
        #[command(flatten)]
        data: DataCli,
    }

    fn parses(args: &[&str]) -> bool {
        let mut argv = vec!["data", "--datastore", "PlayerData"];
        argv.extend_from_slice(args);
        <Wrapper as clap::Parser>::try_parse_from(argv).is_ok()
    }

    /// A format that owns stdout may not stop to ask a question: that is
    /// `OutputFormat::may_prompt`, and it is false for `Json` whatever the
    /// terminal looks like. Every writing subcommand here asks one, through
    /// `confirm_always`, so none of them carries the flag: the guarantee is
    /// structural rather than a check somebody has to remember to write.
    ///
    /// This pins it. Adding `--json` to `set` would fail here, before it could
    /// make `dialoguer` draw a prompt into somebody's pipe.
    #[test]
    fn json_is_confined_to_the_subcommands_that_never_prompt() {
        assert!(!OutputFormat::Json.may_prompt());

        for reading in [
            vec!["get", "Player_156", "--json"],
            vec!["list", "--json"],
            vec!["revisions", "Player_156", "--json"],
            vec!["diff", "Player_156", "--revisions", "a,b", "--json"],
        ] {
            assert!(parses(&reading), "{reading:?} should take --json");
        }

        // A write may report what it did, but only where no prompt can happen.
        // `--yes` is what makes that true, so clap requires it rather than
        // `confirm_always` learning to keep quiet: the guarantee stays at parse
        // time, which is where it was.
        for writing in [
            vec!["set", "Player_156", "--value", "1", "--json"],
            vec!["reset", "Player_156", "--json"],
            vec!["restore-key", "Player_156", "--revision", "r1", "--json"],
            vec!["delete-key", "Player_156", "--json"],
        ] {
            assert!(
                !parses(&writing),
                "{writing:?} must not take --json without --yes"
            );

            let mut allowed = writing.clone();
            allowed.push("--yes");
            assert!(parses(&allowed), "{allowed:?} should be accepted");
        }

        // These three still carry no document at all, so the flag does not
        // exist on them and no amount of --yes conjures it.
        for never in [
            vec![
                "copy",
                "Player_156",
                "--from",
                "a",
                "--to",
                "b",
                "--json",
                "--yes",
            ],
            vec!["increment", "Player_156", "--by", "1", "--json", "--yes"],
            vec!["snapshot", "--json"],
        ] {
            assert!(!parses(&never), "{never:?} must not take --json");
        }
    }

    /// `--open` hands stdout to `git diff --no-index` and the terminal to
    /// `code --diff`. Under `--json` that ruins the document exactly as a
    /// prompt would, so the pair is refused at parse time rather than one
    /// quietly winning.
    #[test]
    fn diff_refuses_open_and_json_together() {
        assert!(parses(&["diff", "E", "--revisions", "a,b", "--open"]));
        assert!(parses(&["diff", "E", "--revisions", "a,b", "--json"]));
        assert!(!parses(&[
            "diff",
            "E",
            "--revisions",
            "a,b",
            "--open",
            "--json"
        ]));
    }
}
