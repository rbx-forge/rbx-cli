//! Per-invocation context for rbx-shop commands. Wraps `GlobalFlags` plus
//! the per-tool `--config` path and resolves env targets either from
//! `rbxplace.toml` (via `GlobalFlags`) or from `[experience]` in
//! `rbxshop.toml` as a fallback.

use std::path::PathBuf;

use anyhow::{bail, Result};

use rbx_core::env::DEFAULT_ENV;
use rbx_core::owner::OwnerType;
use rbx_core::{places, EnvTarget, GlobalFlags};

use crate::api::RbxClient;
use crate::config::Config;

#[derive(Debug)]
pub struct ShopCtx<'a> {
    /// Path to `rbxshop.toml` (per-subcommand flag).
    pub config: PathBuf,
    /// All other auth/env/places flags, owned by the binary.
    pub global: &'a GlobalFlags,
    /// Redirects every client this context builds at one host.
    ///
    /// `cfg(test)` so it cannot become a production code path: without it,
    /// asserting "a dry run sends nothing" would mean either trusting the
    /// control-flow by eye or letting the non-dry-run counterpart talk to the
    /// real Roblox API.
    #[cfg(test)]
    pub base_url: Option<String>,
}

impl<'a> ShopCtx<'a> {
    pub fn api_key(&self) -> Option<String> {
        self.global.api_key.clone()
    }

    /// Build an API client for one env's universe. Commands go through this
    /// rather than `RbxClient::new` directly so there is a single seam where a
    /// test can point the whole command at a mock server.
    pub(crate) fn client(&self, universe_id: u64, bleed: bool) -> RbxClient {
        let client = RbxClient::new(self.api_key(), universe_id, bleed);
        #[cfg(test)]
        if let Some(url) = &self.base_url {
            return client.with_base_url(url.clone());
        }
        client
    }

    pub fn places_path(&self) -> &std::path::Path {
        &self.global.places
    }

    pub fn env(&self) -> Option<&str> {
        self.global.env.as_deref()
    }

    /// Resolve env targets for this invocation. Three modes:
    /// - `--env all`: expand to every env in `rbxplace.toml`.
    /// - `--env <name>`: resolve that single env from `rbxplace.toml`.
    /// - No `--env`: fall back to `[experience]` in `rbxshop.toml` and
    ///   return a single target under the sentinel env name `default`.
    pub fn resolve_envs(&self, config: &Config) -> Result<Vec<EnvTarget>> {
        match self.env() {
            Some("all") => {
                let places = places::PlacesFile::load(self.places_path())?;
                let mut targets = Vec::new();
                for name in places.env_names() {
                    let env = places.get(&name)?;
                    targets.push(EnvTarget {
                        name,
                        universe_id: env.universe_id,
                    });
                }
                if targets.is_empty() {
                    bail!(
                        "rbxplace.toml has no envs defined. \
                         Add at least one [<env>] section with universe_id."
                    );
                }
                Ok(targets)
            }
            Some(name) => {
                let universe_id = places::resolve_universe_id(self.places_path(), name)?;
                Ok(vec![EnvTarget {
                    name: name.to_string(),
                    universe_id,
                }])
            }
            None => {
                let Some(exp) = &config.experience else {
                    bail!(
                        "No target experience. Pass --env <name> (resolved via rbxplace.toml) \
                         or add [experience] (universe_id, creator) to rbxshop.toml."
                    );
                };
                Ok(vec![EnvTarget {
                    name: DEFAULT_ENV.to_string(),
                    universe_id: exp.universe_id,
                }])
            }
        }
    }

    /// Resolve a single env. Errors if `--env all` was used.
    pub fn resolve_single_env(&self, config: &Config) -> Result<EnvTarget> {
        if matches!(self.env(), Some("all")) {
            bail!("--env all is not supported for this command. Pass --env <name> instead.");
        }
        let targets = self.resolve_envs(config)?;
        Ok(targets
            .into_iter()
            .next()
            .expect("resolve_envs returns at least one target when --env != 'all'"))
    }

    /// Who owns this project, which is also who pays for a badge.
    ///
    /// Those are one question, not two. Roblox pays a group-owned game's badge
    /// out of group funds and a user-owned game's out of the user's, and there
    /// is no way to cross them — asking for personal funds on a group game has
    /// been a standing feature request since 2018 and is still not possible.
    /// So there is no payer to name separately from the owner, which is why
    /// `rbxshop.toml` has an `[owner]` override rather than a second concept.
    ///
    /// Hierarchy:
    /// 1. `[owner]` in `rbxshop.toml` (explicit, wins),
    /// 2. `[owner]` in `rbxplace.toml` (per-env `[<env>.owner]` first, then top-level),
    /// 3. Error with an actionable hint.
    ///
    /// Only the `type` is read today: Roblox's payment-source field is a 1/2
    /// enum and ignores the id.
    pub fn resolve_owner_type(&self, config: &Config, env_name: &str) -> Result<OwnerType> {
        if let Some(o) = config.owner.as_ref() {
            return Ok(o.kind);
        }
        let places_path = self.places_path();
        if places_path.exists() {
            let places = places::PlacesFile::load(places_path)?;
            if let Some(owner) = places.resolve_owner(env_name) {
                return Ok(owner.kind);
            }
        }
        bail!(
            "Badge creation needs to know who owns this experience, because that decides which balance Roblox charges: a group-owned game pays from group funds, a user-owned one from the user's. Add [owner] (type, id) to rbxplace.toml so every tool shares it, or to rbxshop.toml to override it here."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_core::owner::{Owner, OwnerType};
    use tempfile::tempdir;

    fn make_global(places: PathBuf) -> GlobalFlags {
        GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: None,
            place: None,
            places,
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    fn empty_config() -> Config {
        Config {
            experience: None,
            owner: None,
            codegen: Default::default(),
            icons: Default::default(),
            gifts: Default::default(),
            include: Default::default(),
            passes: Default::default(),
            badges: Default::default(),
            products: Default::default(),
            envs: Default::default(),
        }
    }

    #[test]
    fn resolve_owner_type_uses_shop_creator_when_present() {
        let dir = tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        // Even with a contradicting owner block in places, the shop's own
        // [creator] wins.
        std::fs::write(
            &places,
            "[owner]\ntype = \"group\"\nid = 1\n\n[dev]\nuniverse_id = 100\n",
        )
        .unwrap();
        let global = make_global(places);
        let ctx = ShopCtx {
            config: dir.path().join("rbxshop.toml"),
            global: &global,
            base_url: None,
        };
        let mut config = empty_config();
        config.owner = Some(Owner {
            kind: OwnerType::User,
            id: 42,
        });
        let resolved = ctx.resolve_owner_type(&config, "dev").unwrap();
        assert!(matches!(resolved, OwnerType::User));
    }

    #[test]
    fn resolve_owner_type_falls_back_to_places_top_level_owner() {
        let dir = tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            "[owner]\ntype = \"group\"\nid = 7\n\n[dev]\nuniverse_id = 100\n",
        )
        .unwrap();
        let global = make_global(places);
        let ctx = ShopCtx {
            config: dir.path().join("rbxshop.toml"),
            global: &global,
            base_url: None,
        };
        let config = empty_config();
        let resolved = ctx.resolve_owner_type(&config, "dev").unwrap();
        assert!(matches!(resolved, OwnerType::Group));
    }

    #[test]
    fn resolve_owner_type_prefers_per_env_owner() {
        let dir = tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            "[owner]\ntype = \"group\"\nid = 7\n\n\
             [dev]\nuniverse_id = 100\nowner = { type = \"user\", id = 42 }\n",
        )
        .unwrap();
        let global = make_global(places);
        let ctx = ShopCtx {
            config: dir.path().join("rbxshop.toml"),
            global: &global,
            base_url: None,
        };
        let config = empty_config();
        let resolved = ctx.resolve_owner_type(&config, "dev").unwrap();
        assert!(matches!(resolved, OwnerType::User));
    }

    #[test]
    fn resolve_owner_type_errors_when_nothing_is_set() {
        let dir = tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        // No [owner] block.
        std::fs::write(&places, "[dev]\nuniverse_id = 100\n").unwrap();
        let global = make_global(places);
        let ctx = ShopCtx {
            config: dir.path().join("rbxshop.toml"),
            global: &global,
            base_url: None,
        };
        let config = empty_config();
        let err = ctx
            .resolve_owner_type(&config, "dev")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("creator") || err.contains("owner"),
            "got {}",
            err
        );
    }
}
