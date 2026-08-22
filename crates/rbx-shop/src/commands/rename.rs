use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::config::{Config, ConfigFile, KeyRename, ResourceKind};
use crate::ctx::ShopCtx;
use crate::gifts::gift_key;
use crate::lockfile::{Lockfile, LOCKFILE_NAME};

pub fn run(ctx: &ShopCtx<'_>, resource: ResourceKind, old_key: &str, new_key: &str) -> Result<()> {
    let config_path = &ctx.config;
    let lockfile_path = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(LOCKFILE_NAME);

    let mut files = Config::load_all(config_path)?;
    let mut lockfile = Lockfile::load(&lockfile_path)?;
    let kind = resource;

    rename_across_files(kind, &mut files, &mut lockfile, old_key, new_key)?;

    // Told about the move, the write-back carries the entry's comments and its
    // place in the file across to the new key instead of dropping the old
    // table and appending a bare one at the end.
    let renames = [KeyRename {
        kind,
        from: old_key.to_string(),
        to: new_key.to_string(),
    }];
    for file in &files {
        file.config
            .save_in_place_renaming(&file.path, &renames)
            .with_context(|| format!("Failed to rewrite {}", file.path.display()))?;
    }
    lockfile.save(&lockfile_path)?;

    println!("Renamed {kind} '{old_key}' -> '{new_key}'");
    Ok(())
}

/// Rename the key across every loaded file's base + env overlays, plus every
/// env lock section. The resource must exist somewhere (any file's base or
/// overlay). `files[0]` is always the main file (see `Config::load_all`).
fn rename_across_files(
    kind: ResourceKind,
    files: &mut [ConfigFile],
    lockfile: &mut Lockfile,
    old_key: &str,
    new_key: &str,
) -> Result<()> {
    let gift_key_prefix = files[0].config.gifts.key_prefix.clone();
    let capitalize_key = files[0].config.gifts.capitalize_key;

    // Pre-check across every file: new_key must not already exist anywhere.
    for file in files.iter() {
        check_new_key_available(
            kind,
            &file.config,
            new_key,
            &file.path,
            &gift_key_prefix,
            capitalize_key,
        )?;
    }

    // Applying the rename to every file is safe unconditionally: it's a
    // no-op in whichever files don't currently have `old_key` anywhere.
    let mut found_anywhere = false;
    for file in files.iter_mut() {
        // Both sides run: an entry can live in base *and* be overridden in an
        // env, and `|` rather than `||` is what keeps the second from being
        // skipped once the first hits.
        found_anywhere |= kind.rename_base(&mut file.config, old_key, new_key)
            | kind.rename_overlays(&mut file.config, old_key, new_key);
    }
    if !found_anywhere {
        bail!(
            "{kind} '{old_key}' not found in base or any env overlay (across {} file(s))",
            files.len()
        );
    }

    // The lockfile is a single global structure (never split across files)
    // so this part is unchanged regardless of how many config files exist.
    for env_lock in lockfile.envs.values_mut() {
        match kind {
            ResourceKind::Pass => rename_entry_silent(&mut env_lock.passes, old_key, new_key),
            ResourceKind::Badge => rename_entry_silent(&mut env_lock.badges, old_key, new_key),
            ResourceKind::Product => rename_entry_silent(&mut env_lock.products, old_key, new_key),
        }

        // A gift twin (if any) lives in the products lockfile section under a
        // derived key: carry the rename over so the next sync updates the
        // existing remote product instead of creating a duplicate. Badges have
        // no `create_gift`, so nothing twins them.
        if kind != ResourceKind::Badge {
            rename_entry_silent(
                &mut env_lock.products,
                &gift_key(&gift_key_prefix, capitalize_key, old_key),
                &gift_key(&gift_key_prefix, capitalize_key, new_key),
            );
        }
    }

    Ok(())
}

/// Part of the pre-check: `new_key` must not already exist in `config`
/// (base or any overlay), nor collide with a gift-derived key.
fn check_new_key_available(
    kind: ResourceKind,
    config: &Config,
    new_key: &str,
    path: &Path,
    gift_key_prefix: &str,
    capitalize_key: bool,
) -> Result<()> {
    if kind.base_contains(config, new_key) {
        bail!("{kind} '{new_key}' already exists in {}", path.display());
    }
    if let Some(env) = kind.overlay_envs(config, new_key).first() {
        bail!(
            "{kind} '{new_key}' already exists in env '{env}' overlay in {}",
            path.display()
        );
    }
    // Only passes and products can carry `create_gift`, so only they derive a
    // twin key that a rename could collide with.
    if kind != ResourceKind::Badge {
        check_gift_collision(config, new_key, gift_key_prefix, capitalize_key, path, kind)?;
    }
    Ok(())
}

/// A gift-enabled pass/product derives a product keyed `<prefix><new_key>` at
/// resolve time (see `crate::gifts`): refuse a rename that would collide
/// with a real product under that key, in base or any env-exclusive overlay.
fn check_gift_collision(
    config: &Config,
    new_key: &str,
    gift_key_prefix: &str,
    capitalize_key: bool,
    path: &Path,
    kind: ResourceKind,
) -> Result<()> {
    let candidate = gift_key(gift_key_prefix, capitalize_key, new_key);
    if config.products.contains_key(&candidate) {
        bail!(
            "Renaming to '{new_key}' would collide with existing product '{candidate}' in {}              if this {kind} has `create_gift` enabled anywhere",
            path.display()
        );
    }
    for (env, ov) in &config.envs {
        if ov.products.contains_key(&candidate) {
            bail!(
                "Renaming to '{new_key}' would collide with product '{candidate}' in env '{env}'                  overlay in {} if this {kind} has `create_gift` enabled anywhere",
                path.display()
            );
        }
    }
    Ok(())
}

fn rename_entry_silent<V>(map: &mut BTreeMap<String, V>, old_key: &str, new_key: &str) {
    if let Some(value) = map.remove(old_key) {
        map.insert(new_key.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::config::PassConfig;
    use crate::ctx::ShopCtx;
    use crate::lockfile::{EnvLock, PassLock, ProductLock};
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn gift_enabled_pass() -> PassConfig {
        PassConfig {
            name: None,
            price: Some(499),
            description: None,
            icon: None,
            for_sale: true,
            regional_pricing: false,
            create_gift: true,
            path: None,
        }
    }

    fn pass_lock() -> PassLock {
        PassLock {
            id: 1,
            name: "VIP".into(),
            price: Some(499),
            description: None,
            icon_asset_id: None,
            icon_hash: None,
            for_sale: true,
            regional_pricing: false,
        }
    }

    fn product_lock(name: &str) -> ProductLock {
        ProductLock {
            id: 2,
            name: name.into(),
            price: 499,
            description: None,
            icon_asset_id: None,
            icon_hash: None,
            for_sale: true,
            regional_pricing: false,
            store_page: false,
        }
    }

    #[test]
    fn renaming_gift_enabled_pass_renames_lockfile_twin() {
        let config = Config {
            experience: None,
            owner: None,
            codegen: Default::default(),
            icons: Default::default(),
            gifts: Default::default(),
            include: Default::default(),
            passes: BTreeMap::from([("VIP".to_string(), gift_enabled_pass())]),
            badges: BTreeMap::new(),
            products: BTreeMap::new(),
            envs: BTreeMap::new(),
        };

        let mut lockfile = Lockfile {
            version: crate::lockfile::LOCKFILE_VERSION,
            envs: BTreeMap::from([(
                "dev".to_string(),
                EnvLock {
                    universe_id: 1,
                    passes: BTreeMap::from([("VIP".to_string(), pass_lock())]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::from([("GiftVIP".to_string(), product_lock("[GIFT] VIP"))]),
                },
            )]),
        };

        let mut files = vec![ConfigFile {
            path: PathBuf::from("rbxshop.toml"),
            config,
        }];

        rename_across_files(
            ResourceKind::Pass,
            &mut files,
            &mut lockfile,
            "VIP",
            "vip_pass",
        )
        .unwrap();

        assert!(files[0].config.passes.contains_key("vip_pass"));
        let dev_lock = &lockfile.envs["dev"];
        assert!(dev_lock.passes.contains_key("vip_pass"));
        assert!(dev_lock.products.contains_key("Giftvip_pass"));
        assert!(!dev_lock.products.contains_key("GiftVIP"));
    }

    #[test]
    fn rename_bails_when_gift_key_collides_with_real_product() {
        let config = Config {
            experience: None,
            owner: None,
            codegen: Default::default(),
            icons: Default::default(),
            gifts: Default::default(),
            include: Default::default(),
            passes: BTreeMap::from([("VIP".to_string(), gift_enabled_pass())]),
            badges: BTreeMap::new(),
            products: BTreeMap::from([(
                "GiftNew".to_string(),
                crate::config::ProductConfig {
                    name: None,
                    price: 1,
                    description: None,
                    icon: None,
                    for_sale: true,
                    regional_pricing: false,
                    store_page: false,
                    create_gift: false,
                    path: None,
                },
            )]),
            envs: BTreeMap::new(),
        };
        let mut lockfile = Lockfile {
            version: crate::lockfile::LOCKFILE_VERSION,
            envs: BTreeMap::new(),
        };

        let mut files = vec![ConfigFile {
            path: PathBuf::from("rbxshop.toml"),
            config,
        }];
        let err = rename_across_files(ResourceKind::Pass, &mut files, &mut lockfile, "VIP", "New")
            .unwrap_err();
        assert!(err.to_string().contains("GiftNew"));
    }

    #[test]
    fn rename_bails_when_gift_key_collides_with_overlay_only_product() {
        // The colliding real product lives only in an env overlay (env-
        // exclusive), never in base: the precheck must still catch it.
        let config = Config {
            experience: None,
            owner: None,
            codegen: Default::default(),
            icons: Default::default(),
            gifts: Default::default(),
            include: Default::default(),
            passes: BTreeMap::from([("VIP".to_string(), gift_enabled_pass())]),
            badges: BTreeMap::new(),
            products: BTreeMap::new(),
            envs: BTreeMap::from([(
                "prod".to_string(),
                crate::config::EnvOverlay {
                    passes: BTreeMap::new(),
                    badges: BTreeMap::new(),
                    products: BTreeMap::from([(
                        "GiftNew".to_string(),
                        crate::config::ProductOverlay {
                            price: Some(1),
                            ..Default::default()
                        },
                    )]),
                },
            )]),
        };
        let mut lockfile = Lockfile {
            version: crate::lockfile::LOCKFILE_VERSION,
            envs: BTreeMap::new(),
        };

        let mut files = vec![ConfigFile {
            path: PathBuf::from("rbxshop.toml"),
            config,
        }];
        let err = rename_across_files(ResourceKind::Pass, &mut files, &mut lockfile, "VIP", "New")
            .unwrap_err();
        assert!(err.to_string().contains("GiftNew"));
        assert!(err.to_string().contains("prod"));
    }

    #[test]
    fn run_renames_a_pass_that_lives_in_an_included_file() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("rbxshop.toml");
        std::fs::write(
            &config_path,
            r#"
[experience]
universe_id = 1

[include]
files = ["rbxshop.passes.toml"]
"#,
        )
        .unwrap();
        let passes_path = dir.path().join("rbxshop.passes.toml");
        std::fs::write(
            &passes_path,
            r#"
[passes.VIP]
price = 499

[envs.prod.passes.VIP]
price = 999
"#,
        )
        .unwrap();

        let global = rbx_core::GlobalFlags {
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
            config: config_path.clone(),
            global: &global,
            base_url: None,
        };

        run(&ctx, ResourceKind::Pass, "VIP", "vip_pass").unwrap();

        // Renamed in place, in the file that actually owned it: the main
        // file (still just [experience] + [include]) is untouched. The old
        // key is preserved as an explicit `name` so the Roblox display name
        // doesn't change.
        let passes_content = std::fs::read_to_string(&passes_path).unwrap();
        assert!(passes_content.contains("[passes.vip_pass]"));
        assert!(passes_content.contains("[envs.prod.passes.vip_pass]"));
        assert!(passes_content.contains("name = \"VIP\""));
        assert!(!passes_content.contains("[passes.VIP]"));

        let main_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(!main_content.contains("[passes"));
    }

    #[test]
    fn run_bails_when_key_not_found_in_any_file() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("rbxshop.toml");
        std::fs::write(
            &config_path,
            r#"
[experience]
universe_id = 1
"#,
        )
        .unwrap();

        let global = rbx_core::GlobalFlags {
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
            config: config_path,
            global: &global,
            base_url: None,
        };

        let err = run(&ctx, ResourceKind::Pass, "VIP", "vip_pass").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
