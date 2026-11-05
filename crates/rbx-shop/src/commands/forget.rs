//! Stop tracking a resource the config no longer declares.
//!
//! The way out of the drift `check` reports for one. Local only: Roblox has no
//! delete for these, so the resource stays exactly as it is, and the output says
//! so with the id, which is about to leave the only file that holds it.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Result};
use colored::Colorize;

use crate::config::{Config, ResolvedResources, ResourceKind};
use crate::ctx::ShopCtx;
use crate::lockfile::{EnvLock, Lockfile, LOCKFILE_NAME};

pub fn run(ctx: &ShopCtx<'_>, resource: ResourceKind, key: &str) -> Result<()> {
    let config = Config::load_merged(&ctx.config)?;
    let lockfile_path = ctx
        .config
        .parent()
        .unwrap_or(Path::new("."))
        .join(LOCKFILE_NAME);
    let mut lockfile = Lockfile::load(&lockfile_path)?;

    let mut forgotten = Vec::new();
    for env in ctx.resolve_envs(&config)? {
        let resources = config.resolve_env(Some(&env.name))?;
        if declares(&resources, resource, key) {
            bail!(
                "{resource} '{key}' is still declared for env '{}'. Remove it from the config \
                 first, or the next `rbx shop sync` would create a second one.",
                env.name
            );
        }
        if let Some(id) = lockfile
            .envs
            .get_mut(&env.name)
            .and_then(|lock| remove(lock, resource, key))
        {
            forgotten.push((env.name, id));
        }
    }

    if forgotten.is_empty() {
        bail!("{resource} '{key}' is not in {LOCKFILE_NAME} for the targeted env(s).");
    }
    lockfile.save(&lockfile_path)?;

    for (env, id) in &forgotten {
        println!(
            "{} No longer tracking {resource} '{key}' in env '{env}'. It stays on Roblox as it \
             is: id {id}.",
            "✓".green()
        );
    }
    println!(
        "{}",
        "If it should not stay on sale, change that in the Creator Hub: this tool no longer \
         manages it."
            .dimmed()
    );
    Ok(())
}

fn declares(resources: &ResolvedResources, kind: ResourceKind, key: &str) -> bool {
    match kind {
        ResourceKind::Pass => resources.passes.contains_key(key),
        ResourceKind::Badge => resources.badges.contains_key(key),
        ResourceKind::Product => resources.products.contains_key(key),
    }
}

/// The id of the entry removed, if there was one.
fn remove(lock: &mut EnvLock, kind: ResourceKind, key: &str) -> Option<u64> {
    fn take<T>(map: &mut BTreeMap<String, T>, key: &str, id: impl Fn(&T) -> u64) -> Option<u64> {
        map.remove(key).map(|entry| id(&entry))
    }
    match kind {
        ResourceKind::Pass => take(&mut lock.passes, key, |e| e.id),
        ResourceKind::Badge => take(&mut lock.badges, key, |e| e.id),
        ResourceKind::Product => take(&mut lock.products, key, |e| e.id),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::lockfile::{PassLock, LOCKFILE_VERSION};
    use rbx_core::GlobalFlags;
    use tempfile::TempDir;

    const NOTHING_DECLARED: &str = "[experience]\nuniverse_id = 100\n";
    const VIP_DECLARED: &str = "[experience]\nuniverse_id = 100\n\n[passes.VIP]\nprice = 99\n";

    fn repo(config: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rbxshop.toml"), config).unwrap();
        let mut lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            ..Default::default()
        };
        lockfile
            .env_mut(crate::lockfile::DEFAULT_ENV, 100)
            .passes
            .insert(
                "VIP".to_string(),
                PassLock {
                    id: 555,
                    name: "VIP".to_string(),
                    price: Some(99),
                    description: None,
                    icon_asset_id: None,
                    icon_hash: None,
                    for_sale: true,
                    regional_pricing: false,
                    managed_pricing: None,
                },
            );
        lockfile.save(&dir.path().join(LOCKFILE_NAME)).unwrap();
        dir
    }

    fn forget(dir: &TempDir, kind: ResourceKind, key: &str) -> Result<()> {
        let global = GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: None,
            place: None,
            places: dir.path().join("rbxplace.toml"),
            universe_id: None,
            place_id: Vec::new(),
        };
        let ctx = ShopCtx {
            config: dir.path().join("rbxshop.toml"),
            global: &global,
            base_url: None,
        };
        run(&ctx, kind, key)
    }

    fn lock_text(dir: &TempDir) -> String {
        std::fs::read_to_string(dir.path().join(LOCKFILE_NAME)).unwrap()
    }

    #[test]
    fn an_undeclared_resource_leaves_the_lockfile() {
        let dir = repo(NOTHING_DECLARED);

        forget(&dir, ResourceKind::Pass, "VIP").unwrap();

        assert!(!lock_text(&dir).contains("VIP"));
    }

    /// Dropping the entry of a resource still declared is what the docs warn
    /// about: the next sync plans a create, and Roblox cannot delete the twin.
    #[test]
    fn a_resource_still_declared_is_refused_and_kept() {
        let dir = repo(VIP_DECLARED);

        let err = forget(&dir, ResourceKind::Pass, "VIP").unwrap_err();

        assert!(err.to_string().contains("still declared"), "{err}");
        assert!(lock_text(&dir).contains("VIP"));
    }

    #[test]
    fn a_key_that_is_not_tracked_is_an_error_rather_than_a_success() {
        let dir = repo(NOTHING_DECLARED);

        let err = forget(&dir, ResourceKind::Pass, "Gold").unwrap_err();

        assert!(err.to_string().contains("not in"), "{err}");
    }

    /// The same key as another kind is another resource.
    #[test]
    fn the_kind_is_part_of_what_is_forgotten() {
        let dir = repo(NOTHING_DECLARED);

        forget(&dir, ResourceKind::Badge, "VIP").unwrap_err();

        assert!(lock_text(&dir).contains("VIP"));
    }
}
