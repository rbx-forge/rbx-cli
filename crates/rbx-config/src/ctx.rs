//! Per-invocation context for rbx-config commands. Built in `lib.rs::run`
//! from the clap-parsed `ConfigCli` plus the binary-wide `GlobalFlags`.

use std::path::PathBuf;

use anyhow::{bail, Result};

use rbx_core::api::Repository;

use crate::places_config::PlacesConfig;

/// Snapshot of the flags / env state every command needs. Constructed once
/// per invocation in `lib.rs::run`; commands borrow it immutably.
pub struct ConfigCtx {
    /// Path to `rbxconfig.toml` (per-subcommand flag).
    pub config: PathBuf,

    /// Path to `rbxplace.toml` (from `GlobalFlags::places`).
    pub places: PathBuf,

    /// Open Cloud API key (from `GlobalFlags::api_key`).
    pub api_key: Option<String>,

    /// Target env name (from `GlobalFlags::env`).
    pub env: Option<String>,

    /// Universe id override (per-subcommand flag). Bypasses rbxplace.toml
    /// lookup; `env` is still required to name a section in rbxconfig.toml
    /// for sync/pull/check.
    pub universe_id: Option<u64>,

    /// The repository named by `--repository`, already parsed.
    ///
    /// `Option` rather than a `Repository` defaulted at parse time, because
    /// `resolve_repository` has to tell "not passed" from "passed the
    /// default": `--repository DataStoresConfig` against a silent file is a
    /// plain instruction, while the same flag against a file naming
    /// `InExperienceConfig` is two people disagreeing.
    pub repository: Option<Repository>,

    /// Redirects every client this context builds at one host.
    ///
    /// `cfg(test)` so it cannot become a production code path: without it, a
    /// command that only talks to Roblox (`check` compares local entries
    /// against the live config) could not be asserted on at all without
    /// reaching the real API.
    #[cfg(test)]
    pub base_url: Option<String>,
}

impl ConfigCtx {
    /// Resolve (env_name, universe_id, confirm_required) for the current invocation.
    ///
    /// - If `--universe-id` is set, uses it directly (confirm defaults to false).
    ///   `--env` still names the section in rbxconfig.toml that sync/pull/check
    ///   operate on; bails if missing.
    /// - Otherwise, `--env` is required and the env entry (universe_id + confirm)
    ///   is read from rbxplace.toml.
    pub fn resolve_target(&self) -> Result<(String, u64, bool)> {
        let env = self.env.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "--env <name> is required. Resolved against rbxplace.toml \
                 (or pass --universe-id <id> to bypass the lookup, but --env \
                 is still required to name the section in rbxconfig.toml)."
            )
        })?;

        let (universe_id, confirm) = match self.universe_id {
            Some(id) => (id, false),
            None => {
                let places = PlacesConfig::load(&self.places)?;
                let entry = places.get_env(env)?;
                (entry.universe_id, entry.confirm)
            }
        };

        Ok((env.to_string(), universe_id, confirm))
    }

    /// Same as resolve_target but read-only commands (get, list, versions, rollback)
    /// can work without --env if --universe-id is given.
    pub fn resolve_universe_only(&self) -> Result<u64> {
        if let Some(id) = self.universe_id {
            return Ok(id);
        }
        let env = self.env.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "--env <name> or --universe-id <id> is required. \
                 --env is resolved against rbxplace.toml."
            )
        })?;
        let places = PlacesConfig::load(&self.places)?;
        places.universe_id(env)
    }

    /// The repository a command that reads `rbxconfig.toml` addresses.
    ///
    /// `declared` is what the file named, `None` when it said nothing. Three
    /// of the four cases are an `unwrap_or`; the fourth is why this function
    /// exists. When the flag and the file name different repositories one of
    /// the two is a mistake, and there is no way to tell which from here, so
    /// picking either would publish a file's entries into a repository nobody
    /// asked for. That write replaces a live config wholesale and cannot be
    /// undone by this command, so the invocation is refused and both names are
    /// printed with the file that holds one of them.
    pub fn resolve_repository(&self, declared: Option<Repository>) -> Result<Repository> {
        match (self.repository, declared) {
            (Some(flag), Some(file)) if flag != file => bail!(
                "--repository {flag} contradicts {}, which names {file}.\n\
                 Publishing into the wrong repository cannot be undone, so this is not \
                 resolved by picking one: drop the flag, or change the `repository` field.",
                self.config.display()
            ),
            (Some(repository), _) | (None, Some(repository)) => Ok(repository),
            (None, None) => Ok(Repository::default()),
        }
    }

    /// The repository a command that never opens `rbxconfig.toml` addresses.
    ///
    /// `get`, `list` and `versions` read the published side of one universe,
    /// which a bare `--universe-id` names on its own, so there is no local file
    /// in play to take a repository from: the flag, or the repository this
    /// command addressed before the flag existed.
    pub fn flag_repository(&self) -> Repository {
        self.repository.unwrap_or_default()
    }

    /// Display label for the env (used in user-facing output).
    pub fn env_label(&self) -> String {
        self.env
            .clone()
            .unwrap_or_else(|| "<universe-id>".to_string())
    }
}
