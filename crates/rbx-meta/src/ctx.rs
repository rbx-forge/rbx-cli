//! Per-invocation context for rbx-meta commands. Bridges the global
//! `rbx-core` flags with rbx-meta's per-tool flag (`--config`) into a single
//! object commands can borrow.

use std::path::PathBuf;

use anyhow::{bail, Result};

use rbx_core::env::DEFAULT_ENV;
use rbx_core::places;
use rbx_core::GlobalFlags;

use crate::config::Config;

pub struct MetaCtx<'a> {
    /// Path to `rbxmeta.toml` (per-subcommand flag).
    pub config: PathBuf,
    /// All other auth/env/places flags, owned by the binary.
    pub global: &'a GlobalFlags,
}

impl<'a> MetaCtx<'a> {
    /// API key from global flags (None if not set anywhere).
    pub fn api_key(&self) -> Option<String> {
        self.global.api_key.clone()
    }

    /// Cookie resolution: explicit/env > Studio auto-detect.
    pub fn resolve_cookie(&self) -> Option<String> {
        self.global.resolve_cookie()
    }

    /// Path to rbxplace.toml (only consulted when --env is passed).
    pub fn places_path(&self) -> &std::path::Path {
        &self.global.places
    }

    /// Currently-targeted env name (if any).
    pub fn env(&self) -> Option<&str> {
        self.global.env.as_deref()
    }

    /// Currently-selected place name within the env (if any).
    pub fn place(&self) -> Option<&str> {
        self.global.place.as_deref()
    }

    /// Resolve `(env_name, universe_id, place_id)` for the current invocation.
    /// Prefers `--env` (rbxplace.toml lookup) over `[experience]` in
    /// rbxmeta.toml.
    pub fn resolve_target(&self, config: &Config) -> Result<(String, u64, u64)> {
        if let Some(env) = self.env() {
            let (universe_id, place_id) = places::resolve(self.places_path(), env, self.place())?;
            Ok((env.to_string(), universe_id, place_id))
        } else if let Some(exp) = &config.experience {
            Ok((DEFAULT_ENV.to_string(), exp.universe_id, exp.place_id))
        } else {
            bail!(
                "No target experience. Pass --env <name> (resolved via rbxplace.toml) \
                 or add [experience] (universe_id, place_id) to rbxmeta.toml."
            )
        }
    }
}
