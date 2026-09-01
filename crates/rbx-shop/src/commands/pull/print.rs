//! What the run says it did, in the human form.

use std::collections::BTreeMap;

use colored::Colorize;

use crate::config::ResourceKind;
use crate::lockfile::{BadgeLock, EnvLock, PassLock, ProductLock};

use super::*;

pub(super) fn print_config_changes(kind: ResourceKind, changes: &[ConfigChange]) {
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

pub(super) struct LockfileDiff {
    new_passes: Vec<String>,
    new_badges: Vec<String>,
    new_products: Vec<String>,
    removed_passes: Vec<String>,
    removed_badges: Vec<String>,
    removed_products: Vec<String>,
}

impl LockfileDiff {
    pub(super) fn is_empty(&self) -> bool {
        self.new_passes.is_empty()
            && self.new_badges.is_empty()
            && self.new_products.is_empty()
            && self.removed_passes.is_empty()
            && self.removed_badges.is_empty()
            && self.removed_products.is_empty()
    }
}

pub(super) fn lockfile_diff(
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

pub(super) fn print_lockfile_diff(diff: &LockfileDiff) {
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
