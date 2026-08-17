//! Apply the plan: create and update passes, badges and products on Roblox,
//! recording each result in the lockfile as it lands.
//!
//! `apply_pass_actions` and `apply_product_actions` used to be two ~108-line
//! functions that were 0.94 identical after renaming their identifiers, with
//! `apply_badge_actions` a third variation on the same theme. The shape they
//! share — resolve the display name, hash the icon, call the API, write the
//! entry back, save — now lives once in `apply_kind`, and each kind supplies
//! only its own two API calls (see `Appliable`).
//!
//! The lockfile is saved after every single resource, not once at the end.
//! That is deliberate: these calls create paid products, and a run that dies
//! halfway must not lose the ids of what it already made.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use crate::codegen;
use crate::config::{
    resolve_name, BadgeConfig, Config, PassConfig, ProductConfig, ResolvedResources, ResourceKind,
};
use crate::ctx::ShopCtx;
use crate::diff::{build_sync_plan, Action, FieldChange, ResourceAction, SyncPlan};
use crate::lockfile::{BadgeLock, EnvLock, Lockfile, PassLock, ProductLock};
use rbx_core::confirm::confirm_destructive;
use rbx_core::owner::OwnerType;
use rbx_core::places::PlacesFile;
use rbx_core::EnvTarget;

pub async fn run(
    ctx: &ShopCtx<'_>,
    dry_run: bool,
    only: Option<Vec<ResourceKind>>,
    badge_cost: u64,
    yes: bool,
    allow_duplicate_names: bool,
) -> Result<()> {
    let config = Config::load_merged(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new("."));
    let lockfile_path = config_dir.join(crate::lockfile::LOCKFILE_NAME);
    let mut lockfile = Lockfile::load(&lockfile_path)?;
    lockfile.version = crate::lockfile::LOCKFILE_VERSION;

    let envs = ctx.resolve_envs(&config)?;

    // For a multi-env sync (--env all), confirm once up front if ANY target
    // env has `confirm = true` in rbxplace.toml — there's no per-env per-plan
    // prompt because we'd hit it mid-loop after starting writes elsewhere.
    // Dry-run skips both — no writes to gate.
    if !dry_run && ctx.env().is_some() {
        let env_requires_confirm = PlacesFile::load(ctx.places_path())
            .ok()
            .map(|pf| {
                envs.iter()
                    .any(|t| pf.get(&t.name).map(|e| e.confirm()).unwrap_or(false))
            })
            .unwrap_or(false);
        let label: String = envs
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        confirm_destructive(
            &format!("Apply rbx shop sync to env(s): [{}]?", label),
            env_requires_confirm,
            yes,
        )?;
    }

    let mut any_synced = false;
    for env_target in &envs {
        if envs.len() > 1 {
            println!("\n{} {}", "env:".bold(), env_target.name.bold());
        }
        let synced = sync_one_env(
            ctx,
            &config,
            config_dir,
            &mut lockfile,
            &lockfile_path,
            env_target,
            dry_run,
            only.as_ref(),
            badge_cost,
            allow_duplicate_names,
        )
        .await?;
        any_synced |= synced;
    }

    if !dry_run {
        if any_synced {
            println!("{} Sync complete.", "✓".green());
        }
        if config.codegen.output.is_some() {
            codegen::generate(&config, &lockfile, config_dir)?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn sync_one_env(
    ctx: &ShopCtx<'_>,
    config: &Config,
    config_dir: &Path,
    lockfile: &mut Lockfile,
    lockfile_path: &Path,
    env_target: &EnvTarget,
    dry_run: bool,
    only: Option<&Vec<ResourceKind>>,
    badge_cost: u64,
    allow_duplicate_names: bool,
) -> Result<bool> {
    let resources = config.resolve_env(Some(&env_target.name))?;
    Config::validate_icon_paths(&resources, config_dir)?;

    // Snapshot the env's lock section, building a fresh one if missing.
    let env_lock_snapshot = lockfile
        .env(&env_target.name)
        .cloned()
        .unwrap_or_else(|| EnvLock {
            universe_id: env_target.universe_id,
            ..Default::default()
        });

    let plan = build_sync_plan(&resources, &env_lock_snapshot, config_dir)?;

    for warning in &plan.warnings {
        println!("{} {}", "!".yellow(), warning);
    }

    if !plan.has_changes() {
        println!("{} Everything is up to date.", "✓".green());
        // Ensure lockfile carries the right universe_id even if no resource changed.
        lockfile.env_mut(&env_target.name, env_target.universe_id);
        return Ok(false);
    }

    let should_sync =
        |kind: ResourceKind| -> bool { only.is_none_or(|kinds| kinds.contains(&kind)) };

    for kind in ResourceKind::ALL {
        if should_sync(kind) {
            for action in plan.actions(kind) {
                print_action(kind, action);
            }
        }
    }

    println!("\n{}", plan.summary());

    if dry_run {
        println!("\nDry run — no changes applied.");
        // Said out loud, because the plan above is incomplete in one specific
        // way and a reader has no way to tell from it. The duplicate-name guard
        // asks Roblox what already exists, and a dry run deliberately never
        // opens a connection — so a plan that says "create" here can still be
        // refused by the real run.
        //
        // The alternative was to run the guard during the dry run, which would
        // cost this command its offline property for the sake of a preview.
        // `rbx shop check` is the online comparison.
        let creating = ResourceKind::ALL
            .into_iter()
            .flat_map(|kind| plan.actions(kind))
            .any(|a| matches!(a.action, Action::Create));
        if creating {
            println!(
                "{}",
                "  Roblox was not contacted, so nothing above was compared against \
                 what already exists there. The real run does that, and refuses on \
                 a name collision."
                    .dimmed()
            );
        }
        return Ok(false);
    }

    let client = ctx.client(env_target.universe_id, config.icons.bleed);

    // Before the first write, and before the badge-payment lookup below: this
    // is the last point where the run can be abandoned having changed nothing.
    // A collision found here costs a listing call; the same collision found
    // one line later costs a duplicate paid product that Roblox will not
    // delete. See `preflight`.
    let syncing: Vec<ResourceKind> = ResourceKind::ALL
        .into_iter()
        .filter(|k| should_sync(*k))
        .collect();
    crate::preflight::guard(
        &client,
        env_target.universe_id,
        &plan,
        &resources,
        &syncing,
        &env_target.name,
        allow_duplicate_names,
    )
    .await?;

    // Only creating a badge needs to know who pays for it, and resolving the
    // owner can fail, so a run that only updates badges must not ask.
    let creating_a_badge = should_sync(ResourceKind::Badge)
        && plan
            .badges
            .iter()
            .any(|a| matches!(a.action, Action::Create));

    let apply = Apply {
        badge_payment_source: if creating_a_badge {
            match resolve_payment_source(&client, ctx, config, &env_target.name).await? {
                OwnerType::User => 1,
                OwnerType::Group => 2,
            }
        } else {
            1
        },
        client,
        badge_cost,
    };

    if should_sync(ResourceKind::Pass) {
        apply_kind::<PassKind>(
            &apply,
            &resources,
            &plan,
            lockfile,
            lockfile_path,
            env_target,
        )
        .await?;
    }
    if should_sync(ResourceKind::Badge) {
        apply_kind::<BadgeKind>(
            &apply,
            &resources,
            &plan,
            lockfile,
            lockfile_path,
            env_target,
        )
        .await?;
    }
    if should_sync(ResourceKind::Product) {
        apply_kind::<ProductKind>(
            &apply,
            &resources,
            &plan,
            lockfile,
            lockfile_path,
            env_target,
        )
        .await?;
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// Applying one kind
// ---------------------------------------------------------------------------

/// Everything the API calls need beyond the resource itself.
///
/// Who pays for a badge: ask Roblox, fall back to what the config declares.
///
/// Roblox already knows. A badge on a group-owned game is paid from group
/// funds and one on a user-owned game from the user's, with no way to cross
/// them, so the answer follows from ownership — and ownership is a field on
/// the universe that `universe:read` returns. Asking removes a whole class of
/// wrong answer: a config saying `user` for a group-owned game is a create
/// Roblox refuses, and nothing local would have caught it.
///
/// It is a preference, not a requirement. A key without `universe:read` gets
/// an error from the call, which is treated as "no answer" and falls through
/// to `[owner]` in `rbxshop.toml`, then `rbxplace.toml`. That is what keeps
/// this from adding a scope to every key that syncs a shop.
async fn resolve_payment_source(
    client: &RbxClient,
    ctx: &ShopCtx<'_>,
    config: &Config,
    env_name: &str,
) -> Result<OwnerType> {
    if let Ok(Some(kind)) = client.universe_owner().await {
        return Ok(kind);
    }
    ctx.resolve_owner_type(config, env_name)
}

/// `badge_payment_source` and `badge_cost` only mean something to badges;
/// they live here rather than in the trait's signature so passes and products
/// are not threading arguments they ignore.
struct Apply {
    client: RbxClient,
    badge_payment_source: u32,
    badge_cost: u64,
}

/// One resource's inputs, resolved once by `apply_kind` so no kind repeats
/// the work: the config entry, the display name Roblox will show, and the icon
/// already located on disk and hashed.
struct Resolved<'a, C> {
    cfg: &'a C,
    key: &'a str,
    display_name: &'a str,
    icon: Option<&'a Path>,
    /// Hash of the icon file as it sits on disk, which is what the next diff
    /// compares against — not the hash of the re-encoded upload.
    icon_hash: Option<String>,
}

/// What a resource kind has to supply for `apply_kind` to drive it.
///
/// The two `async fn`s are the only genuinely per-kind part: which endpoint
/// gets called, and what the response means for the lockfile entry. Each also
/// prints its own update line, because the shapes differ — a badge update can
/// be a metadata call, an icon upload, or both.
trait Appliable {
    type Cfg;
    /// Cloned out of the lockfile before an update: the API call needs the id
    /// it recorded, and the borrow has to end before the entry is written back.
    type Lock: Clone;

    const KIND: ResourceKind;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg>;
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock>;
    fn locked_mut(env_lock: &mut EnvLock) -> &mut BTreeMap<String, Self::Lock>;
    fn icon(cfg: &Self::Cfg) -> Option<&Path>;
    /// The explicit `name` override, if the user set one.
    fn name(cfg: &Self::Cfg) -> Option<&str>;
    fn id(lock: &Self::Lock) -> u64;

    async fn create(apply: &Apply, r: Resolved<'_, Self::Cfg>) -> Result<Self::Lock>;

    async fn update(
        apply: &Apply,
        r: Resolved<'_, Self::Cfg>,
        prior: Self::Lock,
        changes: &[FieldChange],
    ) -> Result<Self::Lock>;
}

async fn apply_kind<K: Appliable>(
    apply: &Apply,
    resources: &ResolvedResources,
    plan: &SyncPlan,
    lockfile: &mut Lockfile,
    lockfile_path: &Path,
    env_target: &EnvTarget,
) -> Result<()> {
    let config_dir = lockfile_path.parent().unwrap_or(Path::new("."));

    for action in plan.actions(K::KIND) {
        if matches!(action.action, Action::Skip) {
            continue;
        }

        let cfg = &K::configured(resources)[&action.name];
        let icon_path = K::icon(cfg).map(|p| config_dir.join(p));
        let resolved = Resolved {
            cfg,
            key: &action.name,
            display_name: resolve_name(K::name(cfg), &action.name),
            icon: icon_path.as_deref(),
            icon_hash: icon_path.as_ref().map(|p| hash_file(p)).transpose()?,
        };

        let entry = match &action.action {
            Action::Create => {
                print!("  Creating {} '{}'...", K::KIND, action.name);
                let entry = K::create(apply, resolved).await?;
                println!(" {} (id: {})", "done".green(), K::id(&entry));
                entry
            }
            Action::Update { changes } => {
                let prior = lockfile
                    .env(&env_target.name)
                    .and_then(|e| K::locked(e).get(&action.name))
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!(
                            "{} should be in lockfile for update action",
                            K::KIND.label()
                        )
                    });
                K::update(apply, resolved, prior, changes).await?
            }
            Action::Skip => unreachable!("skipped above"),
        };

        let env_lock = lockfile.env_mut(&env_target.name, env_target.universe_id);
        K::locked_mut(env_lock).insert(action.name.clone(), entry);
        lockfile.save(lockfile_path)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// The three kinds
// ---------------------------------------------------------------------------

struct PassKind;
struct BadgeKind;
struct ProductKind;

impl Appliable for PassKind {
    type Cfg = PassConfig;
    type Lock = PassLock;

    const KIND: ResourceKind = ResourceKind::Pass;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg> {
        &resources.passes
    }
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock> {
        &env_lock.passes
    }
    fn locked_mut(env_lock: &mut EnvLock) -> &mut BTreeMap<String, Self::Lock> {
        &mut env_lock.passes
    }
    fn icon(cfg: &Self::Cfg) -> Option<&Path> {
        cfg.icon.as_deref()
    }
    fn name(cfg: &Self::Cfg) -> Option<&str> {
        cfg.name.as_deref()
    }
    fn id(lock: &Self::Lock) -> u64 {
        lock.id
    }

    async fn create(apply: &Apply, r: Resolved<'_, Self::Cfg>) -> Result<Self::Lock> {
        let Resolved {
            cfg,
            display_name,
            icon,
            icon_hash,
            ..
        } = r;
        let result = apply
            .client
            .create_game_pass(
                display_name,
                cfg.description.as_deref(),
                cfg.price,
                icon,
                cfg.for_sale,
                cfg.regional_pricing,
            )
            .await?;
        Ok(PassLock {
            id: result.id.unwrap_or(0),
            name: display_name.to_string(),
            price: cfg.price,
            description: cfg.description.clone(),
            icon_asset_id: result.icon_asset_id,
            icon_hash,
            for_sale: cfg.for_sale,
            regional_pricing: cfg.regional_pricing,
        })
    }

    async fn update(
        apply: &Apply,
        r: Resolved<'_, Self::Cfg>,
        prior: Self::Lock,
        _changes: &[FieldChange],
    ) -> Result<Self::Lock> {
        let Resolved {
            cfg,
            key,
            display_name,
            icon,
            icon_hash,
        } = r;
        let send_icon = changed_icon(icon, icon_hash.as_deref(), prior.icon_hash.as_deref());

        print!("  Updating pass '{}'...", key);
        let result = apply
            .client
            .update_game_pass(
                prior.id,
                display_name,
                cfg.description.as_deref(),
                cfg.price,
                send_icon,
                cfg.for_sale,
                cfg.regional_pricing,
            )
            .await?;
        println!(" {}", "done".green());

        Ok(PassLock {
            id: prior.id,
            name: display_name.to_string(),
            price: cfg.price,
            description: cfg.description.clone(),
            icon_asset_id: result.icon_asset_id.or(prior.icon_asset_id),
            icon_hash: icon_hash.or(prior.icon_hash),
            for_sale: cfg.for_sale,
            regional_pricing: cfg.regional_pricing,
        })
    }
}

impl Appliable for ProductKind {
    type Cfg = ProductConfig;
    type Lock = ProductLock;

    const KIND: ResourceKind = ResourceKind::Product;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg> {
        &resources.products
    }
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock> {
        &env_lock.products
    }
    fn locked_mut(env_lock: &mut EnvLock) -> &mut BTreeMap<String, Self::Lock> {
        &mut env_lock.products
    }
    fn icon(cfg: &Self::Cfg) -> Option<&Path> {
        cfg.icon.as_deref()
    }
    fn name(cfg: &Self::Cfg) -> Option<&str> {
        cfg.name.as_deref()
    }
    fn id(lock: &Self::Lock) -> u64 {
        lock.id
    }

    async fn create(apply: &Apply, r: Resolved<'_, Self::Cfg>) -> Result<Self::Lock> {
        let Resolved {
            cfg,
            display_name,
            icon,
            icon_hash,
            ..
        } = r;
        let result = apply
            .client
            .create_developer_product(
                display_name,
                cfg.description.as_deref(),
                cfg.price,
                icon,
                cfg.for_sale,
                cfg.regional_pricing,
            )
            .await?;
        Ok(ProductLock {
            id: result.id.unwrap_or(0),
            name: display_name.to_string(),
            price: cfg.price,
            description: cfg.description.clone(),
            icon_asset_id: result.icon_image_asset_id,
            icon_hash,
            for_sale: cfg.for_sale,
            regional_pricing: cfg.regional_pricing,
            store_page: cfg.store_page,
        })
    }

    async fn update(
        apply: &Apply,
        r: Resolved<'_, Self::Cfg>,
        prior: Self::Lock,
        _changes: &[FieldChange],
    ) -> Result<Self::Lock> {
        let Resolved {
            cfg,
            key,
            display_name,
            icon,
            icon_hash,
        } = r;
        let send_icon = changed_icon(icon, icon_hash.as_deref(), prior.icon_hash.as_deref());

        print!("  Updating product '{}'...", key);
        let result = apply
            .client
            .update_developer_product(
                prior.id,
                display_name,
                cfg.description.as_deref(),
                cfg.price,
                send_icon,
                cfg.for_sale,
                cfg.regional_pricing,
                cfg.store_page,
            )
            .await?;
        println!(" {}", "done".green());

        Ok(ProductLock {
            id: prior.id,
            name: display_name.to_string(),
            price: cfg.price,
            description: cfg.description.clone(),
            icon_asset_id: result.icon_image_asset_id.or(prior.icon_asset_id),
            icon_hash: icon_hash.or(prior.icon_hash),
            for_sale: cfg.for_sale,
            regional_pricing: cfg.regional_pricing,
            store_page: cfg.store_page,
        })
    }
}

impl Appliable for BadgeKind {
    type Cfg = BadgeConfig;
    type Lock = BadgeLock;

    const KIND: ResourceKind = ResourceKind::Badge;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg> {
        &resources.badges
    }
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock> {
        &env_lock.badges
    }
    fn locked_mut(env_lock: &mut EnvLock) -> &mut BTreeMap<String, Self::Lock> {
        &mut env_lock.badges
    }
    fn icon(cfg: &Self::Cfg) -> Option<&Path> {
        cfg.icon.as_deref()
    }
    fn name(cfg: &Self::Cfg) -> Option<&str> {
        cfg.name.as_deref()
    }
    fn id(lock: &Self::Lock) -> u64 {
        lock.id
    }

    async fn create(apply: &Apply, r: Resolved<'_, Self::Cfg>) -> Result<Self::Lock> {
        let Resolved {
            cfg,
            display_name,
            icon,
            icon_hash,
            ..
        } = r;
        let result = apply
            .client
            .create_badge(
                display_name,
                cfg.description.as_deref(),
                icon,
                apply.badge_payment_source,
                apply.badge_cost,
            )
            .await?;
        Ok(BadgeLock {
            id: result.id.unwrap_or(0),
            name: display_name.to_string(),
            description: cfg.description.clone(),
            enabled: cfg.enabled,
            icon_asset_id: result.icon_image_id,
            icon_hash,
        })
    }

    /// Badges are the one kind whose metadata and icon are two endpoints, so
    /// this decides from the plan's own field list which of them to call —
    /// a metadata-only change must not re-upload the icon, and an icon-only
    /// change must not rewrite name and description for nothing.
    async fn update(
        apply: &Apply,
        r: Resolved<'_, Self::Cfg>,
        prior: Self::Lock,
        changes: &[FieldChange],
    ) -> Result<Self::Lock> {
        let Resolved {
            cfg,
            key,
            display_name,
            icon,
            icon_hash,
        } = r;
        let icon_changed = changes.iter().any(|c| c.field == "icon");
        let has_metadata_changes = changes.iter().any(|c| c.field != "icon");

        if has_metadata_changes {
            print!("  Updating badge '{}'...", key);
            apply
                .client
                .update_badge(
                    prior.id,
                    display_name,
                    cfg.description.as_deref(),
                    cfg.enabled,
                )
                .await?;
            println!(" {}", "done".green());
        }

        let mut icon_asset_id = prior.icon_asset_id;
        let mut new_icon_hash = prior.icon_hash;

        if icon_changed {
            if let Some(icon_path) = icon {
                print!("  Updating badge '{}' icon...", key);
                let result = apply.client.update_badge_icon(prior.id, icon_path).await?;
                new_icon_hash = icon_hash;
                icon_asset_id = result.target_id.or(icon_asset_id);
                println!(" {}", "done".green());
            }
        }

        Ok(BadgeLock {
            id: prior.id,
            name: display_name.to_string(),
            description: cfg.description.clone(),
            enabled: cfg.enabled,
            icon_asset_id,
            icon_hash: new_icon_hash,
        })
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The icon to send with an update, or `None` to leave the remote one alone.
///
/// A false negative here re-uploads the same image on every sync; a false
/// positive leaves a stale icon live forever. An icon that was never uploaded
/// (no hash in the lockfile) always counts as changed.
fn changed_icon<'a>(
    icon: Option<&'a Path>,
    icon_hash: Option<&str>,
    locked_hash: Option<&str>,
) -> Option<&'a Path> {
    let changed = match (icon_hash, locked_hash) {
        (Some(new), Some(old)) => new != old,
        (Some(_), None) => true,
        _ => false,
    };
    if changed {
        icon
    } else {
        None
    }
}

fn print_action(kind: ResourceKind, action: &ResourceAction) {
    match &action.action {
        Action::Create => {
            println!(
                "  {} {} {} {}",
                "+".green(),
                "create".green(),
                kind,
                action.name.bold()
            );
        }
        Action::Update { changes } => {
            println!(
                "  {} {} {} {}",
                "~".yellow(),
                "update".yellow(),
                kind,
                action.name.bold()
            );
            for change in changes {
                println!("    {} {}", "·".dimmed(), change);
            }
        }
        Action::Skip => {
            println!(
                "  {} {} {} {}",
                "=".dimmed(),
                "skip".dimmed(),
                kind,
                action.name.dimmed()
            );
        }
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(rbx_core::image::hash_bytes(&bytes))
}

#[cfg(test)]
mod tests;
