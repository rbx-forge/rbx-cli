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
//! It runs only when the plan contains a create. A sync that only updates
//! resources needs no read scope it did not need yesterday.

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::api::RbxClient;
use crate::config::{resolve_name, ResolvedResources, ResourceKind};
use crate::diff::{Action, SyncPlan};

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
///
/// Every field on the API models is optional, and a remote entry missing
/// either its name or its id is dropped: it cannot be matched against, and it
/// cannot be named in the error if it were. Dropping it costs a collision the
/// check would not have found; keeping it would cost a report that points at
/// nothing.
async fn remote_names(
    client: &RbxClient,
    universe_id: u64,
    kind: ResourceKind,
) -> Result<Vec<(String, u64)>> {
    let pairs: Vec<(Option<String>, Option<u64>)> = match kind {
        ResourceKind::Pass => client
            .list_all_game_passes()
            .await?
            .into_iter()
            .map(|p| (p.name, p.id))
            .collect(),
        ResourceKind::Badge => client
            .list_all_badges(universe_id)
            .await?
            .into_iter()
            .map(|b| (b.name, b.id))
            .collect(),
        ResourceKind::Product => client
            .list_all_developer_products()
            .await?
            .into_iter()
            .map(|p| (p.name, p.id))
            .collect(),
    };

    Ok(pairs
        .into_iter()
        .filter_map(|(name, id)| Some((name?, id?)))
        .collect())
}

/// Every pending create whose name a remote resource already answers to.
///
/// `kinds` is the set `--only` left in play; a kind excluded from the sync is
/// not listed, because nothing will be created in it.
pub(crate) async fn find_collisions(
    client: &RbxClient,
    universe_id: u64,
    plan: &SyncPlan,
    resources: &ResolvedResources,
    kinds: &[ResourceKind],
) -> Result<Vec<Collision>> {
    let mut collisions = Vec::new();

    for kind in ResourceKind::ALL {
        if !kinds.contains(&kind) {
            continue;
        }
        let creates = pending_creates(plan, resources, kind);
        if creates.is_empty() {
            continue;
        }

        // The listing needs a read scope, and a create-only sync is exactly
        // where it runs, so a key with write-but-not-read scope that worked
        // yesterday fails here today. The failure is kept (a guard that gave up
        // quietly would not be a guard) but the message has to name the way
        // past it, or the reader is left with a scope error and no route.
        let remote = remote_names(client, universe_id, kind)
            .await
            .with_context(|| {
                format!(
                    "listing the experience's {} to check for a name that \
                     already exists.\n\nThis runs before any create, because a \
                     duplicate pass or product cannot be deleted. If this key \
                     cannot be given read access, `--allow-duplicate-names` \
                     skips the check",
                    kind.plural()
                )
            })?;
        collisions.extend(collide(kind, &creates, &remote));
    }

    Ok(collisions)
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

/// Run the check and stop the sync if it finds anything.
pub(crate) async fn guard(
    client: &RbxClient,
    universe_id: u64,
    plan: &SyncPlan,
    resources: &ResolvedResources,
    kinds: &[ResourceKind],
    env: &str,
    allow_duplicate_names: bool,
) -> Result<()> {
    if allow_duplicate_names {
        return Ok(());
    }

    let collisions = find_collisions(client, universe_id, plan, resources, kinds).await?;
    if collisions.is_empty() {
        return Ok(());
    }

    bail!(refuse(&collisions, env))
}

#[cfg(test)]
mod tests;
