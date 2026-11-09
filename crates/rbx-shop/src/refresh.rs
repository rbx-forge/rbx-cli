//! What `sync` plans against: one env's lock section, brought up to date with
//! what Roblox lists.
//!
//! Three sides. The config says what is wanted, the lockfile what this
//! repository last saw Roblox hold, and the listing what Roblox holds now.
//! `sync` writes the difference between the first and the third, so a price
//! edited on the Creator Dashboard is seen, and not reported as up to date
//! because the two local files agree.
//!
//! The lockfile stays what says which resource is which: a config key is tied
//! to a Roblox id there and nowhere else, so a resource renamed on Roblox is
//! still the same resource, and one made by hand, which no entry tracks, is
//! not `sync`'s to touch. It is also what tells a change made here from a
//! change made there, which `sync` asks of the result through
//! `live::changed_fields`.

use crate::config::ResourceKind;
use crate::live::{gone, RemoteLocks};
use crate::lockfile::{BadgeLock, EnvLock, PassLock, ProductLock};

/// One env's lock section as Roblox stands now.
#[derive(Debug, PartialEq)]
pub(crate) struct Refreshed {
    /// What the plan compares the config with, and what the lockfile records
    /// once the run is allowed to write.
    pub lock: EnvLock,
    /// Resources the lockfile tracks and Roblox does not list, one line each.
    /// Their entries are kept as recorded: an id that stopped being listed is
    /// not proof the resource is gone, and dropping the entry would make the
    /// next `sync` create a second one.
    pub gone: Vec<String>,
}

/// Bring one env's lock section up to date with what Roblox listed, for the
/// kinds that were read. A kind left out of `kinds` was not listed, and stays
/// as recorded.
///
/// The icon is left as recorded for every kind. Its hash describes a local
/// file, which only a `sync` or a `pull` can tie to the icon Roblox holds, so
/// an icon replaced on Roblox is reported by the `shop/live` check and
/// settled by `pull`, with `--accept-remote` or `--accept-local`.
pub(crate) fn refresh(
    recorded: &EnvLock,
    remote: &RemoteLocks,
    kinds: &[ResourceKind],
) -> Refreshed {
    let mut lock = recorded.clone();
    let mut missing = Vec::new();

    if kinds.contains(&ResourceKind::Pass) {
        for (key, entry) in &mut lock.passes {
            match remote.passes.get(&entry.id) {
                Some(live) => {
                    *entry = PassLock {
                        icon_asset_id: entry.icon_asset_id,
                        icon_hash: entry.icon_hash.take(),
                        ..live.clone()
                    }
                }
                None => missing.push(gone(ResourceKind::Pass, key, entry.id)),
            }
        }
    }

    if kinds.contains(&ResourceKind::Badge) {
        for (key, entry) in &mut lock.badges {
            match remote.badges.get(&entry.id) {
                Some(live) => {
                    *entry = BadgeLock {
                        icon_asset_id: entry.icon_asset_id,
                        icon_hash: entry.icon_hash.take(),
                        ..live.clone()
                    }
                }
                None => missing.push(gone(ResourceKind::Badge, key, entry.id)),
            }
        }
    }

    if kinds.contains(&ResourceKind::Product) {
        for (key, entry) in &mut lock.products {
            match remote.products.get(&entry.id) {
                Some(live) => {
                    *entry = ProductLock {
                        icon_asset_id: entry.icon_asset_id,
                        icon_hash: entry.icon_hash.take(),
                        ..live.clone()
                    }
                }
                None => missing.push(gone(ResourceKind::Product, key, entry.id)),
            }
        }
    }

    Refreshed {
        lock,
        gone: missing,
    }
}

#[cfg(test)]
mod tests;
