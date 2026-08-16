//! Adopt an existing universe in one command.
//!
//! Every other stateful tool here assumes the TOML already describes the
//! universe. Nothing brought an existing game *into* that state: adoption meant
//! copying ids out of the Creator Hub by hand, which is exactly the kind of
//! transcription where a typo does damage. `import` is the missing gesture.
//!
//! It is deliberately **composition, not reimplementation**. Each domain is
//! imported by driving the command that already knows how — `shop init
//! --from-remote`, `meta init --from-remote`, `meta pull`, `config pull` —
//! through that crate's own public entry point. Two consequences are the whole
//! design:
//!
//! - Nothing here can drift from what `sync` and `check` expect, because
//!   nothing here decides what a lockfile entry looks like.
//! - The acceptance criterion is reachable at all: `import` then `check` is
//!   green because each domain wrote its own lockfile the way it always does.
//!
//! What `import` owns is the part nobody owned: resolving the universe, laying
//! it down in `rbxplace.toml` without disturbing what is already there, running
//! the domains in an order that works, and saying plainly what it could not
//! bring across.

pub mod discover;
pub mod places_file;
mod report;
mod run;

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use rbx_core::GlobalFlags;

/// `rbx import` — bring an existing universe under management.
#[derive(Args, Debug)]
pub struct ImportCli {
    /// Universe to adopt.
    ///
    /// Read over Open Cloud, so the API key needs `universe:read`. The place
    /// list comes from the legacy host, which needs a cookie only when the
    /// universe is private.
    ///
    /// The env this lands under comes from the global `--env`, as everywhere
    /// else: an existing `rbxplace.toml` keeps every other env, its `[owner]`
    /// and `[codegen]` blocks, and its comments, and importing a second
    /// universe under a different `--env` produces the differential overlay
    /// layout the tools already use.
    #[arg(long)]
    pub universe_id: u64,

    /// Directory to write the config files into.
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,

    /// Resolve and report what would be written, without writing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Skip the domains that need extra credentials rather than failing.
    ///
    /// On by default: an import that dies halfway because a key lacks one
    /// scope leaves a half-adopted directory, which is worse than an import
    /// that finishes and names what it skipped. Pass `--strict` to fail
    /// instead.
    #[arg(long)]
    pub strict: bool,

    /// Import only these domains (comma-separated). Defaults to all of them.
    #[arg(long, value_delimiter = ',')]
    pub only: Option<Vec<Domain>>,
}

/// One tool's worth of state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Domain {
    /// `rbxshop.toml` — game passes, badges, developer products.
    Shop,
    /// `rbxmeta.toml` — universe and place metadata, icon, thumbnails.
    Meta,
    /// `rbxconfig.toml` — the live in-experience config.
    Config,
}

impl Domain {
    pub const ALL: [Domain; 3] = [Domain::Shop, Domain::Meta, Domain::Config];

    pub fn label(self) -> &'static str {
        match self {
            Domain::Shop => "shop",
            Domain::Meta => "meta",
            Domain::Config => "config",
        }
    }
}

impl std::fmt::Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

pub async fn run(cli: ImportCli, global: &GlobalFlags) -> Result<()> {
    run::run(cli, global, discover::Hosts::default()).await
}
