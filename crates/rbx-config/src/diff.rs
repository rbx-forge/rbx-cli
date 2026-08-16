/// Diff between local entries (configs.toml) and live entries (Roblox API).
use std::collections::BTreeMap;

use colored::Colorize;
use serde_json::Value as Json;

use crate::value::{canonical, compact};

#[derive(Debug)]
pub struct Change {
    pub key: String,
    pub kind: ChangeKind,
    pub old_value: Option<Json>,
    pub new_value: Option<Json>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Add,
    Update,
    Remove,
}

#[derive(Debug)]
pub struct Diff {
    pub changes: Vec<Change>,
    pub unchanged: Vec<String>,
}

impl Diff {
    pub fn compute(local: &BTreeMap<String, Json>, remote: &BTreeMap<String, Json>) -> Self {
        let mut changes = Vec::new();
        let mut unchanged = Vec::new();

        // Adds + updates
        for (key, new_val) in local {
            match remote.get(key) {
                None => changes.push(Change {
                    key: key.clone(),
                    kind: ChangeKind::Add,
                    old_value: None,
                    new_value: Some(new_val.clone()),
                }),
                Some(old_val) if canonical(old_val) != canonical(new_val) => changes.push(Change {
                    key: key.clone(),
                    kind: ChangeKind::Update,
                    old_value: Some(old_val.clone()),
                    new_value: Some(new_val.clone()),
                }),
                _ => unchanged.push(key.clone()),
            }
        }

        // Removes (in remote but not in local)
        for (key, old_val) in remote {
            if !local.contains_key(key) {
                changes.push(Change {
                    key: key.clone(),
                    kind: ChangeKind::Remove,
                    old_value: Some(old_val.clone()),
                    new_value: None,
                });
            }
        }

        // Sort: add < update < remove, then alphabetical within each kind
        changes.sort_by(|a, b| {
            let order = |k: &ChangeKind| match k {
                ChangeKind::Add => 0,
                ChangeKind::Update => 1,
                ChangeKind::Remove => 2,
            };
            order(&a.kind).cmp(&order(&b.kind)).then(a.key.cmp(&b.key))
        });
        unchanged.sort();

        Diff { changes, unchanged }
    }

    pub fn print(&self) {
        if self.changes.is_empty() {
            println!("  (no changes — local matches live)");
            return;
        }
        for c in &self.changes {
            match c.kind {
                ChangeKind::Add => println!(
                    "  {} {} = {}",
                    "+".green().bold(),
                    c.key.bold(),
                    compact(c.new_value.as_ref().unwrap()).green()
                ),
                ChangeKind::Remove => println!(
                    "  {} {} = {}",
                    "-".red().bold(),
                    c.key.bold(),
                    compact(c.old_value.as_ref().unwrap()).red()
                ),
                ChangeKind::Update => println!(
                    "  {} {}: {} → {}",
                    "~".yellow().bold(),
                    c.key.bold(),
                    compact(c.old_value.as_ref().unwrap()).dimmed(),
                    compact(c.new_value.as_ref().unwrap()).yellow()
                ),
            }
        }
        println!(
            "  ({} change(s), {} unchanged)",
            self.changes.len(),
            self.unchanged.len()
        );
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}
