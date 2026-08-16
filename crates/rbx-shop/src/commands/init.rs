use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::api::RbxClient;
use crate::config::{
    BadgeConfig, CodegenConfig, Config, Experience, IconsConfig, PassConfig, ProductConfig,
    ResourceKind,
};
use crate::ctx::ShopCtx;
use crate::gifts::detect_and_merge_gift_twins;
use crate::lockfile::{
    BadgeLock, EnvLock, Lockfile, PassLock, ProductLock, DEFAULT_ENV, LOCKFILE_NAME,
    LOCKFILE_VERSION,
};
use rbx_core::places;

pub async fn run(
    ctx: &ShopCtx<'_>,
    from_remote: bool,
    universe_id_flag: Option<u64>,
    gift_label: Option<String>,
    dry_run: bool,
) -> Result<()> {
    let config_path = &ctx.config;

    if !from_remote {
        if config_path.exists() {
            bail!(
                "{} already exists. Remove it first or use a different path with --config.",
                config_path.display()
            );
        }

        let template = Config::default_template();
        std::fs::write(config_path, template)
            .with_context(|| format!("Failed to write {}", config_path.display()))?;

        println!("{} Created {}", "✓".green(), config_path.display());
        println!(
            "Edit the file to configure your universe and resources, then run `rbx shop sync`."
        );
        return Ok(());
    }

    // --from-remote — resolve target (env or standalone universe-id).
    let (env_name, universe_id): (String, u64) = match (ctx.env(), universe_id_flag) {
        (Some(name), Some(_)) => bail!(
            "Cannot pass both --env {} and --universe-id. Pick one: --env for multi-env mode, --universe-id for standalone.",
            name
        ),
        (Some(name), None) => {
            let id = places::resolve_universe_id(ctx.places_path(), name)?;
            (name.to_string(), id)
        }
        (None, Some(id)) => (DEFAULT_ENV.to_string(), id),
        (None, None) => bail!(
            "--from-remote requires either --universe-id (standalone) or --env <name> (multi-env via rbxplace.toml)."
        ),
    };

    let client = RbxClient::new(ctx.api_key(), universe_id, true);
    let config_dir = config_path.parent().unwrap_or(Path::new("."));
    let icons_config = IconsConfig::default();

    println!("Fetching remote resources from universe {}...", universe_id);

    let remote_passes = client.list_all_game_passes().await?;
    let remote_badges = client.list_all_badges(universe_id).await?;
    let remote_products = client.list_all_developer_products().await?;

    let mut passes = std::collections::BTreeMap::new();
    let mut pass_locks = std::collections::BTreeMap::new();
    for pass in &remote_passes {
        let name = pass.name.as_deref().unwrap_or("unnamed");
        let id = pass.id.unwrap_or(0);

        let key = if passes.contains_key(name) {
            let kept = pass_locks.get(name).map(|l: &PassLock| l.id);
            match crate::collision::resolve_duplicate(ResourceKind::Pass, name, id, kept, &|k| {
                passes.contains_key(k)
            }) {
                Some(chosen) => chosen,
                None => continue,
            }
        } else {
            name.to_string()
        };

        let icon_asset_id = pass.icon_asset_id;
        let (icon_path, icon_hash) = download_icon(
            &client,
            &icons_config,
            config_dir,
            ResourceKind::Pass,
            id,
            name,
            &icon_asset_id,
            dry_run,
        )
        .await?;

        let is_for_sale = pass.is_for_sale.unwrap_or(true);
        passes.insert(
            key.clone(),
            PassConfig {
                // `None` means "the key is the display name", which is only
                // true while they match. For a resource filed under another
                // key — a duplicate name the developer disambiguated — the
                // real name has to be written down, or the next `sync` reads
                // the key as the intended name and renames a live pass.
                name: display_name_override(&key, name),
                price: pass.price(),
                description: pass.description.clone(),
                icon: icon_path,
                for_sale: is_for_sale,
                regional_pricing: false,
                create_gift: false,
                path: None,
            },
        );
        pass_locks.insert(
            key.clone(),
            PassLock {
                id,
                name: name.to_string(),
                price: pass.price(),
                description: pass.description.clone(),
                icon_asset_id,
                icon_hash,
                for_sale: is_for_sale,
                regional_pricing: false,
            },
        );
    }

    let mut badges = std::collections::BTreeMap::new();
    let mut badge_locks = std::collections::BTreeMap::new();
    for badge in &remote_badges {
        let name = badge.name.as_deref().unwrap_or("unnamed");
        let id = badge.id.unwrap_or(0);

        let key = if badges.contains_key(name) {
            let kept = badge_locks.get(name).map(|l: &BadgeLock| l.id);
            match crate::collision::resolve_duplicate(ResourceKind::Badge, name, id, kept, &|k| {
                badges.contains_key(k)
            }) {
                Some(chosen) => chosen,
                None => continue,
            }
        } else {
            name.to_string()
        };

        let icon_asset_id = badge.icon_image_id;
        let (icon_path, icon_hash) = download_icon(
            &client,
            &icons_config,
            config_dir,
            ResourceKind::Badge,
            id,
            name,
            &icon_asset_id,
            dry_run,
        )
        .await?;

        badges.insert(
            key.clone(),
            BadgeConfig {
                name: display_name_override(&key, name),
                description: badge.description.clone(),
                icon: icon_path,
                enabled: badge.enabled.unwrap_or(true),
                path: None,
            },
        );
        badge_locks.insert(
            key.clone(),
            BadgeLock {
                id,
                name: name.to_string(),
                description: badge.description.clone(),
                enabled: badge.enabled.unwrap_or(true),
                icon_asset_id,
                icon_hash,
            },
        );
    }

    let mut products = std::collections::BTreeMap::new();
    let mut product_locks = std::collections::BTreeMap::new();
    for product in &remote_products {
        let name = product.name.as_deref().unwrap_or("unnamed");
        let id = product.id.unwrap_or(0);

        let key = if products.contains_key(name) {
            let kept = product_locks.get(name).map(|l: &ProductLock| l.id);
            match crate::collision::resolve_duplicate(ResourceKind::Product, name, id, kept, &|k| {
                products.contains_key(k)
            }) {
                Some(chosen) => chosen,
                None => continue,
            }
        } else {
            name.to_string()
        };

        let icon_asset_id = product.icon_image_asset_id;
        let (icon_path, icon_hash) = download_icon(
            &client,
            &icons_config,
            config_dir,
            ResourceKind::Product,
            id,
            name,
            &icon_asset_id,
            dry_run,
        )
        .await?;

        let is_for_sale = product.is_for_sale.unwrap_or(true);
        let store_page = product.store_page_enabled.unwrap_or(false);
        products.insert(
            key.clone(),
            ProductConfig {
                name: display_name_override(&key, name),
                price: product.price().unwrap_or(0),
                description: product.description.clone(),
                icon: icon_path,
                for_sale: is_for_sale,
                regional_pricing: false,
                store_page,
                create_gift: false,
                path: None,
            },
        );
        product_locks.insert(
            key.clone(),
            ProductLock {
                id,
                name: name.to_string(),
                price: product.price().unwrap_or(0),
                description: product.description.clone(),
                icon_asset_id,
                icon_hash,
                for_sale: is_for_sale,
                regional_pricing: false,
                store_page,
            },
        );
    }

    // Fold pre-existing, manually-created gift-twin products into the
    // `create_gift` convention before writing anything, so the config never
    // contains both a `create_gift = true` source and a separate literal
    // entry for what is really its twin.
    if let Some(label) = &gift_label {
        let report = detect_and_merge_gift_twins(
            &mut passes,
            &mut products,
            &mut product_locks,
            label,
            crate::gifts::DEFAULT_GIFT_KEY_PREFIX,
            false,
        );
        for merge in &report.merges {
            println!(
                "{} Detected gift twin: {} '{}' <- product '{}' (now `create_gift = true`, tracked as '{}')",
                "✓".green(),
                merge.source_kind,
                merge.source_key,
                merge.twin_key,
                merge.gift_lock_key
            );
        }
        for mismatch in &report.mismatches {
            println!(
                "{} Product '{}' looks like a gift twin of '{}' but its price differs \
                 ({} vs {}) — left as a separate entry, review manually.",
                "!".yellow(),
                mismatch.twin_key,
                mismatch.source_key,
                mismatch.twin_price,
                mismatch.source_price
            );
        }
        if report.merges.is_empty() && report.mismatches.is_empty() {
            println!(
                "{} No products matching the '{}' gift-label convention found.",
                "ℹ".blue(),
                label
            );
        }
    }

    // Build the config:
    // - Standalone mode (default env, --universe-id was used): include [experience] with universe_id
    // - Multi-env (--env <name> was used): omit [experience] entirely
    let standalone = env_name == DEFAULT_ENV;
    let experience = if standalone {
        Some(Experience { universe_id })
    } else {
        None
    };

    let config = Config {
        experience,
        owner: Some(rbx_core::owner::Owner {
            kind: rbx_core::owner::OwnerType::User,
            id: 0,
        }),
        codegen: CodegenConfig::default(),
        icons: icons_config,
        gifts: Default::default(),
        include: Default::default(),
        passes,
        badges,
        products,
        envs: std::collections::BTreeMap::new(),
    };

    if dry_run {
        println!(
            "\n{} Dry run — would create {} with {} passes, {} badges, {} products (env: {}). \
             No files written.",
            "ℹ".blue(),
            config_path.display(),
            config.passes.len(),
            config.badges.len(),
            config.products.len(),
            env_name
        );
        return Ok(());
    }

    config.save(config_path)?;

    let lockfile_path = config_dir.join(LOCKFILE_NAME);
    let mut envs_map = std::collections::BTreeMap::new();
    envs_map.insert(
        env_name.clone(),
        EnvLock {
            universe_id,
            passes: pass_locks,
            badges: badge_locks,
            products: product_locks,
        },
    );
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: envs_map,
    };
    lockfile.save(&lockfile_path)?;

    println!(
        "{} Created {} with {} passes, {} badges, {} products",
        "✓".green(),
        config_path.display(),
        config.passes.len(),
        config.badges.len(),
        config.products.len(),
    );
    println!(
        "{} Created {} (env: {})",
        "✓".green(),
        lockfile_path.display(),
        env_name
    );

    if !standalone {
        println!(
            "{} Multi-env mode initialised on '{}'. Run `rbx shop pull --env <other>` \
             to layer additional envs (overlays will be written automatically when remote diverges).",
            "ℹ".blue(),
            env_name
        );
    } else {
        println!(
            "{} Standalone mode. Fill in [creator] in the config before running sync if you plan to create badges.",
            "ℹ".blue()
        );
    }

    Ok(())
}

/// Download an icon during init --from-remote.
/// Returns (relative icon path for config, icon hash for lockfile).
#[allow(clippy::too_many_arguments)]
async fn download_icon(
    client: &RbxClient,
    icons_config: &IconsConfig,
    config_dir: &Path,
    kind: ResourceKind,
    resource_id: u64,
    name: &str,
    icon_asset_id: &Option<u64>,
    dry_run: bool,
) -> Result<(Option<PathBuf>, Option<String>)> {
    let Some(&asset_id) = icon_asset_id.as_ref() else {
        return Ok((None, None));
    };

    if dry_run {
        return Ok((None, None));
    }

    let relative_str = format!(
        "{}/{}-{}-{}.png",
        icons_config.dir.display(),
        kind,
        resource_id,
        name
    );
    let relative = PathBuf::from(&relative_str);
    let full_path = config_dir.join(&relative);

    println!("  {} Downloading {} '{}' icon...", "↓".cyan(), kind, name);

    let bytes = client.download_asset(asset_id).await?;
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full_path, &bytes)?;
    let hash = rbx_core::image::hash_bytes(&bytes);

    println!("  {} Saved to {}", "✓".green(), relative.display());

    Ok((Some(relative), Some(hash)))
}

/// What to write into a resource's `name` field.
///
/// `None` is the common case and means "the config key is the display name",
/// which keeps a generated `rbxshop.toml` free of a line repeating its own
/// table header. It stops being true the moment a resource is filed under a
/// key it did not pick — a duplicate name the developer disambiguated — and
/// then the real name has to be recorded, or the next `sync` treats the key as
/// the intended name and renames the live resource.
fn display_name_override(key: &str, name: &str) -> Option<String> {
    (key != name).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// `None` means "the key is the display name". The case below is the one
    /// that would otherwise rename a live resource: a second "VIP" filed under
    /// `VIP_2` needs its real name recorded, or the next `sync` reads the key
    /// as the name it should have.
    #[test]
    fn a_key_matching_the_name_needs_no_override() {
        assert_eq!(display_name_override("VIP", "VIP"), None);
    }

    #[test]
    fn a_disambiguated_key_records_the_real_name() {
        assert_eq!(
            display_name_override("VIP_2", "VIP"),
            Some("VIP".to_string())
        );
    }

    #[tokio::test]
    async fn download_icon_dry_run_never_touches_the_network() {
        // dry_run must short-circuit before `client` is used at all — a
        // client with no api key would error on any real network call, so
        // this only passes if the short-circuit actually happens first.
        let client = RbxClient::new(None, 0, false);
        let (path, hash) = download_icon(
            &client,
            &IconsConfig::default(),
            Path::new("."),
            ResourceKind::Pass,
            1,
            "VIP",
            &Some(999),
            true,
        )
        .await
        .unwrap();
        assert!(path.is_none());
        assert!(hash.is_none());
    }

    #[tokio::test]
    async fn download_icon_dry_run_with_no_remote_icon_is_also_none() {
        let client = RbxClient::new(None, 0, false);
        let (path, hash) = download_icon(
            &client,
            &IconsConfig::default(),
            Path::new("."),
            ResourceKind::Pass,
            1,
            "VIP",
            &None,
            true,
        )
        .await
        .unwrap();
        assert!(path.is_none());
        assert!(hash.is_none());
    }
}
