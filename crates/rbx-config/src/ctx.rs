//! Per-invocation context for rbx-config commands. Built in `lib.rs::run`
//! from the clap-parsed `ConfigCli` plus the binary-wide `GlobalFlags`.

use std::path::PathBuf;

use anyhow::Result;

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

    /// Redirects every client this context builds at one host.
    ///
    /// `cfg(test)` so it cannot become a production code path: without it, a
    /// command that only talks to Roblox — `check` compares local entries
    /// against the live config — could not be asserted on at all without
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

    /// Display label for the env (used in user-facing output).
    pub fn env_label(&self) -> String {
        self.env
            .clone()
            .unwrap_or_else(|| "<universe-id>".to_string())
    }
}
