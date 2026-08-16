use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use rbx_core::lockfile::{LockfileFormat, LockfileMigration};
use serde::{Deserialize, Serialize};

pub const LOCKFILE_NAME: &str = "rbxshop.lock.toml";
pub const LOCKFILE_VERSION: u32 = 2;
/// Env name used when `--env` is omitted (standalone mode).
pub const DEFAULT_ENV: &str = "default";

/// Format migrations, keyed `"N -> N+1"` and applied in sequence on load.
///
/// Empty despite the current version being 2: `rbxshop.lock.toml` has been at
/// version 2 since the first commit, so no version 1 file was ever written by
/// a released build and there is nothing to migrate from. A version 1 file is
/// therefore refused with the "no migration registered" error rather than
/// guessed at. See `rbx_core::lockfile`.
const MIGRATIONS: &[LockfileMigration] = &[];

pub const FORMAT: LockfileFormat = LockfileFormat {
    name: LOCKFILE_NAME,
    current: LOCKFILE_VERSION,
    migrations: MIGRATIONS,
};

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct Lockfile {
    pub version: u32,

    /// One section per env name (or `default` for standalone mode).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub envs: BTreeMap<String, EnvLock>,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct EnvLock {
    pub universe_id: u64,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passes: BTreeMap<String, PassLock>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub badges: BTreeMap<String, BadgeLock>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub products: BTreeMap<String, ProductLock>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PassLock {
    pub id: u64,
    pub name: String,
    pub price: Option<u64>,
    pub description: Option<String>,
    pub icon_asset_id: Option<u64>,
    pub icon_hash: Option<String>,
    #[serde(default = "default_true")]
    pub for_sale: bool,
    #[serde(default)]
    pub regional_pricing: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BadgeLock {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub icon_asset_id: Option<u64>,
    pub icon_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ProductLock {
    pub id: u64,
    pub name: String,
    pub price: u64,
    pub description: Option<String>,
    pub icon_asset_id: Option<u64>,
    pub icon_hash: Option<String>,
    #[serde(default = "default_true")]
    pub for_sale: bool,
    #[serde(default)]
    pub regional_pricing: bool,
    #[serde(default)]
    pub store_page: bool,
}

fn default_true() -> bool {
    true
}

impl Lockfile {
    /// Load the lockfile, validating its `version` and applying any pending
    /// migrations. A missing file is a first run, not an error.
    pub fn load(path: &Path) -> Result<Self> {
        Ok(FORMAT.load(path)?.unwrap_or(Self {
            version: LOCKFILE_VERSION,
            envs: BTreeMap::new(),
        }))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    /// Mutable access to an env's lock section, inserting a fresh one when missing.
    pub fn env_mut(&mut self, env: &str, universe_id: u64) -> &mut EnvLock {
        let entry = self.envs.entry(env.to_string()).or_default();
        entry.universe_id = universe_id;
        entry
    }

    /// Read-only access to an env's lock section.
    pub fn env(&self, env: &str) -> Option<&EnvLock> {
        self.envs.get(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(LOCKFILE_NAME);
        std::fs::write(&path, content).expect("write");
        (dir, path)
    }

    #[test]
    fn the_migration_registry_is_well_formed() {
        FORMAT.validate_registry().expect("registry");
    }

    #[test]
    fn a_newer_lockfile_is_refused_instead_of_parsed_as_the_current_version() {
        let (_d, path) = write(&format!(
            "version = {}\n\n[envs.dev]\nuniverse_id = 42\n",
            LOCKFILE_VERSION + 1
        ));
        let err = Lockfile::load(&path).expect_err("a newer lockfile must be refused");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(&format!("version {}", LOCKFILE_VERSION + 1)),
            "the error must name the file's version: {msg}"
        );
        assert!(
            msg.contains(&format!("version {LOCKFILE_VERSION}")),
            "the error must name the version this build knows: {msg}"
        );
        assert!(
            msg.contains("update rbx"),
            "the error must point at the upgrade: {msg}"
        );
    }

    /// No migration is registered for `"1 -> 2"`, so a version 1 file is
    /// refused rather than read as version 2. That is the enforcement point:
    /// before this, it parsed silently.
    #[test]
    fn an_older_lockfile_with_no_registered_migration_is_refused() {
        let (_d, path) = write("version = 1\n\n[envs.dev]\nuniverse_id = 42\n");
        let err = Lockfile::load(&path).expect_err("an unmigratable lockfile must be refused");
        assert!(format!("{err:#}").contains("\"1 -> 2\""), "{err:#}");
    }
}
