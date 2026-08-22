use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colored::Colorize;
use reqwest::StatusCode;

use crate::api::RbxClient;
use crate::config::{
    find_overlay_owner, find_owner, BadgeConfig, BadgeOverlay, Config, ConfigFile, EnvOverlay,
    PassConfig, PassOverlay, ProductConfig, ProductOverlay, ResourceKind,
};
use crate::ctx::ShopCtx;
use crate::gifts::is_gift_key;
use crate::lockfile::{
    BadgeLock, EnvLock, Lockfile, PassLock, ProductLock, DEFAULT_ENV, LOCKFILE_NAME,
    LOCKFILE_VERSION,
};
use rbx_core::api::is_api_status;
use rbx_core::confirm::confirm_always;
use rbx_core::EnvTarget;

struct IconConflict {
    env: String,
    kind: ResourceKind,
    name: String,
    local_path: String,
    local_hash: String,
    remote_asset_id: String,
}

/// An icon queued for download by `--accept-remote`.
///
/// `kind` was a `&'static str` even though `ResourceKind` already existed and
/// was used in the same expression, which meant every consumer needed a `_`
/// arm that silently did nothing.
struct PendingDownload {
    env: String,
    kind: ResourceKind,
    name: String,
    asset_id: u64,
    save_path: PathBuf,
}

pub async fn run(
    ctx: &ShopCtx<'_>,
    dry_run: bool,
    accept_remote: bool,
    accept_local: bool,
    yes: bool,
) -> Result<()> {
    let mut files = Config::load_all(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new("."));
    let lockfile_path = config_dir.join(LOCKFILE_NAME);

    let old_lockfile = Lockfile::load(&lockfile_path)?;
    let mut new_lockfile = old_lockfile.clone();
    new_lockfile.version = LOCKFILE_VERSION;

    let envs = ctx.resolve_envs(&files[0].config)?;

    let mut all_conflicts = Vec::new();
    let mut all_downloads = Vec::new();
    let mut had_any_changes = false;

    for env_target in &envs {
        if envs.len() > 1 {
            println!("\n{} {}", "env:".bold(), env_target.name.bold());
        }
        // Recomputed every iteration so a `--env all` run sees mutations
        // applied by a prior env in the same loop (matches the old
        // single-`Config` behavior, where all envs shared one mutable view).
        let merged = Config::merge_loaded(&files)?;
        let changed = pull_one_env(
            ctx,
            &merged,
            &mut files,
            config_dir,
            &old_lockfile,
            &mut new_lockfile,
            env_target,
            accept_remote,
            accept_local,
            dry_run,
            &mut all_conflicts,
            &mut all_downloads,
        )
        .await?;
        had_any_changes |= changed;
    }

    // Icon-only differences don't show up in the config/lockfile diffs, but
    // the queued downloads and conflicts below still rewrite local files:
    // count them as changes so dry-run reporting and the confirmation gate
    // see them.
    had_any_changes |= !all_conflicts.is_empty() || !all_downloads.is_empty();

    if dry_run {
        if !had_any_changes {
            println!("{} Already up to date with remote (all envs).", "✓".green());
        } else {
            println!("\nDry run: no changes applied.");
        }
        return Ok(());
    }

    // Report icon conflicts before asking for confirmation: they abort the
    // pull, so prompting first would make the user confirm a no-op.
    if !all_conflicts.is_empty() {
        println!();
        for c in &all_conflicts {
            println!(
                "{} [{}] {} '{}': icon differs from remote",
                "!".yellow(),
                c.env,
                c.kind,
                c.name.bold()
            );
            println!(
                "  Local:  {} (blake3: {}...)",
                c.local_path,
                &c.local_hash[..12.min(c.local_hash.len())]
            );
            println!("  Remote: asset {}", c.remote_asset_id);
        }
        println!();
        bail!(
            "Icon conflicts detected.\n  \
             Use --accept-remote to keep remote icons\n  \
             Use --accept-local to re-upload local icons on next sync"
        );
    }

    // Confirm before any local write (icon downloads, config rewrite,
    // lockfile rewrite). Only ask when there's something to apply; pure
    // already-up-to-date pulls fall through silently.
    if had_any_changes {
        let env_list: String = envs
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        confirm_always(
            &format!(
                "Overwrite local rbxshop.toml and lockfile for env(s) [{}]?",
                env_list
            ),
            yes,
        )?;
    }

    // Download remote icons (--accept-remote)
    let client_cache = build_client_cache(ctx, &envs, files[0].config.icons.bleed);
    for dl in &all_downloads {
        println!(
            "  {} [{}] Downloading {} '{}' icon...",
            "↓".cyan(),
            dl.env,
            dl.kind,
            dl.name
        );
        let client = client_cache
            .get(&dl.env)
            .expect("client should exist for env");
        let bytes = client.download_icon(dl.asset_id).await?;
        if let Some(parent) = dl.save_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dl.save_path, &bytes)?;
        let hash = rbx_core::image::hash_bytes(&bytes);

        let relative_icon = dl
            .save_path
            .strip_prefix(config_dir)
            .unwrap_or(&dl.save_path)
            .to_path_buf();

        // Persist the downloaded icon hash into the lockfile for this env.
        if let Some(env_lock) = new_lockfile.envs.get_mut(&dl.env) {
            if let Some(slot) = lock_icon_hash_mut(env_lock, dl.kind, &dl.name) {
                *slot = Some(hash.clone());
            }
        }

        // Persist the icon path wherever the resource currently lives: base
        // if env is default and the resource lives in base; otherwise the
        // env overlay wherever it lives, falling back to the base's file
        // when no overlay exists yet for this env.
        persist_icon_path(&mut files, dl.kind, &dl.env, &dl.name, &relative_icon);

        println!("  {} Saved to {}", "✓".green(), dl.save_path.display());
    }

    new_lockfile.save(&lockfile_path)?;
    // Written back through toml_edit, not reserialised: the user's comments,
    // key order, and any key rbx does not model all have to survive a pull.
    for file in &files {
        file.config.save_in_place(&file.path)?;
    }

    println!("{} Pull complete.", "✓".green());

    Ok(())
}

/// The `icon_hash` field of one locked resource, whichever kind it is.
///
/// Lives here rather than on `EnvLock` because the lockfile module is shared
/// state; this is the only caller that needs to write the field blind.
fn lock_icon_hash_mut<'a>(
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
fn persist_icon_path(
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

fn build_client_cache(
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
async fn pull_one_env(
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

// ---------------------------------------------------------------------------
// Config overlay computation
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ConfigChange {
    key: String,
    kind: ChangeKind,
}

#[derive(Debug)]
enum ChangeKind {
    AddedToBase,
    AddedToOverlay(String),
    OverlayDiverged { fields: Vec<String> },
    OverlayCleared,
    BaseUpdated { fields: Vec<String> },
}

/// Convert lock name to config name field: None if name == key (TOML convention).
fn config_name(lock_name: &str, key: &str) -> Option<String> {
    if lock_name == key {
        None
    } else {
        Some(lock_name.to_string())
    }
}

// ---------------------------------------------------------------------------
// One kind's view of the config side of a pull
// ---------------------------------------------------------------------------

/// What a resource kind supplies so that [`compute_config_changes`] and
/// [`apply_config_changes`] can run over it.
///
/// The shape they share is the whole flow: for every locked resource, decide
/// whether the remote diverges from base, and record that either on the base
/// entry (standalone `default` env) or as an `[envs.<name>.*]` overlay. That
/// flow was written out three times; only the field lists below actually
/// differed between the copies.
trait Pullable {
    type Cfg: Clone;
    type Overlay: Clone + Default;
    type Lock;

    const KIND: ResourceKind;

    fn base(config: &Config) -> &BTreeMap<String, Self::Cfg>;
    fn base_mut(config: &mut Config) -> &mut BTreeMap<String, Self::Cfg>;
    fn overlay_entries(overlay: &mut EnvOverlay) -> &mut BTreeMap<String, Self::Overlay>;
    fn overlay_entry(overlay: &EnvOverlay) -> &BTreeMap<String, Self::Overlay>;

    /// Base fields whose value differs from the remote's, by name.
    fn base_fields(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Vec<String>;
    /// Copy the remote's values onto an existing base entry.
    fn write_base(base: &mut Self::Cfg, key: &str, lock: &Self::Lock);
    /// A base entry for a resource the config did not have at all.
    fn base_from_lock(key: &str, lock: &Self::Lock) -> Self::Cfg;

    /// The overlay expressing how the remote diverges from the base entry.
    /// Empty when it does not.
    fn overlay_from_base(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Self::Overlay;
    /// A full overlay for a resource with no base entry to diverge from.
    fn overlay_from_lock(key: &str, lock: &Self::Lock) -> Self::Overlay;

    /// Whether the overlay carries nothing at all, including the fields pull
    /// never writes, since an overlay the user hand-wrote still counts.
    fn overlay_is_empty(overlay: &Self::Overlay) -> bool;
    /// Whether the overlay already on disk says something different from the
    /// one just computed, across the fields pull manages.
    fn overlay_diverges(current: &Self::Overlay, computed: &Self::Overlay) -> bool;
    /// The managed fields the computed overlay actually sets, for reporting.
    fn overlay_fields(overlay: &Self::Overlay) -> Vec<String>;

    /// Keys that are derived at resolve time and must never be written back.
    /// Only products have any (gift twins).
    fn is_derived(_config: &Config, _overlay: Option<&EnvOverlay>, _key: &str) -> bool {
        false
    }
}

fn compute_config_changes<K: Pullable>(
    config: &Config,
    env: &str,
    env_is_default: bool,
    locks: &BTreeMap<String, K::Lock>,
) -> Vec<ConfigChange> {
    let mut out = Vec::new();
    let overlay = config.envs.get(env);

    for (key, lock) in locks {
        if K::is_derived(config, overlay, key) {
            continue;
        }

        let Some(base) = K::base(config).get(key) else {
            out.push(ConfigChange {
                key: key.clone(),
                kind: if env_is_default {
                    ChangeKind::AddedToBase
                } else {
                    ChangeKind::AddedToOverlay(env.to_string())
                },
            });
            continue;
        };

        if env_is_default {
            let fields = K::base_fields(base, key, lock);
            if !fields.is_empty() {
                out.push(ConfigChange {
                    key: key.clone(),
                    kind: ChangeKind::BaseUpdated { fields },
                });
            }
            continue;
        }

        let computed = K::overlay_from_base(base, key, lock);
        let current = overlay
            .map(K::overlay_entry)
            .and_then(|entries| entries.get(key))
            .cloned()
            .unwrap_or_default();

        if K::overlay_is_empty(&computed) {
            // The env caught up with base, so the overlay that used to record
            // the divergence is now noise.
            if !K::overlay_is_empty(&current) {
                out.push(ConfigChange {
                    key: key.clone(),
                    kind: ChangeKind::OverlayCleared,
                });
            }
        } else if K::overlay_diverges(&current, &computed) {
            out.push(ConfigChange {
                key: key.clone(),
                kind: ChangeKind::OverlayDiverged {
                    fields: K::overlay_fields(&computed),
                },
            });
        }
    }

    out
}

fn apply_config_changes<K: Pullable>(
    files: &mut [ConfigFile],
    merged: &Config,
    env: &str,
    env_is_default: bool,
    locks: &BTreeMap<String, K::Lock>,
) {
    let overlay = merged.envs.get(env).cloned();

    for (key, lock) in locks {
        if K::is_derived(merged, overlay.as_ref(), key) {
            continue;
        }

        match find_owner(files, K::KIND, key) {
            // The resource already has a base entry: update it in place, or
            // record the divergence as this env's overlay.
            Some(idx) => {
                if env_is_default {
                    let base = K::base_mut(&mut files[idx].config)
                        .get_mut(key)
                        .expect("find_owner just said this file declares the key");
                    K::write_base(base, key, lock);
                } else {
                    let base = K::base(&files[idx].config)[key].clone();
                    let computed = K::overlay_from_base(&base, key, lock);
                    // A new overlay is co-located with the base it overrides,
                    // rather than always landing in the main file.
                    let owner = find_overlay_owner(files, K::KIND, env, key).unwrap_or(idx);
                    let entries = K::overlay_entries(
                        files[owner].config.envs.entry(env.to_string()).or_default(),
                    );
                    if K::overlay_is_empty(&computed) {
                        entries.remove(key);
                    } else {
                        entries.insert(key.clone(), computed);
                    }
                }
            }
            // Newly discovered remotely. With no base entry to diverge from,
            // the whole resource is written: to the main file for the
            // standalone env, or as a complete overlay for a named one.
            None => {
                if env_is_default {
                    K::base_mut(&mut files[0].config)
                        .insert(key.clone(), K::base_from_lock(key, lock));
                } else {
                    let owner = find_overlay_owner(files, K::KIND, env, key).unwrap_or(0);
                    K::overlay_entries(
                        files[owner].config.envs.entry(env.to_string()).or_default(),
                    )
                    .insert(key.clone(), K::overlay_from_lock(key, lock));
                }
            }
        }
    }
}

/// `Some(field)` when the remote value differs from the base one, ready to
/// drop straight into an overlay field.
fn diverged<T: Clone + PartialEq>(base: &T, remote: &T) -> Option<T> {
    (base != remote).then(|| remote.clone())
}

/// Names of the fields a `Vec<(name, is_set)>` says are set, in order.
fn set_fields(fields: &[(&str, bool)]) -> Vec<String> {
    fields
        .iter()
        .filter(|(_, set)| *set)
        .map(|(name, _)| (*name).to_string())
        .collect()
}

// ---- Passes ---------------------------------------------------------------

struct PassKind;

impl Pullable for PassKind {
    type Cfg = PassConfig;
    type Overlay = PassOverlay;
    type Lock = PassLock;

    const KIND: ResourceKind = ResourceKind::Pass;

    fn base(config: &Config) -> &BTreeMap<String, Self::Cfg> {
        &config.passes
    }
    fn base_mut(config: &mut Config) -> &mut BTreeMap<String, Self::Cfg> {
        &mut config.passes
    }
    fn overlay_entries(overlay: &mut EnvOverlay) -> &mut BTreeMap<String, Self::Overlay> {
        &mut overlay.passes
    }
    fn overlay_entry(overlay: &EnvOverlay) -> &BTreeMap<String, Self::Overlay> {
        &overlay.passes
    }

    fn base_fields(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Vec<String> {
        set_fields(&[
            ("name", base.name != config_name(&lock.name, key)),
            ("price", base.price != lock.price),
            ("description", base.description != lock.description),
            ("for_sale", base.for_sale != lock.for_sale),
        ])
    }

    fn write_base(base: &mut Self::Cfg, key: &str, lock: &Self::Lock) {
        base.name = config_name(&lock.name, key);
        base.price = lock.price;
        base.description = lock.description.clone();
        base.for_sale = lock.for_sale;
    }

    fn base_from_lock(key: &str, lock: &Self::Lock) -> Self::Cfg {
        PassConfig {
            name: config_name(&lock.name, key),
            price: lock.price,
            description: lock.description.clone(),
            icon: None,
            for_sale: lock.for_sale,
            regional_pricing: false,
            create_gift: false,
            path: None,
        }
    }

    fn overlay_from_base(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Self::Overlay {
        PassOverlay {
            name: diverged(&base.name, &config_name(&lock.name, key)).flatten(),
            price: diverged(&base.price, &lock.price).flatten(),
            description: diverged(&base.description, &lock.description).flatten(),
            for_sale: diverged(&base.for_sale, &lock.for_sale),
            ..Default::default()
        }
    }

    fn overlay_from_lock(key: &str, lock: &Self::Lock) -> Self::Overlay {
        PassOverlay {
            name: config_name(&lock.name, key),
            price: lock.price,
            description: lock.description.clone(),
            for_sale: Some(lock.for_sale),
            ..Default::default()
        }
    }

    fn overlay_is_empty(ov: &Self::Overlay) -> bool {
        ov.name.is_none()
            && ov.price.is_none()
            && ov.description.is_none()
            && ov.icon.is_none()
            && ov.for_sale.is_none()
            && ov.regional_pricing.is_none()
            && ov.path.is_none()
    }

    fn overlay_diverges(current: &Self::Overlay, computed: &Self::Overlay) -> bool {
        current.name != computed.name
            || current.price != computed.price
            || current.description != computed.description
            || current.for_sale != computed.for_sale
    }

    fn overlay_fields(ov: &Self::Overlay) -> Vec<String> {
        set_fields(&[
            ("name", ov.name.is_some()),
            ("price", ov.price.is_some()),
            ("description", ov.description.is_some()),
            ("for_sale", ov.for_sale.is_some()),
        ])
    }
}

// ---- Badges ---------------------------------------------------------------

struct BadgeKind;

impl Pullable for BadgeKind {
    type Cfg = BadgeConfig;
    type Overlay = BadgeOverlay;
    type Lock = BadgeLock;

    const KIND: ResourceKind = ResourceKind::Badge;

    fn base(config: &Config) -> &BTreeMap<String, Self::Cfg> {
        &config.badges
    }
    fn base_mut(config: &mut Config) -> &mut BTreeMap<String, Self::Cfg> {
        &mut config.badges
    }
    fn overlay_entries(overlay: &mut EnvOverlay) -> &mut BTreeMap<String, Self::Overlay> {
        &mut overlay.badges
    }
    fn overlay_entry(overlay: &EnvOverlay) -> &BTreeMap<String, Self::Overlay> {
        &overlay.badges
    }

    fn base_fields(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Vec<String> {
        set_fields(&[
            ("name", base.name != config_name(&lock.name, key)),
            ("description", base.description != lock.description),
            ("enabled", base.enabled != lock.enabled),
        ])
    }

    fn write_base(base: &mut Self::Cfg, key: &str, lock: &Self::Lock) {
        base.name = config_name(&lock.name, key);
        base.description = lock.description.clone();
        base.enabled = lock.enabled;
    }

    fn base_from_lock(key: &str, lock: &Self::Lock) -> Self::Cfg {
        BadgeConfig {
            name: config_name(&lock.name, key),
            description: lock.description.clone(),
            icon: None,
            enabled: lock.enabled,
            path: None,
        }
    }

    fn overlay_from_base(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Self::Overlay {
        BadgeOverlay {
            name: diverged(&base.name, &config_name(&lock.name, key)).flatten(),
            description: diverged(&base.description, &lock.description).flatten(),
            enabled: diverged(&base.enabled, &lock.enabled),
            ..Default::default()
        }
    }

    fn overlay_from_lock(key: &str, lock: &Self::Lock) -> Self::Overlay {
        BadgeOverlay {
            name: config_name(&lock.name, key),
            description: lock.description.clone(),
            enabled: Some(lock.enabled),
            ..Default::default()
        }
    }

    fn overlay_is_empty(ov: &Self::Overlay) -> bool {
        ov.name.is_none()
            && ov.description.is_none()
            && ov.icon.is_none()
            && ov.enabled.is_none()
            && ov.path.is_none()
    }

    fn overlay_diverges(current: &Self::Overlay, computed: &Self::Overlay) -> bool {
        current.name != computed.name
            || current.description != computed.description
            || current.enabled != computed.enabled
    }

    fn overlay_fields(ov: &Self::Overlay) -> Vec<String> {
        set_fields(&[
            ("name", ov.name.is_some()),
            ("description", ov.description.is_some()),
            ("enabled", ov.enabled.is_some()),
        ])
    }
}

// ---- Products -------------------------------------------------------------

struct ProductKind;

impl Pullable for ProductKind {
    type Cfg = ProductConfig;
    type Overlay = ProductOverlay;
    type Lock = ProductLock;

    const KIND: ResourceKind = ResourceKind::Product;

    fn base(config: &Config) -> &BTreeMap<String, Self::Cfg> {
        &config.products
    }
    fn base_mut(config: &mut Config) -> &mut BTreeMap<String, Self::Cfg> {
        &mut config.products
    }
    fn overlay_entries(overlay: &mut EnvOverlay) -> &mut BTreeMap<String, Self::Overlay> {
        &mut overlay.products
    }
    fn overlay_entry(overlay: &EnvOverlay) -> &BTreeMap<String, Self::Overlay> {
        &overlay.products
    }

    /// Gift twins are derived from their source at resolve time (see
    /// `crate::gifts`) and never live in rbxshop.toml: writing one back would
    /// turn the remote twin into a real `[products.*]` entry, which the next
    /// resolve would then collide with.
    fn is_derived(config: &Config, overlay: Option<&EnvOverlay>, key: &str) -> bool {
        is_gift_key(config, overlay, key)
    }

    fn base_fields(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Vec<String> {
        set_fields(&[
            ("name", base.name != config_name(&lock.name, key)),
            ("price", base.price != lock.price),
            ("description", base.description != lock.description),
            ("for_sale", base.for_sale != lock.for_sale),
            ("store_page", base.store_page != lock.store_page),
        ])
    }

    fn write_base(base: &mut Self::Cfg, key: &str, lock: &Self::Lock) {
        base.name = config_name(&lock.name, key);
        base.price = lock.price;
        base.description = lock.description.clone();
        base.for_sale = lock.for_sale;
        base.store_page = lock.store_page;
    }

    fn base_from_lock(key: &str, lock: &Self::Lock) -> Self::Cfg {
        ProductConfig {
            name: config_name(&lock.name, key),
            price: lock.price,
            description: lock.description.clone(),
            icon: None,
            for_sale: lock.for_sale,
            regional_pricing: false,
            store_page: lock.store_page,
            create_gift: false,
            path: None,
        }
    }

    fn overlay_from_base(base: &Self::Cfg, key: &str, lock: &Self::Lock) -> Self::Overlay {
        ProductOverlay {
            name: diverged(&base.name, &config_name(&lock.name, key)).flatten(),
            price: diverged(&base.price, &lock.price),
            description: diverged(&base.description, &lock.description).flatten(),
            for_sale: diverged(&base.for_sale, &lock.for_sale),
            store_page: diverged(&base.store_page, &lock.store_page),
            ..Default::default()
        }
    }

    fn overlay_from_lock(key: &str, lock: &Self::Lock) -> Self::Overlay {
        ProductOverlay {
            name: config_name(&lock.name, key),
            price: Some(lock.price),
            description: lock.description.clone(),
            for_sale: Some(lock.for_sale),
            store_page: Some(lock.store_page),
            ..Default::default()
        }
    }

    fn overlay_is_empty(ov: &Self::Overlay) -> bool {
        ov.name.is_none()
            && ov.price.is_none()
            && ov.description.is_none()
            && ov.icon.is_none()
            && ov.for_sale.is_none()
            && ov.regional_pricing.is_none()
            && ov.store_page.is_none()
            && ov.path.is_none()
    }

    fn overlay_diverges(current: &Self::Overlay, computed: &Self::Overlay) -> bool {
        current.name != computed.name
            || current.price != computed.price
            || current.description != computed.description
            || current.for_sale != computed.for_sale
            || current.store_page != computed.store_page
    }

    fn overlay_fields(ov: &Self::Overlay) -> Vec<String> {
        set_fields(&[
            ("name", ov.name.is_some()),
            ("price", ov.price.is_some()),
            ("description", ov.description.is_some()),
            ("for_sale", ov.for_sale.is_some()),
            ("store_page", ov.store_page.is_some()),
        ])
    }
}

fn cleanup_empty_overlays(files: &mut [ConfigFile], env: &str) {
    for file in files.iter_mut() {
        if let Some(ov) = file.config.envs.get(env) {
            if ov.is_empty() {
                file.config.envs.remove(env);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Printing
// ---------------------------------------------------------------------------

fn print_config_changes(kind: ResourceKind, changes: &[ConfigChange]) {
    for change in changes {
        match &change.kind {
            ChangeKind::AddedToBase => println!(
                "  {} {} {} {} in base config",
                "+".green(),
                "add".green(),
                kind,
                change.key.bold()
            ),
            ChangeKind::AddedToOverlay(env) => println!(
                "  {} {} {} {} in [envs.{}] overlay",
                "+".green(),
                "add".green(),
                kind,
                change.key.bold(),
                env
            ),
            ChangeKind::BaseUpdated { fields } => println!(
                "  {} {} {} {} in base config ({})",
                "~".yellow(),
                "update".yellow(),
                kind,
                change.key.bold(),
                fields.join(", ")
            ),
            ChangeKind::OverlayDiverged { fields } => println!(
                "  {} {} {} {} in overlay ({})",
                "~".yellow(),
                "update".yellow(),
                kind,
                change.key.bold(),
                fields.join(", ")
            ),
            ChangeKind::OverlayCleared => println!(
                "  {} {} {} {} (overlay no longer needed)",
                "-".dimmed(),
                "clear".dimmed(),
                kind,
                change.key.dimmed()
            ),
        }
    }
}

struct LockfileDiff {
    new_passes: Vec<String>,
    new_badges: Vec<String>,
    new_products: Vec<String>,
    removed_passes: Vec<String>,
    removed_badges: Vec<String>,
    removed_products: Vec<String>,
}

impl LockfileDiff {
    fn is_empty(&self) -> bool {
        self.new_passes.is_empty()
            && self.new_badges.is_empty()
            && self.new_products.is_empty()
            && self.removed_passes.is_empty()
            && self.removed_badges.is_empty()
            && self.removed_products.is_empty()
    }
}

fn lockfile_diff(
    old: &EnvLock,
    pass_locks: &BTreeMap<String, PassLock>,
    badge_locks: &BTreeMap<String, BadgeLock>,
    product_locks: &BTreeMap<String, ProductLock>,
) -> LockfileDiff {
    let mut diff = LockfileDiff {
        new_passes: Vec::new(),
        new_badges: Vec::new(),
        new_products: Vec::new(),
        removed_passes: Vec::new(),
        removed_badges: Vec::new(),
        removed_products: Vec::new(),
    };
    for k in pass_locks.keys() {
        if !old.passes.contains_key(k) {
            diff.new_passes.push(k.clone());
        }
    }
    for k in old.passes.keys() {
        if !pass_locks.contains_key(k) {
            diff.removed_passes.push(k.clone());
        }
    }
    for k in badge_locks.keys() {
        if !old.badges.contains_key(k) {
            diff.new_badges.push(k.clone());
        }
    }
    for k in old.badges.keys() {
        if !badge_locks.contains_key(k) {
            diff.removed_badges.push(k.clone());
        }
    }
    for k in product_locks.keys() {
        if !old.products.contains_key(k) {
            diff.new_products.push(k.clone());
        }
    }
    for k in old.products.keys() {
        if !product_locks.contains_key(k) {
            diff.removed_products.push(k.clone());
        }
    }
    diff
}

fn print_lockfile_diff(diff: &LockfileDiff) {
    for k in &diff.new_passes {
        println!("  {} {} pass {}", "+".green(), "new".green(), k.bold());
    }
    for k in &diff.removed_passes {
        println!("  {} {} pass {}", "-".red(), "removed".red(), k.bold());
    }
    for k in &diff.new_badges {
        println!("  {} {} badge {}", "+".green(), "new".green(), k.bold());
    }
    for k in &diff.removed_badges {
        println!("  {} {} badge {}", "-".red(), "removed".red(), k.bold());
    }
    for k in &diff.new_products {
        println!("  {} {} product {}", "+".green(), "new".green(), k.bold());
    }
    for k in &diff.removed_products {
        println!("  {} {} product {}", "-".red(), "removed".red(), k.bold());
    }
}

// ---------------------------------------------------------------------------
// Icon resolution (per-env)
// ---------------------------------------------------------------------------

enum IconResolution {
    SetNone,
    PreserveOld,
    PendingDownload,
}

#[allow(clippy::too_many_arguments)]
fn resolve_icon(
    env: &str,
    kind: ResourceKind,
    name: &str,
    resource_id: u64,
    old_icon_id: Option<&u64>,
    new_icon_id: &Option<u64>,
    local_icon: Option<&PathBuf>,
    config_dir: &Path,
    icon_dir: &Path,
    accept_remote: bool,
    accept_local: bool,
    conflicts: &mut Vec<IconConflict>,
    downloads: &mut Vec<PendingDownload>,
) -> Result<IconResolution> {
    let icon_changed = match (old_icon_id, new_icon_id.as_ref()) {
        (Some(old), Some(new)) => old != new,
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => false,
    };

    if !icon_changed {
        return Ok(IconResolution::PreserveOld);
    }

    if accept_remote {
        if let Some(&asset_id) = new_icon_id.as_ref() {
            let save_path = if let Some(local_path) = local_icon {
                config_dir.join(local_path)
            } else {
                // Same rule as `shop init`: the display name is Roblox's, and
                // Roblox allows characters Windows refuses in a filename. See
                // `rbx_core::fs_name`.
                let safe = rbx_core::fs_name::safe_component(name);
                let stem = if safe.is_empty() {
                    format!("{kind}-{resource_id}")
                } else {
                    format!("{kind}-{resource_id}-{safe}")
                };
                config_dir.join(format!("{}/{stem}.png", icon_dir.display()))
            };
            downloads.push(PendingDownload {
                env: env.to_string(),
                kind,
                name: name.to_string(),
                asset_id,
                save_path,
            });
            return Ok(IconResolution::PendingDownload);
        } else {
            return Ok(IconResolution::SetNone);
        }
    }

    if accept_local {
        return Ok(IconResolution::SetNone);
    }

    let Some(local_path) = local_icon else {
        return Ok(IconResolution::SetNone);
    };

    let full_path = config_dir.join(local_path);
    let local_hash = hash_file(&full_path)?;

    conflicts.push(IconConflict {
        env: env.to_string(),
        kind,
        name: name.to_string(),
        local_path: local_path.display().to_string(),
        local_hash,
        remote_asset_id: new_icon_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string()),
    });
    Ok(IconResolution::SetNone)
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(rbx_core::image::hash_bytes(&bytes))
}

#[cfg(test)]
mod tests;
