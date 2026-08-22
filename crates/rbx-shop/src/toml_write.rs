//! Comment-preserving write-back for `rbxshop.toml`.
//!
//! `pull` and `rename` used to rewrite the file with
//! `toml::to_string_pretty(&Config)`, which meant every write dropped the
//! user's comments, reordered their keys, and deleted any key the model does
//! not have a field for. The project already documents that exact failure as a
//! removed bug in another tool (`docs/env.md`, on the `rbxplace.toml`
//! generator): "it reserialized the document through serde, which dropped
//! comments, reordered keys, and silently deleted any field it did not model".
//!
//! So this module edits the existing document instead, in the style
//! `rbx-meta`'s pull already uses. Two consequences are load-bearing:
//!
//! - **Only the tables the shop owns are touched**: `passes`, `badges`,
//!   `products` and the `envs.<name>` overlays. Anything else in the file,
//!   modeled or not, is left exactly as the user wrote it.
//! - **Within those tables only the modeled keys are written**, so an
//!   unrecognised key inside a resource survives the round trip too. It is
//!   reported on load (see `config::warn_unknown_root_keys`) rather than
//!   quietly deleted here.
//!
//! Booleans with a default are written only when they are already in the file
//! or when they diverge from that default, so a pull does not sprinkle
//! `for_sale = true` through a file that never mentioned it.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::{value, DocumentMut, Item, Table};

use crate::config::{
    BadgeConfig, BadgeOverlay, Config, EnvOverlay, PassConfig, PassOverlay, ProductConfig,
    ProductOverlay, ResourceKind,
};

/// A key that moved, so the write-back can carry the entry's comments and its
/// place in the file across to the new name instead of dropping the old table
/// and appending a fresh one.
///
/// Only `rbx shop rename` produces these.
#[derive(Debug, Clone)]
pub struct KeyRename {
    pub kind: ResourceKind,
    pub from: String,
    pub to: String,
}

/// Write `config`'s resource tables back into the document at `path`.
///
/// The file must already exist: this is a write-*back*. Creating a config
/// from nothing is `Config::save`, which serialises the whole model.
pub fn save_in_place(config: &Config, path: &Path, renames: &[KeyRename]) -> Result<()> {
    let original = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut doc: DocumentMut = original
        .parse()
        .with_context(|| format!("Failed to parse {} as TOML", path.display()))?;

    write_resource_tables(&mut doc, config, renames);

    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

fn write_resource_tables(doc: &mut DocumentMut, config: &Config, renames: &[KeyRename]) {
    let root = doc.as_table_mut();

    sync_entries(
        root,
        "passes",
        &config.passes,
        &renames_for(renames, ResourceKind::Pass),
        write_pass,
    );
    sync_entries(
        root,
        "badges",
        &config.badges,
        &renames_for(renames, ResourceKind::Badge),
        write_badge,
    );
    sync_entries(
        root,
        "products",
        &config.products,
        &renames_for(renames, ResourceKind::Product),
        write_product,
    );

    write_envs(root, config, renames);
}

fn renames_for(renames: &[KeyRename], kind: ResourceKind) -> Vec<(&str, &str)> {
    renames
        .iter()
        .filter(|r| r.kind == kind)
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect()
}

// ---------------------------------------------------------------------------
// envs.<name> overlays
// ---------------------------------------------------------------------------

fn write_envs(root: &mut Table, config: &Config, renames: &[KeyRename]) {
    if config.envs.values().all(EnvOverlay::is_empty) && !root.contains_key("envs") {
        return;
    }

    let envs = ensure_table(root, "envs");
    for (name, overlay) in &config.envs {
        if overlay.is_empty() {
            envs.remove(name);
            continue;
        }
        let env_table = ensure_table(envs, name);
        sync_entries(
            env_table,
            "passes",
            &overlay.passes,
            &renames_for(renames, ResourceKind::Pass),
            write_pass_overlay,
        );
        sync_entries(
            env_table,
            "badges",
            &overlay.badges,
            &renames_for(renames, ResourceKind::Badge),
            write_badge_overlay,
        );
        sync_entries(
            env_table,
            "products",
            &overlay.products,
            &renames_for(renames, ResourceKind::Product),
            write_product_overlay,
        );
    }

    // Envs the config no longer has at all.
    let stale: Vec<String> = envs
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !config.envs.contains_key(k))
        .collect();
    for k in stale {
        envs.remove(&k);
    }

    if envs.is_empty() {
        root.remove("envs");
    }
}

// ---------------------------------------------------------------------------
// One `[<section>.<key>]` map
// ---------------------------------------------------------------------------

/// Reconcile `parent[section]` against `entries`: rename first (so comments and
/// file position travel with the entry), then write each entry's modeled keys,
/// then drop the tables the config no longer declares.
fn sync_entries<T>(
    parent: &mut Table,
    section: &str,
    entries: &BTreeMap<String, T>,
    renames: &[(&str, &str)],
    write: fn(&mut Table, &T),
) {
    if entries.is_empty() {
        parent.remove(section);
        return;
    }

    let table = ensure_table(parent, section);

    for (from, to) in renames {
        move_entry(table, from, to);
    }

    for (key, entry) in entries {
        let entry_table = ensure_table(table, key);
        write(entry_table, entry);
    }

    let stale: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !entries.contains_key(k))
        .collect();
    for key in stale {
        table.remove(&key);
    }
}

/// Move a table to a new key, item and all. The `Item` carries the table's own
/// decor (the comment written above `[passes.VIP]`) and its parsed position,
/// so a rename keeps both instead of appending a bare table at the end.
fn move_entry(table: &mut Table, from: &str, to: &str) {
    if from == to || !table.contains_key(from) || table.contains_key(to) {
        return;
    }
    if let Some(item) = table.remove(from) {
        table.insert(to, item);
    }
}

fn ensure_table<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
    if !parent.contains_key(key) {
        let mut fresh = Table::new();
        // `[passes]` is never written on its own (only `[passes.VIP]`) so a
        // section this function had to create stays implicit.
        fresh.set_implicit(true);
        parent.insert(key, Item::Table(fresh));
    }
    parent[key]
        .as_table_mut()
        .expect("rbxshop.toml section is not a table")
}

// ---------------------------------------------------------------------------
// Per-resource field writers
// ---------------------------------------------------------------------------

fn write_pass(t: &mut Table, cfg: &PassConfig) {
    set_opt_str(t, "name", cfg.name.as_deref());
    set_opt_int(t, "price", cfg.price);
    set_opt_str(t, "description", cfg.description.as_deref());
    set_opt_path(t, "icon", cfg.icon.as_deref());
    set_bool(t, "for_sale", cfg.for_sale, true);
    set_bool(t, "regional_pricing", cfg.regional_pricing, false);
    set_bool(t, "create_gift", cfg.create_gift, false);
    set_opt_str(t, "path", cfg.path.as_deref());
}

fn write_badge(t: &mut Table, cfg: &BadgeConfig) {
    set_opt_str(t, "name", cfg.name.as_deref());
    set_opt_str(t, "description", cfg.description.as_deref());
    set_opt_path(t, "icon", cfg.icon.as_deref());
    set_bool(t, "enabled", cfg.enabled, true);
    set_opt_str(t, "path", cfg.path.as_deref());
}

fn write_product(t: &mut Table, cfg: &ProductConfig) {
    set_opt_str(t, "name", cfg.name.as_deref());
    // Required by the model, so always written.
    t["price"] = value(cfg.price as i64);
    set_opt_str(t, "description", cfg.description.as_deref());
    set_opt_path(t, "icon", cfg.icon.as_deref());
    set_bool(t, "for_sale", cfg.for_sale, true);
    set_bool(t, "regional_pricing", cfg.regional_pricing, false);
    set_bool(t, "store_page", cfg.store_page, false);
    set_bool(t, "create_gift", cfg.create_gift, false);
    set_opt_str(t, "path", cfg.path.as_deref());
}

fn write_pass_overlay(t: &mut Table, ov: &PassOverlay) {
    set_opt_str(t, "name", ov.name.as_deref());
    set_opt_int(t, "price", ov.price);
    set_opt_str(t, "description", ov.description.as_deref());
    set_opt_path(t, "icon", ov.icon.as_deref());
    set_opt_bool(t, "for_sale", ov.for_sale);
    set_opt_bool(t, "regional_pricing", ov.regional_pricing);
    set_opt_bool(t, "create_gift", ov.create_gift);
    set_opt_str(t, "path", ov.path.as_deref());
}

fn write_badge_overlay(t: &mut Table, ov: &BadgeOverlay) {
    set_opt_str(t, "name", ov.name.as_deref());
    set_opt_str(t, "description", ov.description.as_deref());
    set_opt_path(t, "icon", ov.icon.as_deref());
    set_opt_bool(t, "enabled", ov.enabled);
    set_opt_str(t, "path", ov.path.as_deref());
}

fn write_product_overlay(t: &mut Table, ov: &ProductOverlay) {
    set_opt_str(t, "name", ov.name.as_deref());
    set_opt_int(t, "price", ov.price);
    set_opt_str(t, "description", ov.description.as_deref());
    set_opt_path(t, "icon", ov.icon.as_deref());
    set_opt_bool(t, "for_sale", ov.for_sale);
    set_opt_bool(t, "regional_pricing", ov.regional_pricing);
    set_opt_bool(t, "store_page", ov.store_page);
    set_opt_bool(t, "create_gift", ov.create_gift);
    set_opt_str(t, "path", ov.path.as_deref());
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

fn set_opt_str(t: &mut Table, key: &str, val: Option<&str>) {
    match val {
        Some(v) => t[key] = value(v),
        None => {
            t.remove(key);
        }
    }
}

fn set_opt_int(t: &mut Table, key: &str, val: Option<u64>) {
    match val {
        Some(v) => t[key] = value(v as i64),
        None => {
            t.remove(key);
        }
    }
}

fn set_opt_bool(t: &mut Table, key: &str, val: Option<bool>) {
    match val {
        Some(v) => t[key] = value(v),
        None => {
            t.remove(key);
        }
    }
}

fn set_opt_path(t: &mut Table, key: &str, val: Option<&Path>) {
    match val {
        Some(p) => t[key] = value(path_to_toml_str(p)),
        None => {
            t.remove(key);
        }
    }
}

/// A bool whose absence already means `default`: update it where the user
/// wrote it, add it only when it diverges, never introduce noise.
fn set_bool(t: &mut Table, key: &str, val: bool, default: bool) {
    if t.contains_key(key) || val != default {
        t[key] = value(val);
    }
}

/// Backslashes are legal in a TOML basic string only as escapes, and a Windows
/// path written verbatim would either fail to parse or silently change meaning.
fn path_to_toml_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
