//! Manage Roblox Open Cloud API keys declaratively from `rbxapikey.toml`.
//!
//! Cookie auth is required (the API key admin endpoints aren't on Open Cloud
//! yet). Cookie resolution comes from [`rbx_core::GlobalFlags`].
//!
//! [`diagnostics`] is the read-only façade `rbx-doctor` links against; it is
//! the only supported way into this crate's internals from outside.

mod api;
mod commands;
pub mod config;
pub mod diagnostics;
mod drift;
mod git_guard;
mod introspect;
pub mod json;
mod lock;
mod owner_resolver;
mod remote_view;
pub mod scope_builder;
pub mod scope_catalog;
mod secret_store;
mod time_iso;

use anyhow::Result;
use clap::{Args, Subcommand};

use rbx_core::output::OutputFormat;
use rbx_core::GlobalFlags;

#[derive(Args, Debug)]
pub struct ApikeyCli {
    #[command(subcommand)]
    pub command: ApikeyCommands,
}

#[derive(Subcommand, Debug)]
pub enum ApikeyCommands {
    /// Create a new API key for an entry in rbxapikey.toml.
    Create {
        /// Key name from rbxapikey.toml.
        key: Option<String>,

        /// Create every key declared in rbxapikey.toml.
        #[arg(long)]
        all: bool,

        /// Allow all IPs (default: restrict to current public IP / configured CIDRs).
        #[arg(long = "no-ip")]
        no_ip: bool,

        /// Overwrite existing lockfile entries.
        #[arg(long)]
        force: bool,

        /// Skip post-create introspect verification.
        #[arg(long = "no-verify")]
        no_verify: bool,

        /// Skip the confirmation that names the account the key is minted on.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Check whether you can create keys for an experience
    ///
    /// Authenticates as you with the Studio cookie, not with an API key: a key
    /// is bound to its universes, so it cannot answer for one it does not
    /// already cover. Run it before writing a `[keys.x]` block for a universe
    /// you have never touched, especially a group-owned one where the answer
    /// depends on your role.
    ///
    /// Targets come from the global `--place-id` (repeatable, each resolved to
    /// the universe containing it), or from `--universe-id` / `--env`. There is
    /// deliberately no bare positional id: place ids and universe ids are both
    /// plain integers and overlap in practice, so a tool that decides which one
    /// you meant can answer about a different game.
    ///
    /// `--place-id` used to be declared here as well as globally, which clap
    /// rejects once both exist. The duplicate was the real problem rather than
    /// the error: one spelling of "a place id" has to mean one thing everywhere
    /// in the tool. It stays repeatable globally for this command's sake, and
    /// commands acting on one place call `single_place`, which refuses a
    /// repeated flag by name rather than taking the first.
    CanManage,

    /// List all keys from rbxapikey.toml and the lockfile.
    List {
        /// Compact view: only name and expiration date.
        #[arg(long = "expiry-only")]
        expiry_only: bool,

        /// Sort keys (use "expiry" to sort by expiration date; default alphabetical).
        #[arg(long)]
        sort: Option<String>,

        /// List what the account actually holds, not what this project declares.
        ///
        /// Shows every key the Creator Hub would show for the active cookie,
        /// each marked tracked (in this project's lockfile) or untracked. Most
        /// keys on a working account are untracked: they belong to other
        /// checkouts, other tools, or were made by hand.
        #[arg(long)]
        remote: bool,

        /// List a group's keys instead of your own (only with --remote).
        #[arg(long = "group-id", value_name = "GROUP_ID")]
        group_id: Option<u64>,

        /// Write the result to stdout as one JSON document.
        ///
        /// Two shapes, and `--remote` is what picks between them: what this
        /// project declares, or what the account holds. Neither carries a
        /// secret, a piece of one, or the path to the file holding one, not
        /// even the Creator Hub's preview column. stdout carries the document
        /// and nothing else; notes and warnings stay on stderr. Field names are
        /// documented in docs/apikey.md.
        ///
        /// Refused with --expiry-only, which is a narrower rendering of the
        /// same rows and has no document of its own.
        #[arg(long, conflicts_with = "expiry_only")]
        json: bool,
    },

    /// Select keys on the account and delete them.
    ///
    /// Works from the account listing rather than the lockfile, so it can
    /// reach keys this project never created, which is what makes it useful
    /// and what makes it dangerous. Nothing is preselected and there is no
    /// `--all`: deleting an untracked key breaks whatever else depends on it,
    /// silently.
    Prune {
        /// Prune a group's keys instead of your own.
        #[arg(long = "group-id", value_name = "GROUP_ID")]
        group_id: Option<u64>,

        /// Only offer keys this project's lockfile does not track.
        #[arg(long = "untracked-only")]
        untracked_only: bool,

        /// Only offer keys whose expiry has passed.
        #[arg(long = "expired-only")]
        expired_only: bool,

        /// Show the candidates and exit without deleting anything.
        #[arg(long = "dry-run")]
        dry_run: bool,

        /// Skip the final confirmation (the selection is still interactive).
        #[arg(long, short = 'y')]
        yes: bool,

        /// Also delete secret_file for tracked keys, without asking.
        #[arg(long = "clean-files")]
        clean_files: bool,
    },

    /// Reconcile rbxapikey.toml, lockfile, and Roblox state.
    Status {
        /// Also query Roblox for each cloud_auth_id (detects orphan_remote).
        #[arg(long)]
        remote: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// One verdict per key, plus the counts the summary line prints. No
        /// secret and no free-text advice: the status word says what the
        /// sentence said. stdout carries the document and nothing else; notes
        /// and warnings stay on stderr. Field names are documented in
        /// docs/apikey.md.
        #[arg(long)]
        json: bool,
    },

    /// Re-apply key configuration from rbxapikey.toml.
    Update {
        /// Key name from rbxapikey.toml.
        key: Option<String>,

        /// Update every key in the lockfile.
        #[arg(long)]
        all: bool,

        /// Allow all IPs (default: restrict to current public IP / configured CIDRs).
        #[arg(long = "no-ip")]
        no_ip: bool,

        /// If `secret_file` changed in rbxapikey.toml, delete the old secret file without asking.
        #[arg(long = "clean-files")]
        clean_files: bool,

        /// Skip confirmation prompt before pushing config to Roblox.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Rotate the API key secret.
    Regenerate {
        /// Key name from rbxapikey.toml.
        key: Option<String>,

        /// Rotate every key in the lockfile.
        #[arg(long)]
        all: bool,

        /// Skip confirmation (required for --all).
        #[arg(long, short = 'y')]
        yes: bool,

        /// If `secret_file` changed in rbxapikey.toml, delete the old secret file without asking.
        #[arg(long = "clean-files")]
        clean_files: bool,
    },

    /// Delete a key from Roblox and the local lock.
    Delete {
        /// Key name from rbxapikey.toml.
        key: Option<String>,

        /// Delete every key in the lockfile.
        #[arg(long)]
        all: bool,

        /// Skip confirmation prompts.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Also delete secret_file (if configured) without asking.
        #[arg(long = "clean-files")]
        clean_files: bool,
    },

    /// Print the raw API key secret (for use in scripts).
    Resolve {
        /// Key name from rbxapikey.toml.
        key: String,
    },

    /// Show what Roblox has stored for a key (requires key < 1h old).
    Introspect {
        /// Key name from rbxapikey.toml.
        key: String,
    },

    /// Inspect the bundled scope catalog.
    Scopes {
        #[command(subcommand)]
        action: ScopesAction,
    },

    /// Manage the embedded scope catalog.
    Catalog {
        #[command(subcommand)]
        action: CatalogAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum ScopesAction {
    /// List all known scopes grouped by target type.
    List,
    /// Show details for one scope type.
    Show {
        /// Scope type (e.g. "universe", "asset").
        scope_type: String,

        /// Write the result to stdout as one JSON document.
        ///
        /// The catalog's answer for one scope type, including `known: false`
        /// for a scope it does not list: the catalog is advisory and
        /// rbxapikey.toml forwards any string. Field names are documented in
        /// docs/apikey.md.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CatalogAction {
    /// Regenerate catalog from Roblox openapi.json (uses default URL if not provided).
    Regenerate {
        /// Custom openapi.json URL.
        url: Option<String>,
    },
    // There was a `List` here that called the same function as
    // `scopes list`, byte for byte. Two spellings of one output is a thing to
    // learn rather than a convenience, in the subcommand that is already the
    // largest in the tool. `catalog` keeps the verb only it has.
}

pub async fn run(cli: ApikeyCli, global: &GlobalFlags) -> Result<()> {
    match cli.command {
        ApikeyCommands::Create {
            key,
            all,
            no_ip,
            force,
            no_verify,
            yes,
        } => commands::create::run(global, key.as_deref(), all, no_ip, force, no_verify, yes).await,

        ApikeyCommands::CanManage => commands::permissions::run(global, &global.place_id).await,

        ApikeyCommands::List {
            expiry_only,
            sort,
            remote,
            group_id,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            if remote {
                commands::remote::run(global, group_id, format).await
            } else if group_id.is_some() {
                anyhow::bail!("--group-id only applies with --remote")
            } else {
                commands::list::run(expiry_only, sort.as_deref(), format)
            }
        }

        ApikeyCommands::Prune {
            group_id,
            untracked_only,
            expired_only,
            dry_run,
            yes,
            clean_files,
        } => {
            commands::prune::run(
                global,
                commands::prune::PruneOptions {
                    group_id,
                    untracked_only,
                    expired_only,
                    dry_run,
                    yes,
                    clean_files,
                },
            )
            .await
        }

        ApikeyCommands::Status { remote, json } => {
            commands::status::run(global, remote, OutputFormat::from_json_flag(json)).await
        }

        ApikeyCommands::Update {
            key,
            all,
            no_ip,
            clean_files,
            yes,
        } => commands::update::run(global, key.as_deref(), all, no_ip, clean_files, yes).await,

        ApikeyCommands::Regenerate {
            key,
            all,
            yes,
            clean_files,
        } => commands::regenerate::run(global, key.as_deref(), all, yes, clean_files).await,

        ApikeyCommands::Delete {
            key,
            all,
            yes,
            clean_files,
        } => commands::delete::run(global, key.as_deref(), all, yes, clean_files).await,

        ApikeyCommands::Resolve { key } => commands::resolve::run(&key),

        ApikeyCommands::Introspect { key } => commands::introspect::run(global, &key).await,

        ApikeyCommands::Scopes { action } => match action {
            ScopesAction::List => {
                commands::scopes::list();
                Ok(())
            }
            ScopesAction::Show { scope_type, json } => {
                commands::scopes::show(&scope_type, OutputFormat::from_json_flag(json))
            }
        },

        ApikeyCommands::Catalog { action } => match action {
            CatalogAction::Regenerate { url } => {
                commands::catalog::regenerate(url.as_deref()).await
            }
        },
    }
}

/// Where `--json` is allowed to appear, and why that is a rule rather than an
/// arrangement.
#[cfg(test)]
mod json_flag_tests {
    use super::*;

    #[derive(clap::Parser)]
    struct Wrapper {
        #[command(flatten)]
        apikey: ApikeyCli,
    }

    fn parses(args: &[&str]) -> bool {
        let mut argv = vec!["apikey"];
        argv.extend_from_slice(args);
        <Wrapper as clap::Parser>::try_parse_from(argv).is_ok()
    }

    /// A format that owns stdout may not stop to ask a question: that is
    /// `OutputFormat::may_prompt`, and it is false for `Json` whatever the
    /// terminal looks like. Every subcommand here that writes asks one
    /// (`create` and `update` confirm, `prune` draws a multi-select, `delete`
    /// and `regenerate` confirm twice) so none of them carries the flag. The
    /// guarantee is structural rather than a check somebody has to remember.
    ///
    /// `resolve` is on the list for a different reason and a harder one: it
    /// prints the raw secret. There is no document that could carry its answer,
    /// so there is no `--json` on it to ask for one.
    #[test]
    fn json_is_confined_to_the_subcommands_that_neither_prompt_nor_print_a_secret() {
        assert!(!OutputFormat::Json.may_prompt());

        for reading in [
            vec!["list", "--json"],
            vec!["list", "--remote", "--json"],
            vec!["list", "--remote", "--group-id", "1", "--json"],
            vec!["list", "--sort", "expiry", "--json"],
            vec!["status", "--json"],
            vec!["status", "--remote", "--json"],
            vec!["scopes", "show", "universe", "--json"],
        ] {
            assert!(parses(&reading), "{reading:?} should take --json");
        }

        for writing in [
            vec!["create", "deploy", "--json"],
            vec!["update", "deploy", "--json"],
            vec!["regenerate", "deploy", "--json"],
            vec!["delete", "deploy", "--json"],
            vec!["prune", "--json"],
            vec!["prune", "--dry-run", "--json"],
            vec!["resolve", "deploy", "--json"],
        ] {
            assert!(!parses(&writing), "{writing:?} must not take --json");
        }
    }

    /// The subcommands left out of this lot are absent rather than refused:
    /// `introspect`, `can-manage`, `scopes list` and `catalog regenerate` have
    /// documents worth writing and nobody has asked for them yet. Adding one
    /// later is a normal change; this only pins that none of them silently
    /// accepts the flag today and prints a human table anyway.
    #[test]
    fn the_subcommands_outside_this_lot_have_no_document_yet() {
        for outside in [
            vec!["introspect", "deploy"],
            // Bare: `--place-id` is a global flag now, declared on the
            // top-level parser rather than on this subcommand, so it is not
            // part of what this parser sees. That is the point of the move:
            // one spelling of a place id, in one place.
            vec!["can-manage"],
            vec!["scopes", "list"],
            vec!["catalog", "regenerate"],
        ] {
            assert!(parses(&outside), "{outside:?} should still parse");
            let mut with_flag = outside.clone();
            with_flag.push("--json");
            assert!(!parses(&with_flag), "{with_flag:?} must not take --json");
        }
    }

    /// `--expiry-only` is a narrower rendering of the same rows: name and
    /// expiry, no id, no secret line. A document has no narrower rendering
    /// (it has the fields it promises) so asking for both is a mistake worth
    /// reporting rather than a precedence question. `--sort` is not, because
    /// the order of an array is a real difference and the document says which
    /// order it is in.
    #[test]
    fn list_refuses_the_compact_layout_alongside_json() {
        assert!(parses(&["list", "--expiry-only"]));
        assert!(parses(&["list", "--json"]));
        assert!(!parses(&["list", "--expiry-only", "--json"]));
        assert!(parses(&["list", "--sort", "expiry", "--json"]));
    }
}
