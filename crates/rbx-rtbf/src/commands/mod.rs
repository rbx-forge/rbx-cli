pub mod check;
pub mod init;
pub mod pull;
pub mod show;
pub mod sync;
pub mod verify;

use std::path::PathBuf;

use anyhow::Result;

use rbx_core::api::{ConfigsClient, Repository};
use rbx_core::{EnvTarget, GlobalFlags};

/// What every subcommand here needs and none of them should resolve twice.
#[derive(Debug)]
pub struct RtbfCtx<'a> {
    pub global: &'a GlobalFlags,
    pub config: PathBuf,
    pub base_url: Option<String>,
}

impl RtbfCtx<'_> {
    /// A client aimed at `DataStoresConfig`, always.
    ///
    /// The repository is not a flag here. This command knows what the templates
    /// are and where they live, and offering to send them somewhere else would
    /// be offering to write a typed payload into a repository that means
    /// something different. `rbx config --repository` is the escape hatch for
    /// anyone who wants the raw transport.
    pub fn client(&self) -> Result<ConfigsClient> {
        let api_key = rbx_core::api::require_api_key(self.global.api_key.as_deref())?;
        let client = ConfigsClient::new(api_key.to_string(), Repository::DataStoresConfig);
        Ok(match &self.base_url {
            Some(url) => client.with_base_url(url.clone()),
            None => client,
        })
    }

    /// The universes to act on.
    ///
    /// Through the shared resolver, so `--env all` and `--env <group>` both
    /// work and a group is already expanded. Templates are not per env, so a
    /// fan-out here means "publish the same declaration to each of these
    /// universes", which is what a project running one codebase in several
    /// envs wants.
    pub fn targets(&self) -> Result<Vec<EnvTarget>> {
        let targets = self.global.resolve_envs()?;
        if targets.is_empty() {
            anyhow::bail!(
                "no target universe. Pass --env <name> to resolve one from rbxplace.toml, \
                 or --universe-id <id> to name it directly."
            );
        }
        Ok(targets)
    }

    /// The single universe to act on, for the commands that read one.
    pub fn single(&self) -> Result<u64> {
        self.global.single_universe()
    }
}

/// The `env: <name>` header, printed only when there is more than one target.
///
/// One spelling for this crate, matching `rbx shop sync` and `rbx place upload`:
/// a single-env run prints no header at all, so its output does not change
/// shape the day a second env is added to the file.
pub fn print_env_header(target: &EnvTarget, many: bool) {
    if many {
        use colored::Colorize;
        println!("\n{} {}", "env:".bold(), target.name.bold());
    }
}
