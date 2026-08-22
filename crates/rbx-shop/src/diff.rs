//! Config + lockfile → the plan `sync` applies and `check` reports.
//!
//! The three resource kinds used to be three ~90-line functions with the same
//! structure: create-if-absent, field-by-field `FieldChange`, icon-hash
//! compare, skip-or-update, and the orphan-warning loop was the same
//! paragraph three times. A fix applied to one was a bug waiting in the other
//! two.
//!
//! Now the shape lives once, in `diff_kind`, and each kind supplies only
//! what actually differs: which maps it reads and which fields it compares
//! (see `Diffable`). The icon comparison in particular is written once,
//! which matters because it is a path where a false negative re-uploads on
//! every sync and a false positive leaves a stale icon live.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::Result;

use crate::config::{
    resolve_name, BadgeConfig, PassConfig, ProductConfig, ResolvedResources, ResourceKind,
};
use crate::lockfile::{BadgeLock, EnvLock, PassLock, ProductLock};

#[derive(Debug)]
pub struct SyncPlan {
    pub passes: Vec<ResourceAction>,
    pub badges: Vec<ResourceAction>,
    pub products: Vec<ResourceAction>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct ResourceAction {
    pub name: String,
    pub action: Action,
}

#[derive(Debug)]
pub enum Action {
    Create,
    Update { changes: Vec<FieldChange> },
    Skip,
}

#[derive(Debug)]
pub struct FieldChange {
    pub field: String,
    pub old: String,
    pub new: String,
}

impl FieldChange {
    fn new(field: &str, old: impl fmt::Display, new: impl fmt::Display) -> Self {
        Self {
            field: field.to_string(),
            old: old.to_string(),
            new: new.to_string(),
        }
    }
}

impl fmt::Display for FieldChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} -> {}", self.field, self.old, self.new)
    }
}

impl SyncPlan {
    /// The plan's actions for one kind. Lets callers iterate
    /// `ResourceKind::ALL` instead of naming the three fields.
    pub fn actions(&self, kind: ResourceKind) -> &[ResourceAction] {
        match kind {
            ResourceKind::Pass => &self.passes,
            ResourceKind::Badge => &self.badges,
            ResourceKind::Product => &self.products,
        }
    }

    pub fn has_changes(&self) -> bool {
        ResourceKind::ALL
            .iter()
            .flat_map(|k| self.actions(*k))
            .any(|a| !matches!(a.action, Action::Skip))
    }

    pub fn summary(&self) -> String {
        let mut creates = 0;
        let mut updates = 0;
        let mut skips = 0;

        for action in ResourceKind::ALL.iter().flat_map(|k| self.actions(*k)) {
            match &action.action {
                Action::Create => creates += 1,
                Action::Update { .. } => updates += 1,
                Action::Skip => skips += 1,
            }
        }

        format!(
            "{} to create, {} to update, {} unchanged",
            creates, updates, skips
        )
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(rbx_core::image::hash_bytes(&bytes))
}

pub fn build_sync_plan(
    resources: &ResolvedResources,
    env_lock: &EnvLock,
    config_dir: &Path,
) -> Result<SyncPlan> {
    let mut warnings = Vec::new();
    orphan_warnings::<PassKind>(resources, env_lock, &mut warnings);
    orphan_warnings::<BadgeKind>(resources, env_lock, &mut warnings);
    orphan_warnings::<ProductKind>(resources, env_lock, &mut warnings);

    Ok(SyncPlan {
        passes: diff_kind::<PassKind>(resources, env_lock, config_dir)?,
        badges: diff_kind::<BadgeKind>(resources, env_lock, config_dir)?,
        products: diff_kind::<ProductKind>(resources, env_lock, config_dir)?,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// One kind's view of the two sides being compared
// ---------------------------------------------------------------------------

/// What a resource kind has to supply for the shared diff to run over it.
///
/// Deliberately small: everything else (create-if-absent, the icon-hash
/// compare, skip-versus-update) belongs to every kind equally and lives in
/// `diff_kind`.
trait Diffable {
    type Cfg;
    type Lock;

    const KIND: ResourceKind;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg>;
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock>;

    /// The icon this kind declares, if any, relative to the config directory.
    fn icon(cfg: &Self::Cfg) -> Option<&Path>;
    fn locked_icon_hash(lock: &Self::Lock) -> Option<&str>;

    /// Every non-icon field that diverged, in the order the user should read
    /// them. Called only when the resource already exists remotely.
    fn field_changes(cfg: &Self::Cfg, key: &str, lock: &Self::Lock, out: &mut Vec<FieldChange>);
}

/// A lock entry with no config entry is not deleted: it just falls out of
/// management, and this warning is the only signal the user gets.
fn orphan_warnings<K: Diffable>(
    resources: &ResolvedResources,
    env_lock: &EnvLock,
    out: &mut Vec<String>,
) {
    for key in K::locked(env_lock).keys() {
        if !K::configured(resources).contains_key(key) {
            out.push(format!(
                "{} '{}' exists in lockfile but not in resolved config (will not be deleted)",
                K::KIND.title(),
                key
            ));
        }
    }
}

fn diff_kind<K: Diffable>(
    resources: &ResolvedResources,
    env_lock: &EnvLock,
    config_dir: &Path,
) -> Result<Vec<ResourceAction>> {
    let mut actions = Vec::new();

    for (name, cfg) in K::configured(resources) {
        let Some(lock) = K::locked(env_lock).get(name) else {
            actions.push(ResourceAction {
                name: name.clone(),
                action: Action::Create,
            });
            continue;
        };

        let mut changes = Vec::new();
        K::field_changes(cfg, name, lock, &mut changes);

        // Always last, and always after the kind's own fields: the printed
        // order is what a user reads before approving a sync that spends
        // money. An icon the config no longer declares is left alone rather
        // than deleted remotely.
        if let Some(icon) = K::icon(cfg) {
            let current = hash_file(&config_dir.join(icon))?;
            let locked = K::locked_icon_hash(lock).unwrap_or("");
            if current != locked {
                changes.push(FieldChange::new(
                    "icon",
                    short_hash(locked),
                    short_hash(&current),
                ));
            }
        }

        actions.push(ResourceAction {
            name: name.clone(),
            action: if changes.is_empty() {
                Action::Skip
            } else {
                Action::Update { changes }
            },
        });
    }

    Ok(actions)
}

/// Hashes are 64 hex characters; the first eight are enough to tell two apart
/// and short enough to read in a terminal.
fn short_hash(hash: &str) -> String {
    hash.chars().take(8).collect::<String>() + "..."
}

/// An absent description and an empty one are the same thing to Roblox, so
/// the diff must not churn on `None` vs `Some("")`.
fn description_change(cfg: Option<&str>, lock: Option<&str>, out: &mut Vec<FieldChange>) {
    let (cfg, lock) = (cfg.unwrap_or(""), lock.unwrap_or(""));
    if cfg != lock {
        out.push(FieldChange::new("description", lock, cfg));
    }
}

/// The display name is the explicit `name` when set, else the TOML key: read
/// the key instead and every resource without an explicit name diffs forever.
fn name_change(cfg: Option<&str>, key: &str, lock: &str, out: &mut Vec<FieldChange>) {
    let resolved = resolve_name(cfg, key);
    if resolved != lock {
        out.push(FieldChange::new("name", lock, resolved));
    }
}

fn bool_change(field: &str, cfg: bool, lock: bool, out: &mut Vec<FieldChange>) {
    if cfg != lock {
        out.push(FieldChange::new(field, lock, cfg));
    }
}

// ---------------------------------------------------------------------------
// The three kinds
// ---------------------------------------------------------------------------

struct PassKind;
struct BadgeKind;
struct ProductKind;

impl Diffable for PassKind {
    type Cfg = PassConfig;
    type Lock = PassLock;

    const KIND: ResourceKind = ResourceKind::Pass;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg> {
        &resources.passes
    }
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock> {
        &env_lock.passes
    }
    fn icon(cfg: &Self::Cfg) -> Option<&Path> {
        cfg.icon.as_deref()
    }
    fn locked_icon_hash(lock: &Self::Lock) -> Option<&str> {
        lock.icon_hash.as_deref()
    }

    fn field_changes(cfg: &Self::Cfg, key: &str, lock: &Self::Lock, out: &mut Vec<FieldChange>) {
        name_change(cfg.name.as_deref(), key, &lock.name, out);
        // Debug-formatted because a pass price is optional, and `None` is not
        // the same state as `Some(0)`: a pass with no price is not free.
        if cfg.price != lock.price {
            out.push(FieldChange::new(
                "price",
                format!("{:?}", lock.price),
                format!("{:?}", cfg.price),
            ));
        }
        description_change(cfg.description.as_deref(), lock.description.as_deref(), out);
        bool_change("for_sale", cfg.for_sale, lock.for_sale, out);
        bool_change(
            "regional_pricing",
            cfg.regional_pricing,
            lock.regional_pricing,
            out,
        );
    }
}

impl Diffable for BadgeKind {
    type Cfg = BadgeConfig;
    type Lock = BadgeLock;

    const KIND: ResourceKind = ResourceKind::Badge;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg> {
        &resources.badges
    }
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock> {
        &env_lock.badges
    }
    fn icon(cfg: &Self::Cfg) -> Option<&Path> {
        cfg.icon.as_deref()
    }
    fn locked_icon_hash(lock: &Self::Lock) -> Option<&str> {
        lock.icon_hash.as_deref()
    }

    fn field_changes(cfg: &Self::Cfg, key: &str, lock: &Self::Lock, out: &mut Vec<FieldChange>) {
        name_change(cfg.name.as_deref(), key, &lock.name, out);
        description_change(cfg.description.as_deref(), lock.description.as_deref(), out);
        bool_change("enabled", cfg.enabled, lock.enabled, out);
    }
}

impl Diffable for ProductKind {
    type Cfg = ProductConfig;
    type Lock = ProductLock;

    const KIND: ResourceKind = ResourceKind::Product;

    fn configured(resources: &ResolvedResources) -> &BTreeMap<String, Self::Cfg> {
        &resources.products
    }
    fn locked(env_lock: &EnvLock) -> &BTreeMap<String, Self::Lock> {
        &env_lock.products
    }
    fn icon(cfg: &Self::Cfg) -> Option<&Path> {
        cfg.icon.as_deref()
    }
    fn locked_icon_hash(lock: &Self::Lock) -> Option<&str> {
        lock.icon_hash.as_deref()
    }

    fn field_changes(cfg: &Self::Cfg, key: &str, lock: &Self::Lock, out: &mut Vec<FieldChange>) {
        name_change(cfg.name.as_deref(), key, &lock.name, out);
        // A product price is required, so it prints bare rather than as
        // `Some(99)`.
        if cfg.price != lock.price {
            out.push(FieldChange::new("price", lock.price, cfg.price));
        }
        description_change(cfg.description.as_deref(), lock.description.as_deref(), out);
        bool_change("for_sale", cfg.for_sale, lock.for_sale, out);
        bool_change(
            "regional_pricing",
            cfg.regional_pricing,
            lock.regional_pricing,
            out,
        );
        bool_change("store_page", cfg.store_page, lock.store_page, out);
    }
}
