use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use rbx_core::lockfile::{LockfileFormat, LockfileMigration};
use serde::{Deserialize, Serialize};

pub const LOCKFILE_NAME: &str = "rbxconfig.lock.toml";
pub const LOCKFILE_VERSION: u32 = 2;

/// Empty on purpose: `rbxconfig.lock.toml` has only ever been written at
/// version 2. The registry exists so the enforcement point is here before a
/// format change needs it, which is the part that cannot be retrofitted once
/// lockfiles are in the wild.
const MIGRATIONS: &[LockfileMigration] = &[];

pub const FORMAT: LockfileFormat = LockfileFormat {
    name: LOCKFILE_NAME,
    current: LOCKFILE_VERSION,
    migrations: MIGRATIONS,
};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LockEntry {
    pub revision_id: String,
    pub synced_at: String,
    /// universe_id resolved from rbxplace.toml at last sync. Used to detect
    /// drift: if the same env name now resolves to a different universe,
    /// the tool refuses to proceed. 0 means "not yet recorded" (older lock).
    #[serde(default)]
    pub universe_id: u64,
    #[serde(default)]
    pub entries: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LockFile {
    pub version: u32,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub envs: BTreeMap<String, LockEntry>,
}

impl Default for LockFile {
    fn default() -> Self {
        Self {
            version: LOCKFILE_VERSION,
            envs: BTreeMap::new(),
        }
    }
}

impl LockFile {
    /// Load lock file, returning empty LockFile if file doesn't exist.
    ///
    /// Goes through [`FORMAT`] so this lockfile gets the same version contract
    /// as the other two: a file from a newer `rbx` is refused rather than
    /// misread, and anything older is migrated stepwise or refused.
    pub fn load(path: &Path) -> Result<Self> {
        Ok(FORMAT.load(path)?.unwrap_or_default())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("Failed to serialize lock file")?;
        std::fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))
    }

    /// Update lock entry for an environment.
    pub fn update_env(
        &mut self,
        env: &str,
        universe_id: u64,
        revision_id: String,
        entries: BTreeMap<String, toml::Value>,
    ) {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.envs.insert(
            env.to_string(),
            LockEntry {
                revision_id,
                synced_at: now,
                universe_id,
                entries,
            },
        );
    }

    /// Refuse to operate on `env` if the recorded `universe_id` differs from
    /// the freshly-resolved one. Same defensive pattern as `rbx shop` and `rbx meta`.
    /// Pre-v0.2.0 lockfiles have `universe_id = 0` and are skipped.
    pub fn check_env_drift(&self, env: &str, current_universe_id: u64) -> Result<()> {
        if let Some(entry) = self.envs.get(env) {
            if entry.universe_id != 0 && entry.universe_id != current_universe_id {
                anyhow::bail!(
                    "Lockfile env '{}' tracks universe_id {} but the resolved target is {}. \
                     Delete the [envs.{}] section in {} if intentional.",
                    env,
                    entry.universe_id,
                    current_universe_id,
                    env,
                    LOCKFILE_NAME
                );
            }
        }
        Ok(())
    }
}

/// Load the lockfile sitting next to `config_file` and run [`LockFile::check_env_drift`]
/// in one shot. Sites that need the loaded lockfile for further work (e.g. to call
/// `update_env` after a successful write) keep loading it explicitly; sites that only
/// need the drift gate use this one-liner.
pub fn check_drift_beside(config_file: &Path, env: &str, current_universe_id: u64) -> Result<()> {
    let lock_path = config_file
        .parent()
        .unwrap_or(Path::new("."))
        .join(LOCKFILE_NAME);
    LockFile::load(&lock_path)?.check_env_drift(env, current_universe_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migration_registry_is_well_formed() {
        FORMAT.validate_registry().expect("registry");
    }

    #[test]
    fn a_missing_lockfile_loads_as_an_empty_one_at_the_current_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lockfile = LockFile::load(&dir.path().join(LOCKFILE_NAME)).expect("first run");
        assert_eq!(lockfile.version, LOCKFILE_VERSION);
        assert!(lockfile.envs.is_empty());
    }

    #[test]
    fn a_newer_lockfile_is_refused_instead_of_parsed_as_the_current_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(LOCKFILE_NAME);
        std::fs::write(
            &path,
            format!(
                "version = {}\n\n[envs.dev]\nrevision_id = \"7\"\nsynced_at = \"2026-01-01T00:00:00Z\"\n",
                LOCKFILE_VERSION + 1
            ),
        )
        .expect("write");

        let err = LockFile::load(&path).expect_err("a newer lockfile must be refused");
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

    #[test]
    fn a_current_version_lockfile_still_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(LOCKFILE_NAME);
        let mut lockfile = LockFile::default();
        lockfile.update_env("dev", 42, "rev-1".to_string(), BTreeMap::new());
        lockfile.save(&path).expect("save");

        let loaded = LockFile::load(&path).expect("load");
        assert_eq!(loaded.version, LOCKFILE_VERSION);
        assert_eq!(loaded.envs["dev"].universe_id, 42);
        assert_eq!(loaded.envs["dev"].revision_id, "rev-1");
    }
}
