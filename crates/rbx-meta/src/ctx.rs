//! Per-invocation context for rbx-meta commands. Bridges the global
//! `rbx-core` flags with rbx-meta's per-tool flag (`--config`) into a single
//! object commands can borrow.

use std::path::PathBuf;

use anyhow::{bail, Result};

use rbx_core::env::DEFAULT_ENV;
use rbx_core::places::{EnvSelector, PlacesFile};
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

    /// Every `(env_name, universe_id, place_id)` this invocation targets.
    ///
    /// `--env all` and `--env <group>` name several envs, so the list is what
    /// commands loop over; a single `--env` and the `[experience]` fallback are
    /// the one-element cases of the same shape.
    ///
    /// The selector is expanded here and never travels further. A group has no
    /// universe of its own, so a group name reaching an `[envs.<name>]` section
    /// would invent an env; expanding at the front makes that unrepresentable
    /// rather than merely avoided. `GlobalFlags::resolve_envs` states the same
    /// rule for the universe-only tools.
    pub fn resolve_targets(&self, config: &Config) -> Result<Vec<(String, u64, u64)>> {
        let Some(selector) = self.global.env_selector()? else {
            let Some(exp) = &config.experience else {
                bail!(
                    "No target experience. Pass --env <name> (resolved via rbxplace.toml) \
                     or add [experience] (universe_id, place_id) to rbxmeta.toml."
                )
            };
            return Ok(vec![(
                DEFAULT_ENV.to_string(),
                exp.universe_id,
                exp.place_id,
            )]);
        };

        let names = match selector {
            EnvSelector::One(name) => vec![name],
            EnvSelector::Every => PlacesFile::load(self.places_path())?.env_names(),
            EnvSelector::Group { members, .. } => members,
        };
        if names.is_empty() {
            bail!(
                "rbxplace.toml has no envs defined. \
                 Add at least one [<env>] section with universe_id."
            )
        }
        // Per env rather than from one load of the file, so `--place` is
        // honoured exactly as a single-env run honours it.
        names
            .into_iter()
            .map(|name| {
                let (universe_id, place_id) = self.global.resolve_place(&name)?;
                Ok((name, universe_id, place_id))
            })
            .collect()
    }

    /// Resolve `(env_name, universe_id, place_id)` for a command that acts on
    /// one env. Prefers `--env` (rbxplace.toml lookup) over `[experience]` in
    /// rbxmeta.toml.
    ///
    /// A plural selector is refused here rather than resolved to its first
    /// member. `--env all` is a legitimate flag everywhere else in the suite,
    /// and the answer it used to get (`unknown env 'all'`, from handing the
    /// literal name to `rbxplace.toml`) said nothing about why a command will
    /// not take it.
    ///
    /// Reserved: `check`, `sync` and `pull` all fan out, so nothing calls this
    /// today. It stays as the one place a single-env `rbx meta` subcommand gets
    /// its target, so that adding one cannot reintroduce the unknown-env answer
    /// to `--env all`. The refusal is pinned by a test.
    #[allow(dead_code)]
    pub fn resolve_target(&self, config: &Config) -> Result<(String, u64, u64)> {
        if let Some(selector) = self.global.env_selector()? {
            selector.single("envs")?;
        }
        Ok(self
            .resolve_targets(config)?
            .into_iter()
            .next()
            .expect("a selector naming one env resolves to exactly one target"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Two envs, one place each, plus a group naming them in an order that is
    /// neither alphabetical nor the file's own, so an expansion that ignored
    /// the declaration order would show.
    const PLACES: &str = r#"
[groups]
nonprod = ["staging", "dev"]

[dev]
universe_id = 100
[dev.places]
main = 200

[staging]
universe_id = 150
[staging.places]
main = 250
lobby = 260
"#;

    const CONFIG: &str = r#"
[experience]
universe_id = 300
place_id = 400

[game]
name = "Declared"
"#;

    fn repo() -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxplace.toml"), PLACES).expect("write places");
        std::fs::write(dir.path().join("rbxmeta.toml"), CONFIG).expect("write config");
        dir
    }

    fn flags(dir: &TempDir, env: Option<&str>, place: Option<&str>) -> GlobalFlags {
        GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: env.map(String::from),
            place: place.map(String::from),
            places: dir.path().join("rbxplace.toml"),
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    fn config(dir: &TempDir) -> Config {
        Config::load(&dir.path().join("rbxmeta.toml")).expect("load config")
    }

    #[test]
    fn all_expands_to_every_env_in_the_file() {
        let dir = repo();
        let global = flags(&dir, Some("all"), Some("main"));
        let ctx = MetaCtx {
            config: dir.path().join("rbxmeta.toml"),
            global: &global,
        };

        assert_eq!(
            ctx.resolve_targets(&config(&dir)).expect("resolve all"),
            vec![
                ("dev".to_string(), 100, 200),
                ("staging".to_string(), 150, 250),
            ]
        );
    }

    #[test]
    fn a_group_expands_to_its_members_in_declared_order() {
        let dir = repo();
        let global = flags(&dir, Some("nonprod"), Some("main"));
        let ctx = MetaCtx {
            config: dir.path().join("rbxmeta.toml"),
            global: &global,
        };

        assert_eq!(
            ctx.resolve_targets(&config(&dir)).expect("resolve group"),
            vec![
                ("staging".to_string(), 150, 250),
                ("dev".to_string(), 100, 200),
            ]
        );
    }

    /// `--place` picks the same entry it would for a single env, per env: the
    /// alternative (reading it once, for the first env) would silently apply
    /// one env's place name to another's map.
    #[test]
    fn place_is_honoured_per_env() {
        let dir = repo();
        let global = flags(&dir, Some("staging"), Some("lobby"));
        let ctx = MetaCtx {
            config: dir.path().join("rbxmeta.toml"),
            global: &global,
        };

        assert_eq!(
            ctx.resolve_targets(&config(&dir)).expect("resolve staging"),
            vec![("staging".to_string(), 150, 260)]
        );
    }

    #[test]
    fn no_env_falls_back_to_the_experience_block() {
        let dir = repo();
        let global = flags(&dir, None, None);
        let ctx = MetaCtx {
            config: dir.path().join("rbxmeta.toml"),
            global: &global,
        };

        assert_eq!(
            ctx.resolve_targets(&config(&dir)).expect("standalone"),
            vec![(DEFAULT_ENV.to_string(), 300, 400)]
        );
    }

    /// The confusing answer this replaces: `all` was handed to rbxplace.toml as
    /// an env name and came back as "unknown env", which says nothing about the
    /// command refusing the flag.
    #[test]
    fn a_single_env_command_refuses_a_plural_selector() {
        let dir = repo();
        for value in ["all", "nonprod"] {
            let global = flags(&dir, Some(value), Some("main"));
            let ctx = MetaCtx {
                config: dir.path().join("rbxmeta.toml"),
                global: &global,
            };

            let err = ctx
                .resolve_target(&config(&dir))
                .expect_err("several envs, one target");
            let text = format!("{err:#}");
            assert!(
                text.contains("this command acts on one"),
                "{value}: says why, not 'unknown env': {text}"
            );
        }
    }

    #[test]
    fn a_named_env_still_resolves_to_one_target() {
        let dir = repo();
        let global = flags(&dir, Some("dev"), None);
        let ctx = MetaCtx {
            config: dir.path().join("rbxmeta.toml"),
            global: &global,
        };

        assert_eq!(
            ctx.resolve_target(&config(&dir)).expect("resolve dev"),
            ("dev".to_string(), 100, 200)
        );
    }
}
