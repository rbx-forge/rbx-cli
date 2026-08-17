use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use rbx_core::lockfile::{LockfileFormat, LockfileMigration};
use serde::{Deserialize, Serialize};

use crate::config::{
    Avatar, Devices, Genre, PaidAccess, Permissions, PrivateServer, ServerFill, SocialLinks,
    Visibility,
};

pub const LOCKFILE_NAME: &str = "rbxmeta.lock.toml";
pub const LOCKFILE_VERSION: u32 = 1;
/// Env name used when `--env` is omitted (no rbxplace.toml lookup).
pub const DEFAULT_ENV: &str = "default";

/// Format migrations, keyed `"N -> N+1"` and applied in sequence on load.
///
/// Empty because `rbxmeta.lock.toml` has only ever had one format. The
/// registry exists anyway: it is the hook a future format change plugs into,
/// and it is worth nothing if it is added after the files are already out
/// there. See `rbx_core::lockfile`.
const MIGRATIONS: &[LockfileMigration] = &[];

pub const FORMAT: LockfileFormat = LockfileFormat {
    name: LOCKFILE_NAME,
    current: LOCKFILE_VERSION,
    migrations: MIGRATIONS,
};

#[derive(Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Lockfile {
    pub version: u32,

    /// One section per env name (or `default` for the no-env mode).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub envs: BTreeMap<String, EnvLock>,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct EnvLock {
    pub universe_id: u64,
    pub place_id: u64,

    #[serde(default, skip_serializing_if = "GameLock::is_empty")]
    pub game: GameLock,

    #[serde(default, skip_serializing_if = "MediaLockfile::is_empty")]
    pub media: MediaLockfile,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct GameLock {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_size: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_chat: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_copying: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub studio_access_to_apis_allowed: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub beta_mode: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_server: Option<PrivateServer>,

    #[serde(default, skip_serializing_if = "Devices::is_empty")]
    pub devices: Devices,

    #[serde(default, skip_serializing_if = "SocialLinks::is_empty")]
    pub social_links: SocialLinks,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_fill: Option<ServerFill>,

    /// The permissions this tool last wrote.
    ///
    /// Not "what Roblox has": Roblox exposes no GET for these, so unlike every
    /// other field here this one cannot be re-read. A change made in the
    /// dashboard will not be noticed until the next `sync` overwrites it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,

    #[serde(default, skip_serializing_if = "Avatar::is_empty")]
    pub avatar: Avatar,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_access: Option<PaidAccess>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<Genre>,

    /// Hash of the `engine_avatar_settings` document as it was last sent.
    ///
    /// The hash and not the content: the document is a whole avatar
    /// configuration, and a lockfile is a record of what was applied rather
    /// than a second copy of the input. Hashing the *canonical* serialisation
    /// rather than the file bytes is what keeps reformatting the JSON from
    /// looking like a change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_avatar_settings_hash: Option<String>,
}

impl GameLock {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.server_size.is_none()
            && self.voice_chat.is_none()
            && self.allow_copying.is_none()
            && self.visibility.is_none()
            && self.studio_access_to_apis_allowed.is_none()
            && self.beta_mode.is_none()
            && self.private_server.is_none()
            && self.devices.is_empty()
            && self.social_links.is_empty()
            && self.server_fill.is_none()
            && self.permissions.is_none()
            && self.avatar.is_empty()
            && self.paid_access.is_none()
            && self.genre.is_none()
            && self.engine_avatar_settings_hash.is_none()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MediaLockfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<MediaLock>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thumbnails: Vec<MediaLock>,
}

impl MediaLockfile {
    pub fn is_empty(&self) -> bool {
        self.icon.is_none() && self.thumbnails.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MediaLock {
    /// blake3 hash of processed PNG bytes (after alpha bleed if enabled).
    pub hash: String,
    /// Image ID returned by Roblox (used for thumbnails delete/reorder).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_id: Option<u64>,
}

impl Lockfile {
    /// Load the lockfile, validating its `version` and applying any pending
    /// migrations. A missing file is a first run, not an error.
    pub fn load(path: &Path) -> Result<Self> {
        Ok(FORMAT.load(path)?.unwrap_or(Self {
            version: LOCKFILE_VERSION,
            ..Self::default()
        }))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    /// Get the EnvLock for an env, creating an empty one if absent.
    pub fn env_mut(&mut self, env: &str) -> &mut EnvLock {
        self.envs.entry(env.to_string()).or_default()
    }

    /// Read-only view of an env's lock. Returns a default empty EnvLock if
    /// the env has never been synced.
    pub fn env_view(&self, env: &str) -> EnvLock {
        self.envs.get(env).cloned().unwrap_or_default()
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
    fn a_missing_lockfile_loads_as_an_empty_one_at_the_current_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lockfile = Lockfile::load(&dir.path().join(LOCKFILE_NAME)).expect("first run");
        assert_eq!(lockfile.version, LOCKFILE_VERSION);
        assert!(lockfile.envs.is_empty());
    }

    #[test]
    fn a_current_version_lockfile_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(LOCKFILE_NAME);
        let mut lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            ..Default::default()
        };
        lockfile.env_mut("dev").universe_id = 42;
        lockfile.save(&path).expect("save");

        assert_eq!(Lockfile::load(&path).expect("load"), lockfile);
    }

    #[test]
    fn a_newer_lockfile_is_refused_instead_of_parsed_as_the_current_version() {
        let (_d, path) = write(&format!(
            "version = {}\n\n[envs.dev]\nuniverse_id = 42\nplace_id = 7\n",
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
}
