//! Upload, download, promote, and rollback Roblox place files via the
//! Open Cloud API.

mod api;
mod commands;
pub mod config;
pub mod json;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use rbx_core::GlobalFlags;

#[derive(Args, Debug)]
pub struct PlaceCli {
    #[command(subcommand)]
    pub command: PlaceCommands,

    /// Override the API host. For testing against a mock server.
    ///
    /// Global rather than per-subcommand so it cannot be honored by one verb
    /// and silently ignored by the next.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum PlaceCommands {
    /// Upload a .rbxl file to a place.
    Upload {
        /// Environment name (e.g. prod, staging, dev), a `[groups]` name, or
        /// `all`.
        ///
        /// A plural selector uploads the same file to every env it names, in
        /// turn, under one confirmation covering the lot.
        #[arg(long, short)]
        env: String,

        /// Place name as defined in rbxplace.toml (defaults to the only place if unambiguous).
        #[arg(long, short)]
        place: Option<String>,

        /// Upload to every place defined in the environment.
        #[arg(long, conflicts_with = "place")]
        all_places: bool,

        /// Path to the .rbxl file to upload.
        #[arg(long, short)]
        file: PathBuf,

        /// Publish immediately (default: save as draft).
        #[arg(long)]
        published: bool,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,

        /// Write the result to stdout as one JSON document instead of the
        /// progress lines.
        ///
        /// Carries the version number Roblox assigned to each place, which is
        /// the thing a deploy pipeline cannot compute for itself. stdout
        /// carries the document and nothing else; diagnostics stay on stderr.
        /// Cannot prompt, so an env with `confirm = true` needs `--yes`. Field
        /// names are documented in docs/place.md.
        ///
        /// A plural `--env` emits one envelope holding a receipt per env; a
        /// single env emits the receipt itself, in the shape it has always
        /// had.
        #[arg(long)]
        json: bool,
    },

    /// Download a place file.
    Download {
        /// Environment name. Not needed when `--place-id` names the place.
        ///
        /// Required, like every other subcommand's copy, so a missing target
        /// is a parse error rather than a runtime one. Conditionally so here,
        /// because this is a read and `--place-id` is documented as reaching
        /// the reads without a `rbxplace.toml`: `single_place` already answers
        /// from the id alone, and only this made clap refuse first.
        #[arg(long, short, required_unless_present = "place_id")]
        env: Option<String>,

        /// Place name (defaults to the only place if unambiguous).
        #[arg(long, short)]
        place: Option<String>,

        /// Specific version to download (defaults to latest).
        #[arg(long, short)]
        version: Option<u64>,

        /// Download the latest published version specifically.
        #[arg(long, conflicts_with = "version")]
        published: bool,

        /// Download the latest saved (draft) version specifically.
        #[arg(long, conflicts_with = "version")]
        saved: bool,

        /// Output path (default: <place_id>.rbxl).
        #[arg(long, short)]
        out: Option<PathBuf>,
    },

    /// Roll back a place to a previous version.
    Rollback {
        /// Environment name.
        #[arg(long, short)]
        env: String,

        /// Place name (defaults to the only place if unambiguous).
        #[arg(long, short)]
        place: Option<String>,

        /// Version number to roll back to (shows interactive selector if omitted).
        #[arg(long, short)]
        version: Option<u64>,

        /// Number of recent versions to show in the selector (default: 10).
        #[arg(long, default_value_t = 10)]
        count: usize,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,

        /// Write the result to stdout as one JSON document instead of the
        /// progress lines.
        ///
        /// Reports both versions: the one restored and the new one Roblox
        /// created for it. Cannot prompt, so `--version` is required and an
        /// env with `confirm = true` needs `--yes`. Field names are documented
        /// in docs/place.md.
        #[arg(long)]
        json: bool,
    },

    /// List recent versions of a place.
    Versions {
        /// Environment name. Not needed when `--place-id` names the place.
        ///
        /// See `Download`: required so a missing target is a parse error,
        /// conditionally so because this is a read.
        #[arg(long, short, required_unless_present = "place_id")]
        env: Option<String>,

        /// Place name (defaults to the only place if unambiguous).
        #[arg(long, short)]
        place: Option<String>,

        /// Number of versions to show (default: 20, or 3 when --filter is published/saved).
        #[arg(
            long,
            default_value_t = 20,
            default_value_if("filter", "published", "3"),
            default_value_if("filter", "saved", "3")
        )]
        count: usize,

        /// Filter versions by type: all, published, or saved (default: all).
        #[arg(long, value_parser = ["all", "published", "saved"], default_value = "all")]
        filter: String,

        /// Write the versions to stdout as one JSON document instead of the
        /// listing.
        ///
        /// Timestamps stay in the form Roblox sent them rather than the
        /// listing's rendering. Field names are documented in docs/place.md.
        #[arg(long)]
        json: bool,
    },

    /// List all places in a universe (queries Roblox API).
    Places {
        /// Environment name (or --universe-id for one-shot listing).
        #[arg(long, short)]
        env: Option<String>,

        /// Universe id to list places for (overrides env if provided).
        #[arg(long)]
        universe_id: Option<u64>,

        /// Write the places to stdout as one JSON document instead of the
        /// listing.
        ///
        /// Each place carries the rbxplace.toml name it maps to, when there is
        /// one, so a CI job can tell a missing entry from a renamed place.
        /// Field names are documented in docs/place.md.
        #[arg(long)]
        json: bool,
    },

    /// Promote a place from one environment to another.
    Promote {
        /// Source environment.
        #[arg(long)]
        from: String,

        /// Target environment.
        #[arg(long)]
        to: String,

        /// Place to promote from (defaults to the only place if unambiguous).
        #[arg(long, short)]
        place: Option<String>,

        /// Upload to every place in the target environment.
        #[arg(long)]
        all_places: bool,

        /// Specific source version to promote.
        #[arg(long, short, conflicts_with_all = ["from_published", "from_saved"])]
        version: Option<u64>,

        /// Promote the latest published version from the source.
        #[arg(long, conflicts_with_all = ["version", "from_saved"])]
        from_published: bool,

        /// Promote the latest saved (draft) version from the source.
        #[arg(long, conflicts_with_all = ["version", "from_published"])]
        from_saved: bool,

        /// Publish immediately on the target (default: save as draft).
        #[arg(long)]
        published: bool,

        /// Write a JSON traceability log (merges with existing file).
        #[arg(long)]
        log: Option<PathBuf>,

        /// Skip confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,

        /// Write the result to stdout as one JSON document instead of the
        /// progress lines.
        ///
        /// Carries both ends of the move: the resolved source version and the
        /// version each target received. This is the same information `--log`
        /// files away, without a file. Cannot prompt, so a target env with
        /// `confirm = true` needs `--yes`. Field names are documented in
        /// docs/place.md.
        #[arg(long)]
        json: bool,
    },

    /// Fetch places from Roblox and update rbxplace.toml.
    Fetch {
        /// Environment name to update in rbxplace.toml.
        #[arg(long, short)]
        env: String,

        /// Universe id to fetch places from (overrides the one in rbxplace.toml).
        #[arg(long)]
        universe_id: Option<u64>,

        /// Write changes to rbxplace.toml (default: dry-run / show only).
        #[arg(long)]
        write: bool,
    },

    /// Moved to `rbx env gen-module`.
    ///
    /// Kept as a hidden stub purely so the old invocation gets a message
    /// naming its replacement, instead of clap's "unrecognized subcommand".
    /// Reads no `.rbxl`, so it never belonged to this domain in the first
    /// place. Remove once 0.4 has been out for a while.
    #[command(hide = true)]
    GenEnvModule {
        /// Output file path (supports .lua, .luau, .json, .ts).
        #[arg(long, short)]
        out: Option<String>,
    },
}

/// `--place-id` reaches the reads and is refused by the writes.
///
/// `download`, `versions` and `places` need only an id, so naming one skips
/// `rbxplace.toml` and they work against a place this project never declared.
///
/// The writes need more than an id and the difference is not cosmetic. The
/// `confirm = true` guard belongs to an env, and the `--json` receipt carries
/// `env` as a documented field, so an env-less write would either walk past a
/// guard somebody set on purpose or emit a document missing a field consumers
/// were told to expect. Refusing is better than either, and better than
/// accepting the flag and ignoring it, which is the failure the
/// `--universe-id` doc-comment describes from the last time it happened.
fn refuse_place_id_for_writes(global: &GlobalFlags, verb: &str) -> Result<()> {
    if !global.place_id.is_empty() {
        anyhow::bail!(
            "`--place-id` names a place but no env, and `rbx place {verb}` needs one: the confirm guard and the --json receipt are both env-scoped. Pass --env <name> (with --place <name> if the env has several), or add this place to rbxplace.toml."
        );
    }
    Ok(())
}

/// Refuse a plural `--env` on a command that acts on one env.
///
/// Five commands here resolve their env through `PlacesConfig::get_env`, which
/// knows nothing about groups and answers `Environment 'nonprod' not found`,
/// naming the one thing that is not wrong: the group exists, it just names
/// several. Routed through [`GlobalFlags::env_selector`] so `all` and a group
/// are refused identically, which is the whole point of the selector being a
/// type.
///
/// `upload` does not call this: it fans out. `download` and `promote` refuse
/// with wordings of their own, because the alternative they point at is a flag
/// rather than an env name.
fn refuse_plural_env(global: &GlobalFlags, verb: &str) -> Result<()> {
    if let Some(selector) = global.env_selector()? {
        selector
            .single("envs")
            .with_context(|| format!("`rbx place {verb}` acts on one env"))?;
    }
    Ok(())
}

pub async fn run(cli: PlaceCli, global: &GlobalFlags) -> Result<()> {
    let PlaceCli { command, base_url } = cli;
    let base_url = base_url.as_deref();

    match command {
        PlaceCommands::Upload {
            // Not read here, and not dead either: it is what makes a missing
            // `--env` a parse error instead of a runtime one. clap propagates
            // the global `--env` and fills this subcommand's own required copy
            // from the same occurrence, so the two always hold one value, and
            // the resolution goes through `GlobalFlags` because that is where
            // `all` and group expansion already live. A second expansion here
            // would be a second answer to one question.
            env: _,
            place,
            all_places,
            file,
            published,
            yes,
            json,
        } => {
            refuse_place_id_for_writes(global, "upload")?;
            commands::upload::run(
                global,
                base_url,
                &global.resolve_envs()?,
                place.as_deref(),
                all_places,
                &file,
                published,
                yes,
                json,
            )
            .await
        }

        PlaceCommands::Download {
            env,
            place,
            version,
            published,
            saved,
            out,
        } => {
            commands::download::run(
                global,
                base_url,
                env.as_deref(),
                place.as_deref(),
                version,
                published,
                saved,
                out.as_deref(),
            )
            .await
        }

        PlaceCommands::Promote {
            from,
            to,
            place,
            all_places,
            version,
            from_published,
            from_saved,
            published,
            log,
            yes,
            json,
        } => {
            refuse_place_id_for_writes(global, "promote")?;
            commands::promote::run(
                global,
                base_url,
                &from,
                &to,
                place.as_deref(),
                all_places,
                version,
                from_published,
                from_saved,
                published,
                log.as_deref(),
                yes,
                json,
            )
            .await
        }

        PlaceCommands::Rollback {
            env,
            place,
            version,
            count,
            yes,
            json,
        } => {
            refuse_place_id_for_writes(global, "rollback")?;
            refuse_plural_env(global, "rollback")?;
            commands::rollback::run(
                global,
                base_url,
                &env,
                place.as_deref(),
                version,
                count,
                yes,
                json,
            )
            .await
        }

        PlaceCommands::Versions {
            env,
            place,
            count,
            filter,
            json,
        } => {
            refuse_plural_env(global, "versions")?;
            commands::versions::run(
                global,
                base_url,
                env.as_deref(),
                place.as_deref(),
                count,
                &filter,
                json,
            )
            .await
        }

        PlaceCommands::Places {
            env,
            universe_id,
            json,
        } => {
            refuse_plural_env(global, "places")?;
            commands::places::run(global, base_url, env.as_deref(), universe_id, json).await
        }

        PlaceCommands::Fetch {
            env,
            universe_id,
            write,
        } => {
            refuse_plural_env(global, "fetch")?;
            commands::fetch::run(global, base_url, &env, universe_id, write).await
        }

        PlaceCommands::GenEnvModule { out } => {
            let replacement = match out.as_deref() {
                Some(path) => format!("rbx env gen-module --out {path}"),
                None => "rbx env gen-module --out <path>".to_string(),
            };
            anyhow::bail!(
                "`rbx place gen-env-module` moved to `rbx env gen-module`, which owns \
                 rbxplace.toml.\nRun: {}",
                replacement
            )
        }
    }
}
