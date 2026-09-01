//! What a pull decides to write into the config, per resource kind.
//!
//! `Pullable` is the seam: passes, badges and products differ in which fields
//! exist and where they live, and agree on everything else. The three impls sit
//! together so a field added to one is visibly absent from the others.

use std::collections::BTreeMap;

use crate::config::{
    find_overlay_owner, find_owner, BadgeConfig, BadgeOverlay, Config, ConfigFile, EnvOverlay,
    PassConfig, PassOverlay, ProductConfig, ProductOverlay, ResourceKind,
};
use crate::gifts::is_gift_key;
use crate::lockfile::{BadgeLock, PassLock, ProductLock};

#[derive(Debug)]
pub(super) struct ConfigChange {
    pub(super) key: String,
    pub(super) kind: ChangeKind,
}

#[derive(Debug)]
pub(super) enum ChangeKind {
    AddedToBase,
    AddedToOverlay(String),
    OverlayDiverged { fields: Vec<String> },
    OverlayCleared,
    BaseUpdated { fields: Vec<String> },
}

/// Convert lock name to config name field: None if name == key (TOML convention).
pub(super) fn config_name(lock_name: &str, key: &str) -> Option<String> {
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
pub(super) trait Pullable {
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

pub(super) fn compute_config_changes<K: Pullable>(
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

pub(super) fn apply_config_changes<K: Pullable>(
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
pub(super) fn diverged<T: Clone + PartialEq>(base: &T, remote: &T) -> Option<T> {
    (base != remote).then(|| remote.clone())
}

/// Names of the fields a `Vec<(name, is_set)>` says are set, in order.
pub(super) fn set_fields(fields: &[(&str, bool)]) -> Vec<String> {
    fields
        .iter()
        .filter(|(_, set)| *set)
        .map(|(name, _)| (*name).to_string())
        .collect()
}

// ---- Passes ---------------------------------------------------------------

pub(super) struct PassKind;

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

pub(super) struct BadgeKind;

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

pub(super) struct ProductKind;

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

pub(super) fn cleanup_empty_overlays(files: &mut [ConfigFile], env: &str) {
    for file in files.iter_mut() {
        if let Some(ov) = file.config.envs.get(env) {
            if ov.is_empty() {
                file.config.envs.remove(env);
            }
        }
    }
}
