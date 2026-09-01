//! One env's pull, and the pieces that only it needs.
//!
//! The heart of the command: read the live catalogue, decide what the config
//! and the lockfile should say, and write both. Split out of `pull.rs` because
//! everything else there is either the run around it or a step it calls.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use reqwest::StatusCode;

use crate::api::RbxClient;
use crate::config::{find_overlay_owner, find_owner, Config, ConfigFile, ResourceKind};
use crate::ctx::ShopCtx;
use crate::lockfile::{BadgeLock, EnvLock, Lockfile, PassLock, ProductLock, DEFAULT_ENV};
use rbx_core::api::is_api_status;
use rbx_core::EnvTarget;

use super::*;

/// The `icon_hash` field of one locked resource, whichever kind it is.
///
/// Lives here rather than on `EnvLock` because the lockfile module is shared
/// state; this is the only caller that needs to write the field blind.
pub(super) fn lock_icon_hash_mut<'a>(
    env_lock: &'a mut EnvLock,
    kind: ResourceKind,
    key: &str,
) -> Option<&'a mut Option<String>> {
    match kind {
        ResourceKind::Pass => env_lock.passes.get_mut(key).map(|l| &mut l.icon_hash),
        ResourceKind::Badge => env_lock.badges.get_mut(key).map(|l| &mut l.icon_hash),
        ResourceKind::Product => env_lock.products.get_mut(key).map(|l| &mut l.icon_hash),
    }
}

/// Record a downloaded icon's path on whichever entry currently owns the
/// resource, without overwriting one the user already declared.
///
/// For the standalone `default` env that is the base entry. For a named env it
/// is that env's overlay, falling back to the base's own file when the
/// resource has no overlay there yet.
pub(super) fn persist_icon_path(
    files: &mut [ConfigFile],
    kind: ResourceKind,
    env: &str,
    key: &str,
    icon: &Path,
) {
    let slot = if env == DEFAULT_ENV {
        find_owner(files, kind, key).and_then(|idx| kind.base_icon_mut(&mut files[idx].config, key))
    } else if let Some(idx) = find_overlay_owner(files, kind, env, key) {
        kind.overlay_icon_mut(&mut files[idx].config, env, key)
    } else {
        find_owner(files, kind, key).and_then(|idx| kind.base_icon_mut(&mut files[idx].config, key))
    };

    if let Some(slot) = slot {
        if slot.is_none() {
            *slot = Some(icon.to_path_buf());
        }
    }
}

pub(super) fn build_client_cache(
    ctx: &ShopCtx<'_>,
    envs: &[EnvTarget],
    bleed: bool,
) -> HashMap<String, RbxClient> {
    let mut map = HashMap::new();
    for e in envs {
        map.insert(e.name.clone(), ctx.client(e.universe_id, bleed));
    }
    map
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn pull_one_env(
    ctx: &ShopCtx<'_>,
    merged: &Config,
    files: &mut [ConfigFile],
    config_dir: &Path,
    old_lockfile: &Lockfile,
    new_lockfile: &mut Lockfile,
    env_target: &EnvTarget,
    accept_remote: bool,
    accept_local: bool,
    dry_run: bool,
    conflicts: &mut Vec<IconConflict>,
    downloads: &mut Vec<PendingDownload>,
) -> Result<bool> {
    let client = ctx.client(env_target.universe_id, merged.icons.bleed);

    println!("Pulling remote state for env '{}'...", env_target.name);

    let default_env_lock = EnvLock {
        universe_id: env_target.universe_id,
        ..Default::default()
    };
    let old_env_lock = old_lockfile
        .env(&env_target.name)
        .unwrap_or(&default_env_lock);

    // Build ID → key indexes from existing lockfile section
    let pass_id_to_key: HashMap<u64, String> = old_env_lock
        .passes
        .iter()
        .map(|(k, v)| (v.id, k.clone()))
        .collect();
    let badge_id_to_key: HashMap<u64, String> = old_env_lock
        .badges
        .iter()
        .map(|(k, v)| (v.id, k.clone()))
        .collect();
    let product_id_to_key: HashMap<u64, String> = old_env_lock
        .products
        .iter()
        .map(|(k, v)| (v.id, k.clone()))
        .collect();

    // Fetch passes
    let remote_passes = client.list_all_game_passes().await?;
    let mut pass_locks = BTreeMap::new();
    for pass in &remote_passes {
        let display_name = pass.name.as_deref().unwrap_or("unnamed");
        let id = pass.id.unwrap_or(0);
        let key = pass_id_to_key
            .get(&id)
            .cloned()
            .unwrap_or_else(|| display_name.to_string());

        let key = if pass_locks.contains_key(&key) {
            // Only newly discovered resources can collide: anything already
            // tracked was keyed by id above, so this never displaces a
            // resource the config is managing.
            let kept = pass_locks.get(&key).map(|l: &PassLock| l.id);
            match crate::collision::resolve_duplicate(
                ResourceKind::Pass,
                display_name,
                id,
                kept,
                &|k| pass_locks.contains_key(k),
            ) {
                Some(chosen) => chosen,
                None => continue,
            }
        } else {
            key
        };

        // The list API doesn't report regional pricing, so preserve whatever the
        // lockfile last recorded (set by sync) instead of clobbering it to false:
        // otherwise a synced `regional_pricing = true` shows a phantom diff
        // after every pull.
        let prior_regional = old_env_lock
            .passes
            .get(&key)
            .map(|p| p.regional_pricing)
            .unwrap_or(false);
        pass_locks.insert(
            key,
            PassLock {
                id,
                name: display_name.to_string(),
                price: pass.price(),
                description: pass.description.clone(),
                icon_asset_id: pass.icon_asset_id,
                icon_hash: None,
                for_sale: pass.is_for_sale.unwrap_or(true),
                regional_pricing: prior_regional,
            },
        );
    }

    // Fetch badges
    let remote_badges = client.list_all_badges(env_target.universe_id).await?;
    let mut badge_locks = BTreeMap::new();
    let mut seen_badge_ids = HashSet::new();
    for badge in &remote_badges {
        let display_name = badge.name.as_deref().unwrap_or("unnamed");
        let id = badge.id.unwrap_or(0);
        seen_badge_ids.insert(id);
        let key = badge_id_to_key
            .get(&id)
            .cloned()
            .unwrap_or_else(|| display_name.to_string());

        let key = if badge_locks.contains_key(&key) {
            // Only newly discovered resources can collide: anything already
            // tracked was keyed by id above, so this never displaces a
            // resource the config is managing.
            let kept = badge_locks.get(&key).map(|l: &BadgeLock| l.id);
            match crate::collision::resolve_duplicate(
                ResourceKind::Badge,
                display_name,
                id,
                kept,
                &|k| badge_locks.contains_key(k),
            ) {
                Some(chosen) => chosen,
                None => continue,
            }
        } else {
            key
        };

        badge_locks.insert(
            key,
            BadgeLock {
                id,
                name: display_name.to_string(),
                description: badge.description.clone(),
                enabled: badge.enabled.unwrap_or(true),
                icon_asset_id: badge.icon_image_id,
                icon_hash: None,
            },
        );
    }

    // Disabled badges aren't returned by the list endpoint since Aug 2024.
    // Fetch them individually so they don't appear as "removed".
    for (key, old_lock) in &old_env_lock.badges {
        if !seen_badge_ids.contains(&old_lock.id) {
            match client.get_badge(old_lock.id).await {
                Ok(badge) => {
                    let display_name = badge.name.as_deref().unwrap_or("unnamed");
                    let id = badge.id.unwrap_or(old_lock.id);
                    badge_locks.insert(
                        key.clone(),
                        BadgeLock {
                            id,
                            name: display_name.to_string(),
                            description: badge.description.clone(),
                            enabled: badge.enabled.unwrap_or(false),
                            icon_asset_id: badge.icon_image_id,
                            icon_hash: None,
                        },
                    );
                }
                // Roblox says there is no such badge: it really is gone, and
                // letting the entry fall out of the lockfile is the point of
                // this loop. Matched on the typed status rather than on the
                // rendered message, which embeds a body free to contain "404".
                Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => {}
                // Anything else means the run never learned whether the badge
                // exists. Omitting it from the lockfile would assert that it
                // is gone, and the next `sync` would act on that by creating a
                // second badge on a live universe, which Roblox does not let
                // anyone delete. A pull that fails changes nothing and can be
                // retried; a pull that writes a lockfile short one entry
                // cannot be undone.
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "could not confirm whether badge {} ({}) still exists; \
                             refusing to write a lockfile without it, because the next \
                             sync would create a duplicate badge",
                            key, old_lock.id
                        )
                    })
                }
            }
        }
    }

    // Fetch products
    let remote_products = client.list_all_developer_products().await?;
    let mut product_locks = BTreeMap::new();
    for product in &remote_products {
        let display_name = product.name.as_deref().unwrap_or("unnamed");
        let id = product.id.unwrap_or(0);
        let key = product_id_to_key
            .get(&id)
            .cloned()
            .unwrap_or_else(|| display_name.to_string());

        let key = if product_locks.contains_key(&key) {
            // Only newly discovered resources can collide: anything already
            // tracked was keyed by id above, so this never displaces a
            // resource the config is managing.
            let kept = product_locks.get(&key).map(|l: &ProductLock| l.id);
            match crate::collision::resolve_duplicate(
                ResourceKind::Product,
                display_name,
                id,
                kept,
                &|k| product_locks.contains_key(k),
            ) {
                Some(chosen) => chosen,
                None => continue,
            }
        } else {
            key
        };

        // Preserve the lockfile's regional pricing (see the pass loop above):
        // the list API never reports it, so clobbering to false would create a
        // phantom diff against a synced `regional_pricing = true`.
        let prior_regional = old_env_lock
            .products
            .get(&key)
            .map(|p| p.regional_pricing)
            .unwrap_or(false);
        product_locks.insert(
            key,
            ProductLock {
                id,
                name: display_name.to_string(),
                price: product.price().unwrap_or(0),
                description: product.description.clone(),
                icon_asset_id: product.icon_image_asset_id,
                icon_hash: None,
                for_sale: product.is_for_sale.unwrap_or(true),
                regional_pricing: prior_regional,
                store_page: product.store_page_enabled.unwrap_or(false),
            },
        );
    }

    // Detect icon conflicts and queue downloads BEFORE mutating config: we
    // need the current config.icons.dir + per-resource icon path resolution
    // against the resolved view.
    let resources_pre = merged.resolve_env(Some(&env_target.name))?;

    for (name, new_lock) in &mut pass_locks {
        let old_icon_id = old_env_lock
            .passes
            .get(name)
            .and_then(|l| l.icon_asset_id.as_ref());
        let local_icon = resources_pre.passes.get(name).and_then(|c| c.icon.as_ref());
        match resolve_icon(
            &env_target.name,
            ResourceKind::Pass,
            name,
            new_lock.id,
            old_icon_id,
            &new_lock.icon_asset_id,
            local_icon,
            config_dir,
            &merged.icons.dir,
            accept_remote,
            accept_local,
            conflicts,
            downloads,
        )? {
            IconResolution::SetNone => new_lock.icon_hash = None,
            IconResolution::PreserveOld => {
                new_lock.icon_hash = old_env_lock
                    .passes
                    .get(name)
                    .and_then(|l| l.icon_hash.clone());
            }
            IconResolution::PendingDownload => {}
        }
    }

    for (name, new_lock) in &mut badge_locks {
        let old_icon_id = old_env_lock
            .badges
            .get(name)
            .and_then(|l| l.icon_asset_id.as_ref());
        let local_icon = resources_pre.badges.get(name).and_then(|c| c.icon.as_ref());
        match resolve_icon(
            &env_target.name,
            ResourceKind::Badge,
            name,
            new_lock.id,
            old_icon_id,
            &new_lock.icon_asset_id,
            local_icon,
            config_dir,
            &merged.icons.dir,
            accept_remote,
            accept_local,
            conflicts,
            downloads,
        )? {
            IconResolution::SetNone => new_lock.icon_hash = None,
            IconResolution::PreserveOld => {
                new_lock.icon_hash = old_env_lock
                    .badges
                    .get(name)
                    .and_then(|l| l.icon_hash.clone());
            }
            IconResolution::PendingDownload => {}
        }
    }

    for (name, new_lock) in &mut product_locks {
        let old_icon_id = old_env_lock
            .products
            .get(name)
            .and_then(|l| l.icon_asset_id.as_ref());
        let local_icon = resources_pre
            .products
            .get(name)
            .and_then(|c| c.icon.as_ref());
        match resolve_icon(
            &env_target.name,
            ResourceKind::Product,
            name,
            new_lock.id,
            old_icon_id,
            &new_lock.icon_asset_id,
            local_icon,
            config_dir,
            &merged.icons.dir,
            accept_remote,
            accept_local,
            conflicts,
            downloads,
        )? {
            IconResolution::SetNone => new_lock.icon_hash = None,
            IconResolution::PreserveOld => {
                new_lock.icon_hash = old_env_lock
                    .products
                    .get(name)
                    .and_then(|l| l.icon_hash.clone());
            }
            IconResolution::PendingDownload => {}
        }
    }

    // Compute the diff for reporting + decide which mutations to apply to config.
    let env_is_default = env_target.name == DEFAULT_ENV;
    let pass_changes =
        compute_config_changes::<PassKind>(merged, &env_target.name, env_is_default, &pass_locks);
    let badge_changes =
        compute_config_changes::<BadgeKind>(merged, &env_target.name, env_is_default, &badge_locks);
    let product_changes = compute_config_changes::<ProductKind>(
        merged,
        &env_target.name,
        env_is_default,
        &product_locks,
    );

    let lockfile_diff = lockfile_diff(old_env_lock, &pass_locks, &badge_locks, &product_locks);

    let mut had_changes = false;
    had_changes |= !pass_changes.is_empty();
    had_changes |= !badge_changes.is_empty();
    had_changes |= !product_changes.is_empty();
    had_changes |= !lockfile_diff.is_empty();

    if !had_changes {
        println!(
            "{} env '{}' is in sync with remote.",
            "✓".green(),
            env_target.name
        );
        // Still ensure the env's lockfile section exists and is fresh.
        let env_lock_mut = new_lockfile.env_mut(&env_target.name, env_target.universe_id);
        env_lock_mut.passes = pass_locks;
        env_lock_mut.badges = badge_locks;
        env_lock_mut.products = product_locks;
        return Ok(false);
    }

    print_lockfile_diff(&lockfile_diff);
    print_config_changes(ResourceKind::Pass, &pass_changes);
    print_config_changes(ResourceKind::Badge, &badge_changes);
    print_config_changes(ResourceKind::Product, &product_changes);

    if dry_run {
        return Ok(true);
    }

    // Apply config-side mutations now (only when not dry-run).
    apply_config_changes::<PassKind>(files, merged, &env_target.name, env_is_default, &pass_locks);
    apply_config_changes::<BadgeKind>(
        files,
        merged,
        &env_target.name,
        env_is_default,
        &badge_locks,
    );
    apply_config_changes::<ProductKind>(
        files,
        merged,
        &env_target.name,
        env_is_default,
        &product_locks,
    );

    // Drop any empty overlay sections to keep TOML clean.
    cleanup_empty_overlays(files, &env_target.name);

    // Update lockfile with new state.
    let env_lock = new_lockfile.env_mut(&env_target.name, env_target.universe_id);
    env_lock.passes = pass_locks;
    env_lock.badges = badge_locks;
    env_lock.products = product_locks;

    Ok(true)
}
