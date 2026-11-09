//! Apply the plan: create and update passes, badges and products on Roblox,
//! recording each result in the lockfile as it lands.
//!
//! `apply_pass_actions` and `apply_product_actions` used to be two ~108-line
//! functions that were 0.94 identical after renaming their identifiers, with
//! `apply_badge_actions` a third variation on the same theme. The shape they
//! share: resolve the display name, hash the icon, call the API, write the
//! entry back, save: now lives once in `apply_kind`, and each kind supplies
//! only its own two API calls (see `Appliable`).
//!
//! The lockfile is saved after every single resource, not once at the end.
//! That is deliberate: these calls create paid products, and a run that dies
//! halfway must not lose the ids of what it already made.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Result};
use colored::Colorize;

use crate::api::{FieldUpdate, Pricing, RbxClient};
use crate::codegen;
use crate::config::{
    resolve_name, BadgeConfig, Config, PassConfig, ProductConfig, ResolvedResources, ResourceKind,
};
use crate::ctx::ShopCtx;
use crate::diff::{build_sync_plan, Action, FieldChange, ResourceAction, SyncPlan};
use crate::lockfile::{BadgeLock, EnvLock, Lockfile, PassLock, ProductLock, DEFAULT_ENV};
use crate::refresh::{refresh, Refreshed};
use rbx_core::confirm::confirm_destructive;
use rbx_core::newer::{self, Newer};
use rbx_core::owner::OwnerType;
use rbx_core::places::PlacesFile;
use rbx_core::EnvTarget;

/// What the command line asked of one run.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub dry_run: bool,
    /// The kinds to sync. `None` is all of them.
    pub only: Option<Vec<ResourceKind>>,
    pub badge_cost: u64,
    pub yes: bool,
    pub allow_duplicate_names: bool,
    /// Write over what was changed on Roblox since the last sync or pull.
    pub overwrite_newer: bool,
}

pub async fn run(ctx: &ShopCtx<'_>, options: Options) -> Result<()> {
    let Options {
        dry_run,
        only,
        badge_cost,
        yes,
        allow_duplicate_names,
        overwrite_newer,
    } = options;
    let config = Config::load_merged(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new("."));
    let lockfile_path = config_dir.join(crate::lockfile::LOCKFILE_NAME);
    let mut lockfile = Lockfile::load(&lockfile_path)?;
    lockfile.version = crate::lockfile::LOCKFILE_VERSION;

    let envs = ctx.resolve_envs(&config)?;

    // Ahead of every write and of the dry run, and for every env before the
    // first one starts, so `--env all` cannot apply half a run and then refuse.
    //
    // Letting this through overwrites the recorded universe with the new one and
    // keeps the old ids under it. Nothing fails after that: the plan says up to
    // date, and generated code hands the game ids from another experience.
    for target in &envs {
        if let Some(lock) = lockfile.env(&target.name) {
            if lock.tracks_another_universe(target.universe_id) {
                bail!(
                    "Lockfile env '{}' tracks universe {}, but it now resolves to {}. The ids it \
                     holds belong to the other universe. If this env moved, `rbx shop pull --env \
                     {}` rebuilds the section from the new one. If not, fix rbxplace.toml.",
                    target.name,
                    lock.universe_id,
                    target.universe_id,
                    target.name
                );
            }
        }
    }

    let syncing: Vec<ResourceKind> = ResourceKind::ALL
        .into_iter()
        .filter(|kind| only.as_ref().is_none_or(|kinds| kinds.contains(kind)))
        .collect();

    // Planning pass. It lists Roblox and writes nothing there, for every env
    // before the first one is applied: that is what lets one refusal, and one
    // confirmation, cover the whole run. A prompt inside a single loop is
    // reached after earlier envs have already been written, which is no
    // longer a question anyone can answer no to.
    let mut planned: Vec<PlannedEnv> = Vec::new();
    for env_target in &envs {
        if envs.len() > 1 {
            println!("\n{} {}", "env:".bold(), env_target.name.bold());
        }
        planned.push(
            plan_env(
                ctx,
                &config,
                config_dir,
                &lockfile,
                env_target,
                &syncing,
                allow_duplicate_names,
            )
            .await?,
        );
    }

    // After the plans, so whoever is refused has already seen what would have
    // gone. Before the dry run returns, so a preview cannot report a plan the
    // real run refuses.
    report_newer(ctx, &planned, overwrite_newer)?;

    let pending: Vec<&PlannedEnv> = planned.iter().filter(|p| p.has_work(&syncing)).collect();

    if dry_run {
        if !pending.is_empty() {
            println!("\nDry run: no changes applied.");
        }
        return Ok(());
    }

    if !pending.is_empty() {
        confirm(ctx, &pending, yes)?;

        // The badge quota of every env, before the first write to any of
        // them: passes are applied before badges, so learning it from Roblox
        // halfway leaves an env holding the passes and some of the badges.
        for p in &pending {
            if p.creates_a_badge(&syncing) {
                let client = ctx.client(p.universe_id, config.icons.bleed);
                crate::preflight::guard_badge_quota(&client, p.universe_id, &p.plan, badge_cost)
                    .await?;
            }
        }
    }

    // What the listings found, recorded before the first write: each resource
    // applied below then moves its entry from what Roblox had to what it was
    // sent, and a run that stops halfway leaves the lockfile describing Roblox
    // as it is. With nothing to apply, this is the whole of the run, and what
    // lets `check` agree with a `sync` that found Roblox already holding what
    // the config asks for.
    if record_seen(&mut lockfile, &planned) {
        lockfile.save(&lockfile_path)?;
        if pending.is_empty() {
            println!(
                "{}",
                format!(
                    "{} now records what Roblox has.",
                    crate::lockfile::LOCKFILE_NAME
                )
                .dimmed()
            );
        }
    }

    for p in &pending {
        if envs.len() > 1 {
            println!("\n{} {}", "env:".bold(), p.env.bold());
        }
        apply_env(
            ctx,
            &config,
            p,
            &syncing,
            badge_cost,
            &mut lockfile,
            &lockfile_path,
        )
        .await?;
    }

    if !pending.is_empty() {
        println!("{} Sync complete.", "✓".green());
    }
    if config.codegen.output.is_some() {
        codegen::generate(&config, &lockfile, config_dir)?;
    }

    Ok(())
}

/// One env as the planning pass left it, carried to the apply pass.
///
/// Every env is kept, not only the ones with work: an env whose plan is empty
/// can still have been listed holding something the lockfile does not record.
struct PlannedEnv {
    env: String,
    universe_id: u64,
    resources: ResolvedResources,
    plan: SyncPlan,
    /// The lock section as Roblox was listed to stand, which is what `plan`
    /// was built against. See [`crate::refresh`].
    seen: EnvLock,
    /// Values changed on Roblox since the lockfile recorded them, that `plan`
    /// would write over.
    newer: Vec<String>,
}

impl PlannedEnv {
    /// Whether applying the plan sends anything, for the kinds in play.
    fn has_work(&self, syncing: &[ResourceKind]) -> bool {
        syncing
            .iter()
            .flat_map(|kind| self.plan.actions(*kind))
            .any(|action| !matches!(action.action, Action::Skip))
    }

    /// Only creating a badge needs the quota and whoever pays for it, and
    /// resolving the owner can fail, so a run that only updates badges must
    /// not ask.
    fn creates_a_badge(&self, syncing: &[ResourceKind]) -> bool {
        syncing.contains(&ResourceKind::Badge)
            && self
                .plan
                .badges
                .iter()
                .any(|action| matches!(action.action, Action::Create))
    }
}

/// Plan one env: list what Roblox has, bring the lock section up to date with
/// it, and diff the config against that.
async fn plan_env(
    ctx: &ShopCtx<'_>,
    config: &Config,
    config_dir: &Path,
    lockfile: &Lockfile,
    env_target: &EnvTarget,
    syncing: &[ResourceKind],
    allow_duplicate_names: bool,
) -> Result<PlannedEnv> {
    let resources = config.resolve_env(Some(&env_target.name))?;
    Config::validate_icon_paths(&resources, config_dir)?;
    Config::validate_pricing(&resources)?;

    // The env's lock section as recorded, or a fresh one if it has none.
    let recorded = lockfile
        .env(&env_target.name)
        .cloned()
        .unwrap_or_else(|| EnvLock {
            universe_id: env_target.universe_id,
            ..Default::default()
        });

    // Roblox's side, listed before anything is planned. The plan below is the
    // config against what Roblox holds now, so a price edited on the Creator
    // Dashboard is in it. Planned against the lockfile alone, that edit was
    // invisible: the two local files agreed and the run said up to date.
    let client = ctx.client(env_target.universe_id, config.icons.bleed);
    let remote = crate::live::read(&client, &recorded, syncing).await?;
    let Refreshed { lock: seen, gone } = refresh(&recorded, &remote, syncing);

    let plan = build_sync_plan(&resources, &seen, config_dir)?;
    let newer: Vec<String> = crate::live::changed_fields(&recorded, &seen)
        .into_iter()
        .filter(|changed| plan.writes(changed.kind, &changed.key, changed.field))
        .map(|changed| changed.to_string())
        .collect();

    for message in &plan.undeclared {
        println!("{} {}", "!".yellow(), message);
    }
    for line in &gone {
        println!("{} {line}. Its entry is kept as recorded.", "!".yellow());
    }

    if plan.has_changes() {
        for kind in syncing {
            for action in plan.actions(*kind) {
                print_action(*kind, action);
            }
        }
        println!("\n{}", plan.summary());
    } else {
        println!("{} Everything is up to date.", "✓".green());
    }

    // On the listing just made, so a dry run makes this check too. A collision
    // found here costs nothing; the same collision found after a create costs
    // a duplicate paid product that Roblox will not delete. See `preflight`.
    crate::preflight::guard(
        &plan,
        &resources,
        syncing,
        &remote,
        &env_target.name,
        allow_duplicate_names,
    )?;

    Ok(PlannedEnv {
        env: env_target.name.clone(),
        universe_id: env_target.universe_id,
        resources,
        plan,
        seen,
        newer,
    })
}

/// Ask before applying, when an env that is about to be written asked for it.
///
/// Only the envs with work are named and looked at: an env with `confirm =
/// true` that this run leaves untouched has nothing to confirm.
///
/// A bare `--universe-id` names no env, so no `confirm` can be read for it:
/// such a write is always asked about, or needs `--yes`.
fn confirm(ctx: &ShopCtx<'_>, pending: &[&PlannedEnv], yes: bool) -> Result<()> {
    if ctx.global.universe_id.is_some() {
        return confirm_destructive(
            &format!(
                "Apply rbx shop sync to universe {}?",
                pending[0].universe_id
            ),
            true,
            yes,
        );
    }
    if ctx.env().is_none() {
        return Ok(());
    }
    let env_requires_confirm = PlacesFile::load(ctx.places_path())
        .ok()
        .map(|pf| {
            pending
                .iter()
                .any(|p| pf.get(&p.env).map(|e| e.confirm()).unwrap_or(false))
        })
        .unwrap_or(false);
    let label: String = pending
        .iter()
        .map(|p| p.env.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    confirm_destructive(
        &format!("Apply rbx shop sync to env(s): [{}]?", label),
        env_requires_confirm,
        yes,
    )
}

/// Stop a run that would write over what was changed on Roblox, or say what it
/// replaces when that was asked for. See [`rbx_core::newer`].
fn report_newer(ctx: &ShopCtx<'_>, planned: &[PlannedEnv], overwrite_newer: bool) -> Result<()> {
    let changed: Vec<Newer<'_>> = planned
        .iter()
        .filter(|p| !p.newer.is_empty())
        .map(|p| Newer {
            env: &p.env,
            values: &p.newer,
        })
        .collect();
    let envs: Vec<&str> = changed.iter().map(|newer| newer.env).collect();
    // A target named by id on the command line has no env to name in the
    // hint, so the hint carries the id.
    let by_id = ctx
        .global
        .universe_id
        .map(|universe_id| format!("--universe-id {universe_id}"));

    newer::report(
        &changed,
        overwrite_newer,
        &ctx.config,
        &newer::pull_hint("shop", &envs, DEFAULT_ENV, by_id.as_deref()),
    )
}

/// Record what the listings found, for every env planned. Returns whether a
/// section changed, which is when the file is worth writing.
///
/// Every env gets its section in memory, with its universe id, whether or not
/// anything changed: the generated code is written from the lockfile as it
/// stands at the end of the run, and reads the universe from there.
fn record_seen(lockfile: &mut Lockfile, planned: &[PlannedEnv]) -> bool {
    let mut changed = false;
    for p in planned {
        let section = lockfile.env_mut(&p.env, p.universe_id);
        if *section != p.seen {
            *section = p.seen.clone();
            changed = true;
        }
    }
    changed
}

/// Send one env's plan to Roblox, recording each resource in the lockfile as
/// it lands.
async fn apply_env(
    ctx: &ShopCtx<'_>,
    config: &Config,
    planned: &PlannedEnv,
    syncing: &[ResourceKind],
    badge_cost: u64,
    lockfile: &mut Lockfile,
    lockfile_path: &Path,
) -> Result<()> {
    let PlannedEnv {
        env,
        universe_id,
        resources,
        plan,
        ..
    } = planned;
    let env_target = EnvTarget {
        name: env.clone(),
        universe_id: *universe_id,
    };
    let client = ctx.client(*universe_id, config.icons.bleed);

    let apply = Apply {
        badge_payment_source: if planned.creates_a_badge(syncing) {
            match resolve_payment_source(&client, ctx, config, env).await? {
                OwnerType::User => 1,
                OwnerType::Group => 2,
            }
        } else {
            1
        },
        client,
        badge_cost,
    };

    if syncing.contains(&ResourceKind::Pass) {
        apply_kind::<PassKind>(
            &apply,
            resources,
            plan,
            lockfile,
            lockfile_path,
            &env_target,
        )
        .await?;
    }
    if syncing.contains(&ResourceKind::Badge) {
        apply_kind::<BadgeKind>(
            &apply,
            resources,
            plan,
            lockfile,
            lockfile_path,
            &env_target,
        )
        .await?;
    }
    if syncing.contains(&ResourceKind::Product) {
        apply_kind::<ProductKind>(
            &apply,
            resources,
            plan,
            lockfile,
            lockfile_path,
            &env_target,
        )
        .await?;
    }

    Ok(())
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
/// them, so the answer follows from ownership, and ownership is a field on
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
    /// compares against, not the hash of the re-encoded upload.
    icon_hash: Option<String>,
}

/// What a resource kind has to supply for `apply_kind` to drive it.
///
/// The two `async fn`s are the only genuinely per-kind part: which endpoint
/// gets called, and what the response means for the lockfile entry. Each also
/// prints its own update line, because the shapes differ: a badge update can
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
                let entry = K::create(apply, resolved)
                    .await
                    .map_err(explain_rate_limit)?;
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
                K::update(apply, resolved, prior, changes)
                    .await
                    .map_err(explain_rate_limit)?
            }
            Action::Skip => unreachable!("skipped above"),
        };

        let env_lock = lockfile.env_mut(&env_target.name, env_target.universe_id);
        K::locked_mut(env_lock).insert(action.name.clone(), entry);
        lockfile.save(lockfile_path)?;
    }

    Ok(())
}

/// A rate limit that outlasted every retry.
///
/// A bare `429 Too Many Requests` reads like a failed run to clean up after.
/// It is not one: every resource before this one is already saved in the
/// lockfile, and the refused call applied nothing, so the fix is to run the
/// same command again once Roblox lets up.
fn explain_rate_limit(error: anyhow::Error) -> anyhow::Error {
    if rbx_core::api::is_api_status(&error, reqwest::StatusCode::TOO_MANY_REQUESTS) {
        error.context(format!(
            "Roblox is still rate-limiting writes after about a minute of retries.\n  \
             Everything created before this point is saved in {}, and this call applied \
             nothing.\n  Fix: wait a minute, then run the same command again to continue.",
            crate::lockfile::LOCKFILE_NAME
        ))
    } else {
        error
    }
}

/// The id Roblox gave a resource it just created.
///
/// A reply without one is not recorded as `0`: every later sync would address
/// a resource that does not exist, while the real one sat unmanaged. Stopping
/// leaves no lock entry, and the name guard then keeps the next sync from
/// making a second one.
fn created_id(id: Option<u64>, kind: ResourceKind, name: &str) -> Result<u64> {
    id.ok_or_else(|| {
        anyhow::anyhow!(
            "Roblox created {kind} '{name}' but its reply carried no id, so it is not in the \
             lockfile.\n  Adopt it with `rbx shop pull` rather than syncing again."
        )
    })
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
                Pricing::from_config(cfg.regional_pricing, cfg.managed_pricing),
            )
            .await?;
        Ok(PassLock {
            id: created_id(result.id, Self::KIND, display_name)?,
            name: display_name.to_string(),
            price: cfg.price,
            description: cfg.description.clone(),
            icon_asset_id: result.icon_asset_id,
            icon_hash,
            for_sale: cfg.for_sale,
            regional_pricing: cfg.regional_pricing,
            // What Roblox reports back, not what was asked: a pass Roblox
            // opted in by itself records as on even when the config is silent.
            managed_pricing: result.is_managed_pricing_enabled,
        })
    }

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
        let send_icon = changed_icon(icon, icon_hash.as_deref(), prior.icon_hash.as_deref());
        let for_sale_changed = changed(changes, "for_sale");
        let update = FieldUpdate {
            name: changed(changes, "name").then_some(display_name),
            description: changed(changes, "description")
                .then(|| cfg.description.as_deref().unwrap_or("")),
            // A pass put back on sale takes its price with it: the spec does
            // not say what Roblox does with an on-sale pass sent no price.
            price: (changed(changes, "price") || (for_sale_changed && cfg.for_sale))
                .then_some(cfg.price)
                .flatten(),
            is_for_sale: for_sale_changed.then_some(cfg.for_sale),
            pricing: pricing_update(changes, cfg.regional_pricing, cfg.managed_pricing),
            store_page: None,
            icon: send_icon,
        };

        print!("  Updating pass '{}'...", key);
        let result = apply.client.update_game_pass(prior.id, &update).await?;
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
            managed_pricing: result.is_managed_pricing_enabled.or(prior.managed_pricing),
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
                Pricing::from_config(cfg.regional_pricing, cfg.managed_pricing),
            )
            .await?;
        Ok(ProductLock {
            id: created_id(result.id, Self::KIND, display_name)?,
            name: display_name.to_string(),
            price: cfg.price,
            description: cfg.description.clone(),
            icon_asset_id: result.icon_image_asset_id,
            icon_hash,
            for_sale: cfg.for_sale,
            regional_pricing: cfg.regional_pricing,
            managed_pricing: result.is_managed_pricing_enabled,
            store_page: cfg.store_page,
        })
    }

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
        let send_icon = changed_icon(icon, icon_hash.as_deref(), prior.icon_hash.as_deref());
        let for_sale_changed = changed(changes, "for_sale");
        let update = FieldUpdate {
            name: changed(changes, "name").then_some(display_name),
            description: changed(changes, "description")
                .then(|| cfg.description.as_deref().unwrap_or("")),
            // As for passes: back on sale, the price goes with it.
            price: (changed(changes, "price") || (for_sale_changed && cfg.for_sale))
                .then_some(cfg.price),
            is_for_sale: for_sale_changed.then_some(cfg.for_sale),
            pricing: pricing_update(changes, cfg.regional_pricing, cfg.managed_pricing),
            // A product off sale is never on the store page, so a change of
            // either field sends the page state the two of them now imply.
            store_page: (changed(changes, "store_page") || for_sale_changed)
                .then_some(cfg.store_page && cfg.for_sale),
            icon: send_icon,
        };

        print!("  Updating product '{}'...", key);
        let result = apply
            .client
            .update_developer_product(prior.id, &update)
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
            managed_pricing: result.is_managed_pricing_enabled.or(prior.managed_pricing),
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
            id: created_id(result.id, Self::KIND, display_name)?,
            name: display_name.to_string(),
            description: cfg.description.clone(),
            enabled: cfg.enabled,
            icon_asset_id: result.icon_image_id,
            icon_hash,
        })
    }

    /// Badges are the one kind whose metadata and icon are two endpoints, so
    /// this decides from the plan's own field list which of them to call:
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

/// Whether the plan changes `field`.
fn changed(changes: &[FieldChange], field: &str) -> bool {
    changes.iter().any(|c| c.field == field)
}

/// The pricing field to send: the config's, when the plan changes either of
/// the two pricing keys, and none otherwise.
fn pricing_update(changes: &[FieldChange], regional: bool, managed: Option<bool>) -> Pricing {
    if changed(changes, "regional_pricing") || changed(changes, "managed_pricing") {
        Pricing::from_config(regional, managed)
    } else {
        Pricing::Untouched
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
