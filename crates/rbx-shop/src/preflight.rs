//! The question `sync` asks Roblox before it creates anything: does a resource
//! by this name already exist?
//!
//! Why it is worth a round trip. `diff` decides `Action::Create` from one
//! fact (the key is absent from `rbxshop.lock.toml`) which is correct right
//! up until the lockfile is not the record it is assumed to be. The lockfile
//! not being committed is the ordinary way that happens: it lands in
//! `.gitignore` by mistake, or a branch is cut before it was added, and the
//! next `sync` on a clean checkout sees an empty lock and plans to create
//! every resource the config declares. They already exist.
//!
//! What makes that worth stopping rather than warning about is that it cannot
//! be undone. Roblox has no delete for a game pass or a developer product;
//! the best available repair is setting the accidental twin to `for_sale =
//! false`, which leaves it in the experience forever, visible to anyone who
//! already owns it. Badges can be disabled and not removed. So the cheap check
//! runs before the expensive mistake, and a duplicate name stops the run.
//!
//! It is deliberately a *name* check and not an existence check, because the
//! display name is the only thing local config and a remote resource have in
//! common when the lockfile is gone. That makes it a heuristic in one
//! direction: two resources may legitimately share a name (see
//! `collision.rs`, which handles the same ambiguity coming the other way), so
//! `--allow-duplicate-names` exists for the developer who means it.
//!
//! It reads nothing itself. `sync` lists the experience before it plans, and
//! the check runs on that listing, so a dry run makes it too: a plan that says
//! "create" has already been compared with what exists.

use anyhow::{bail, Result};
use colored::Colorize;

use crate::api::RbxClient;
use crate::config::{resolve_name, ResolvedResources, ResourceKind};
use crate::diff::{Action, SyncPlan};
use crate::live::RemoteLocks;

/// A resource the plan would create, whose display name a remote resource
/// already answers to.
#[derive(Debug)]
pub(crate) struct Collision {
    pub kind: ResourceKind,
    /// The key in `rbxshop.toml`.
    pub key: String,
    /// The name Roblox would be asked to create, which is what matched.
    pub display_name: String,
    /// Every remote id carrying that name. More than one means the experience
    /// already has duplicates, which is worth showing rather than hiding
    /// behind the first.
    pub remote_ids: Vec<u64>,
}

/// The keys this plan would create for one kind, paired with the display name
/// Roblox would receive.
///
/// The plan carries config keys; the API compares names. `resolve_name` is the
/// same resolution `apply_kind` performs, and calling it here rather than
/// re-deriving it is what keeps the check honest when a resource sets an
/// explicit `name`.
fn pending_creates(
    plan: &SyncPlan,
    resources: &ResolvedResources,
    kind: ResourceKind,
) -> Vec<(String, String)> {
    plan.actions(kind)
        .iter()
        .filter(|a| matches!(a.action, Action::Create))
        .map(|a| {
            let key = a.name.as_str();
            let configured = match kind {
                ResourceKind::Pass => resources.passes.get(key).and_then(|c| c.name.as_deref()),
                ResourceKind::Badge => resources.badges.get(key).and_then(|c| c.name.as_deref()),
                ResourceKind::Product => {
                    resources.products.get(key).and_then(|c| c.name.as_deref())
                }
            };
            (key.to_string(), resolve_name(configured, key).to_string())
        })
        .collect()
}

/// Remote display names for one kind, as `(name, id)` pairs.
///
/// Returns the pairs rather than a map because names are not unique remotely:
/// building a map here would silently drop exactly the duplicates this module
/// exists to report.
fn remote_names(remote: &RemoteLocks, kind: ResourceKind) -> Vec<(String, u64)> {
    match kind {
        ResourceKind::Pass => remote
            .passes
            .iter()
            .map(|(id, lock)| (lock.name.clone(), *id))
            .collect(),
        ResourceKind::Badge => remote
            .badges
            .iter()
            .map(|(id, lock)| (lock.name.clone(), *id))
            .collect(),
        ResourceKind::Product => remote
            .products
            .iter()
            .map(|(id, lock)| (lock.name.clone(), *id))
            .collect(),
    }
}

/// Every pending create whose name a remote resource already answers to.
///
/// `kinds` is the set `--only` left in play; a kind excluded from the sync was
/// not listed, and nothing will be created in it.
pub(crate) fn find_collisions(
    plan: &SyncPlan,
    resources: &ResolvedResources,
    kinds: &[ResourceKind],
    remote: &RemoteLocks,
) -> Vec<Collision> {
    let mut collisions = Vec::new();

    for kind in ResourceKind::ALL {
        if !kinds.contains(&kind) {
            continue;
        }
        let creates = pending_creates(plan, resources, kind);
        if creates.is_empty() {
            continue;
        }
        collisions.extend(collide(kind, &creates, &remote_names(remote, kind)));
    }

    collisions
}

/// The matching itself, with no network in it.
///
/// Separated from `find_collisions` because this is where the judgement calls
/// live (what counts as the same name, and what gets reported when several
/// remote resources answer to it) and those deserve tests that do not need a
/// mock server to state.
fn collide(
    kind: ResourceKind,
    creates: &[(String, String)],
    remote: &[(String, u64)],
) -> Vec<Collision> {
    creates
        .iter()
        .filter_map(|(key, display_name)| {
            // Case-insensitive: a name differing only in case is far more
            // likely to be the resource the lockfile lost track of than a
            // deliberate second one, and the cost of the two answers is not
            // symmetric: a false stop is a flag away, a false create is
            // permanent.
            let remote_ids: Vec<u64> = remote
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case(display_name))
                .map(|(_, id)| *id)
                .collect();
            (!remote_ids.is_empty()).then(|| Collision {
                kind,
                key: key.clone(),
                display_name: display_name.clone(),
                remote_ids,
            })
        })
        .collect()
}

/// Turn collisions into the error that stops the sync.
///
/// The remedy named is `rbx shop pull` and not a block of TOML to paste: a
/// lock entry carries an icon hash and a price the developer would have to
/// look up, and `pull` already knows how to adopt a remote resource into the
/// lockfile under the key the config uses. Naming the command that repairs the
/// state beats naming the file that is wrong.
pub(crate) fn refuse(collisions: &[Collision], env: &str) -> anyhow::Error {
    let mut out = String::new();

    out.push_str(&format!(
        "{} resource{} would be created under {} that already exist{} on Roblox:\n\n",
        collisions.len(),
        if collisions.len() == 1 { "" } else { "s" },
        "a name".bold(),
        if collisions.len() == 1 { "s" } else { "" },
    ));

    for c in collisions {
        let ids = c
            .remote_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "  {} '{}' (key '{}'): already id {}\n",
            c.kind, c.display_name, c.key, ids
        ));
    }

    out.push_str(&format!(
        "\nThe usual cause is a {} that was never committed, which makes every \
         resource look new.\n\n\
         Adopt what already exists, then sync:\n    \
         rbx shop pull --env {}\n\n\
         If these really are meant to be new resources with the same names, \
         re-run with {}. Passes and products cannot be deleted once created.",
        crate::lockfile::LOCKFILE_NAME.bold(),
        env,
        "--allow-duplicate-names".bold(),
    ));

    anyhow::anyhow!(out)
}

/// Run the check on what `sync` listed, and stop the sync if it finds
/// anything.
pub(crate) fn guard(
    plan: &SyncPlan,
    resources: &ResolvedResources,
    kinds: &[ResourceKind],
    remote: &RemoteLocks,
    env: &str,
    allow_duplicate_names: bool,
) -> Result<()> {
    if allow_duplicate_names {
        return Ok(());
    }

    let collisions = find_collisions(plan, resources, kinds, remote);
    if collisions.is_empty() {
        return Ok(());
    }

    bail!(refuse(&collisions, env))
}

/// What the free badge quota means for a plan creating `creating` badges.
#[derive(Debug, PartialEq)]
enum QuotaVerdict {
    /// The quota covers every create.
    Enough,
    /// Short, and `--badge-cost` says spending was asked for.
    ShortButPaying,
    /// Short, and every create past the quota would be refused.
    Refuse,
}

/// The decision, with no network in it.
fn quota_verdict(creating: u64, remaining: u64, badge_cost: u64) -> QuotaVerdict {
    if creating <= remaining {
        QuotaVerdict::Enough
    } else if badge_cost > 0 {
        QuotaVerdict::ShortButPaying
    } else {
        QuotaVerdict::Refuse
    }
}

/// Stop a sync whose badge creates the free quota cannot cover, before it has
/// written anything.
///
/// Passes are applied before badges and products after them, so finding this
/// out from Roblox leaves an env holding the passes and some of the badges,
/// with no products at all, and code generated against the whole config.
///
/// A failed read is not a verdict: the quota is a courtesy check on a route
/// nothing else depends on, and Roblox still refuses a create past it, so the
/// run carries on with a warning.
pub(crate) async fn guard_badge_quota(
    client: &RbxClient,
    universe_id: u64,
    plan: &SyncPlan,
    badge_cost: u64,
) -> Result<()> {
    let creating = plan
        .actions(ResourceKind::Badge)
        .iter()
        .filter(|a| matches!(a.action, Action::Create))
        .count() as u64;

    let remaining = match client.free_badge_quota(universe_id).await {
        Ok(remaining) => remaining,
        Err(e) => {
            println!(
                "{} Could not read the free badge quota, so it was not checked: {e:#}",
                "!".yellow()
            );
            return Ok(());
        }
    };

    match quota_verdict(creating, remaining, badge_cost) {
        QuotaVerdict::Enough => Ok(()),
        QuotaVerdict::ShortButPaying => {
            println!(
                "{} {creating} badge(s) to create and {remaining} free left: \
                 --badge-cost {badge_cost} applies past the quota.",
                "!".yellow()
            );
            Ok(())
        }
        QuotaVerdict::Refuse => bail!(
            "This sync would create {creating} badge(s), and this universe has {remaining} \
             free badge create(s) left. Nothing was changed.\n\n\
             Sync again once Roblox has refilled the quota, or leave badges out for now:\n    \
             rbx shop sync --only passes,products\n\n\
             To pay for the extra badges, pass {}.",
            "--badge-cost".bold(),
        ),
    }
}

#[cfg(test)]
mod tests;
