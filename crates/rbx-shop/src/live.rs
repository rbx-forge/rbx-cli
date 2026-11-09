//! `shop/live`: what Roblox lists, against what the lockfile last recorded.
//!
//! Against the lockfile, not the config. A change written in `rbxshop.toml`
//! and not synced yet is `shop/lockfile`'s to report; comparing the config
//! here would report it a second time, as if Roblox had drifted. The lockfile
//! holds what the last `sync` or `pull` confirmed Roblox had, so a difference
//! here is a change made on Roblox's side since then: a price edited on the
//! Creator Dashboard, a pass taken off sale, an item created by hand.
//!
//! Remote resources go through the conversion `pull` uses to write the
//! lockfile, so the two cannot disagree about what Roblox said.

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use reqwest::StatusCode;

use crate::api::RbxClient;
use crate::lockfile::{BadgeLock, EnvLock, PassLock, ProductLock};

/// What one env's comparison found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LiveReport {
    /// One line per difference between Roblox and the lockfile.
    pub differences: Vec<String>,
    /// What this run could not compare. A clean run is clean only for the
    /// rest.
    pub not_compared: Vec<String>,
}

/// Every resource Roblox has for one universe, as lock entries keyed by id.
#[derive(Debug, Default)]
pub(crate) struct RemoteLocks {
    pub passes: BTreeMap<u64, PassLock>,
    pub badges: BTreeMap<u64, BadgeLock>,
    pub products: BTreeMap<u64, ProductLock>,
}

/// Compare one env's lock section with what Roblox lists now.
pub async fn compare(api_key: String, env_lock: &EnvLock) -> Result<LiveReport> {
    let client = RbxClient::new(Some(api_key), env_lock.universe_id, true);
    let remote = read(&client, env_lock).await?;
    Ok(compare_read(env_lock, &remote))
}

/// The reads `pull` makes, converted the way `pull` converts them.
async fn read(client: &RbxClient, env_lock: &EnvLock) -> Result<RemoteLocks> {
    let mut remote = RemoteLocks::default();

    // `prior` matters only for regional pricing, which is kept from the
    // lockfile rather than read: see `remote::pass_lock`.
    let pass_by_id: BTreeMap<u64, &PassLock> =
        env_lock.passes.values().map(|l| (l.id, l)).collect();
    for pass in client.list_all_game_passes().await? {
        if let Some(id) = pass.id {
            let prior = pass_by_id.get(&id).copied();
            remote
                .passes
                .insert(id, crate::remote::pass_lock(&pass, id, prior));
        }
    }

    let product_by_id: BTreeMap<u64, &ProductLock> =
        env_lock.products.values().map(|l| (l.id, l)).collect();
    for product in client.list_all_developer_products().await? {
        if let Some(id) = product.id {
            let prior = product_by_id.get(&id).copied();
            remote
                .products
                .insert(id, crate::remote::product_lock(&product, id, prior));
        }
    }

    for badge in client.list_all_badges(env_lock.universe_id).await? {
        if let Some(id) = badge.id {
            remote
                .badges
                .insert(id, crate::remote::badge_lock(&badge, id, true));
        }
    }
    // Disabled badges are not listed. Read the locked ones the listing left
    // out by id, as `pull` does, or every disabled badge would read as gone.
    let listed: HashSet<u64> = remote.badges.keys().copied().collect();
    for locked in env_lock.badges.values() {
        if listed.contains(&locked.id) {
            continue;
        }
        match client.get_badge(locked.id).await {
            Ok(badge) => {
                remote.badges.insert(
                    locked.id,
                    crate::remote::badge_lock(&badge, locked.id, false),
                );
            }
            // Gone, which `compare_read` reports.
            Err(error) if rbx_core::api::is_api_status(&error, StatusCode::NOT_FOUND) => {}
            Err(error) => return Err(error),
        }
    }

    Ok(remote)
}

/// The comparison itself, apart from the network.
pub(crate) fn compare_read(env_lock: &EnvLock, remote: &RemoteLocks) -> LiveReport {
    let mut out = Vec::new();

    let mut tracked = HashSet::new();
    for (key, locked) in &env_lock.passes {
        tracked.insert(locked.id);
        match remote.passes.get(&locked.id) {
            Some(live) => compare_pass(key, locked, live, &mut out),
            None => out.push(gone("pass", key, locked.id)),
        }
    }
    untracked(
        "pass",
        remote.passes.iter().map(|(id, l)| (*id, &l.name)),
        &tracked,
        &mut out,
    );

    let mut tracked = HashSet::new();
    for (key, locked) in &env_lock.products {
        tracked.insert(locked.id);
        match remote.products.get(&locked.id) {
            Some(live) => compare_product(key, locked, live, &mut out),
            None => out.push(gone("product", key, locked.id)),
        }
    }
    untracked(
        "product",
        remote.products.iter().map(|(id, l)| (*id, &l.name)),
        &tracked,
        &mut out,
    );

    let mut tracked = HashSet::new();
    for (key, locked) in &env_lock.badges {
        tracked.insert(locked.id);
        match remote.badges.get(&locked.id) {
            Some(live) => compare_badge(key, locked, live, &mut out),
            None => out.push(gone("badge", key, locked.id)),
        }
    }
    untracked(
        "badge",
        remote.badges.iter().map(|(id, l)| (*id, &l.name)),
        &tracked,
        &mut out,
    );

    LiveReport {
        differences: out,
        not_compared: vec![
            "regional_pricing: Roblox reports it as on whenever managed pricing is".to_string(),
            "icon files: the comparison is on the icon id Roblox holds".to_string(),
        ],
    }
}

/// Destructured without `..`: a field added to `PassLock` does not compile
/// here until it is compared or set aside with a reason.
fn compare_pass(key: &str, locked: &PassLock, live: &PassLock, out: &mut Vec<String>) {
    let PassLock {
        id: _,
        name,
        price,
        description,
        icon_asset_id,
        // A hash of the local file: Roblox knows nothing of it.
        icon_hash: _,
        for_sale,
        // Kept from the lockfile on both sides: see `not_compared`.
        regional_pricing: _,
        managed_pricing,
    } = locked;
    let mut field = Field::new("pass", key, out);
    field.check("name", name, &live.name);
    field.check("price", price, &live.price);
    field.description(description, &live.description);
    field.check("icon", icon_asset_id, &live.icon_asset_id);
    field.check("for_sale", for_sale, &live.for_sale);
    field.recorded("managed_pricing", managed_pricing, &live.managed_pricing);
}

/// As `compare_pass`.
fn compare_product(key: &str, locked: &ProductLock, live: &ProductLock, out: &mut Vec<String>) {
    let ProductLock {
        id: _,
        name,
        price,
        description,
        icon_asset_id,
        icon_hash: _,
        for_sale,
        regional_pricing: _,
        managed_pricing,
        store_page,
    } = locked;
    let mut field = Field::new("product", key, out);
    field.check("name", name, &live.name);
    field.check("price", price, &live.price);
    field.description(description, &live.description);
    field.check("icon", icon_asset_id, &live.icon_asset_id);
    field.check("for_sale", for_sale, &live.for_sale);
    field.recorded("managed_pricing", managed_pricing, &live.managed_pricing);
    field.check("store_page", store_page, &live.store_page);
}

/// As `compare_pass`.
fn compare_badge(key: &str, locked: &BadgeLock, live: &BadgeLock, out: &mut Vec<String>) {
    let BadgeLock {
        id: _,
        name,
        description,
        enabled,
        icon_asset_id,
        icon_hash: _,
    } = locked;
    let mut field = Field::new("badge", key, out);
    field.check("name", name, &live.name);
    field.description(description, &live.description);
    field.check("enabled", enabled, &live.enabled);
    field.check("icon", icon_asset_id, &live.icon_asset_id);
}

fn gone(kind: &str, key: &str, id: u64) -> String {
    format!("{kind} '{key}' ({id}): in the lockfile, not on Roblox")
}

/// Resources Roblox has and no lock entry tracks: made by hand, or by another
/// tool. `rbx shop pull` adopts them.
fn untracked<'a>(
    kind: &str,
    remote: impl Iterator<Item = (u64, &'a String)>,
    tracked: &HashSet<u64>,
    out: &mut Vec<String>,
) {
    for (id, name) in remote {
        if !tracked.contains(&id) {
            out.push(format!(
                "{kind} {name:?} ({id}): on Roblox, not in the lockfile"
            ));
        }
    }
}

/// The differences of one resource, each line naming it.
struct Field<'a> {
    kind: &'static str,
    key: &'a str,
    out: &'a mut Vec<String>,
}

impl<'a> Field<'a> {
    fn new(kind: &'static str, key: &'a str, out: &'a mut Vec<String>) -> Self {
        Self { kind, key, out }
    }

    fn check<T: PartialEq + Show>(&mut self, field: &str, locked: &T, live: &T) {
        if locked != live {
            self.push(field, &locked.show(), &live.show());
        }
    }

    /// An absent description and an empty one are the same thing to Roblox,
    /// the rule `sync`'s own diff follows.
    fn description(&mut self, locked: &Option<String>, live: &Option<String>) {
        let (locked, live) = (
            locked.as_deref().unwrap_or(""),
            live.as_deref().unwrap_or(""),
        );
        if locked != live {
            self.push("description", &format!("{locked:?}"), &format!("{live:?}"));
        }
    }

    /// A field the lockfile may not record yet: a lockfile written before it
    /// existed holds nothing to compare, and is not a difference.
    fn recorded<T: PartialEq + Show>(&mut self, field: &str, locked: &Option<T>, live: &Option<T>) {
        if let (Some(locked), Some(live)) = (locked, live) {
            if locked != live {
                self.push(field, &locked.show(), &live.show());
            }
        }
    }

    fn push(&mut self, field: &str, locked: &str, live: &str) {
        self.out.push(format!(
            "{} '{}' {field}: lockfile {locked}, Roblox {live}",
            self.kind, self.key
        ));
    }
}

/// A value as a line of the report spells it.
trait Show {
    fn show(&self) -> String;
}

impl Show for String {
    fn show(&self) -> String {
        format!("{self:?}")
    }
}

impl Show for u64 {
    fn show(&self) -> String {
        self.to_string()
    }
}

impl Show for bool {
    fn show(&self) -> String {
        self.to_string()
    }
}

impl<T: Show> Show for Option<T> {
    fn show(&self) -> String {
        match self {
            Some(v) => v.show(),
            None => "none".to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
