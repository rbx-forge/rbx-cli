#![allow(clippy::unwrap_used)]

use super::*;

fn pass(id: u64, price: u64) -> PassLock {
    PassLock {
        id,
        name: "VIP".into(),
        price: Some(price),
        description: None,
        icon_asset_id: Some(7),
        icon_hash: Some("local-file-hash".into()),
        for_sale: true,
        regional_pricing: false,
        managed_pricing: Some(false),
    }
}

fn product(id: u64) -> ProductLock {
    ProductLock {
        id,
        name: "Coins".into(),
        price: 99,
        description: Some(String::new()),
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
        managed_pricing: Some(false),
        store_page: true,
    }
}

fn locked(passes: Vec<PassLock>, products: Vec<ProductLock>) -> EnvLock {
    EnvLock {
        universe_id: 1,
        passes: passes.into_iter().map(|p| (p.name.clone(), p)).collect(),
        products: products.into_iter().map(|p| (p.name.clone(), p)).collect(),
        ..Default::default()
    }
}

/// What `remote::*_lock` produces: no icon hash, ever.
fn listed(passes: Vec<PassLock>, products: Vec<ProductLock>) -> RemoteLocks {
    RemoteLocks {
        passes: passes
            .into_iter()
            .map(|p| {
                (
                    p.id,
                    PassLock {
                        icon_hash: None,
                        ..p
                    },
                )
            })
            .collect(),
        products: products
            .into_iter()
            .map(|p| {
                (
                    p.id,
                    ProductLock {
                        icon_hash: None,
                        ..p
                    },
                )
            })
            .collect(),
        badges: BTreeMap::new(),
    }
}

#[test]
fn the_same_shop_is_clean() {
    let report = compare_read(
        &locked(vec![pass(1, 499)], vec![product(2)]),
        &listed(vec![pass(1, 499)], vec![product(2)]),
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

/// The case `sync` used to overwrite: a price edited on the Creator Dashboard.
#[test]
fn a_price_edited_on_roblox_is_a_difference() {
    let report = compare_read(
        &locked(vec![pass(1, 499)], vec![]),
        &listed(vec![pass(1, 599)], vec![]),
    );
    assert_eq!(
        report.differences,
        vec!["pass 'VIP' price: lockfile 499, Roblox 599"]
    );
}

#[test]
fn a_pass_taken_off_sale_on_roblox_is_a_difference() {
    let off = PassLock {
        for_sale: false,
        ..pass(1, 499)
    };
    let report = compare_read(
        &locked(vec![pass(1, 499)], vec![]),
        &listed(vec![off], vec![]),
    );
    assert_eq!(
        report.differences,
        vec!["pass 'VIP' for_sale: lockfile true, Roblox false"]
    );
}

#[test]
fn a_resource_gone_from_roblox_is_a_difference() {
    let report = compare_read(&locked(vec![pass(1, 499)], vec![]), &listed(vec![], vec![]));
    assert_eq!(
        report.differences,
        vec!["pass 'VIP' (1): in the lockfile, not on Roblox"]
    );
}

#[test]
fn a_resource_made_by_hand_on_roblox_is_a_difference() {
    let report = compare_read(&locked(vec![], vec![]), &listed(vec![], vec![product(9)]));
    assert_eq!(
        report.differences,
        vec![r#"product "Coins" (9): on Roblox, not in the lockfile"#]
    );
}

/// Roblox does not tell an absent description from an empty one.
#[test]
fn an_empty_description_matches_an_absent_one() {
    let absent = ProductLock {
        description: None,
        ..product(2)
    };
    let report = compare_read(
        &locked(vec![], vec![product(2)]),
        &listed(vec![], vec![absent]),
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

/// A lockfile written before `managed_pricing` was recorded holds nothing to
/// compare: the false alarm a `pull` then `git diff` raises after an upgrade.
#[test]
fn a_lockfile_without_managed_pricing_is_not_a_difference() {
    let old = PassLock {
        managed_pricing: None,
        ..pass(1, 499)
    };
    let report = compare_read(
        &locked(vec![old], vec![]),
        &listed(vec![pass(1, 499)], vec![]),
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn a_disabled_badge_is_compared_like_any_other() {
    let badge = BadgeLock {
        id: 3,
        name: "Welcome".into(),
        description: None,
        enabled: true,
        icon_asset_id: Some(5),
        icon_hash: None,
    };
    let env_lock = EnvLock {
        universe_id: 1,
        badges: [("Welcome".to_string(), badge.clone())].into(),
        ..Default::default()
    };
    let remote = RemoteLocks {
        badges: [(
            3,
            BadgeLock {
                enabled: false,
                ..badge
            },
        )]
        .into(),
        ..Default::default()
    };

    let report = compare_read(&env_lock, &remote);
    assert_eq!(
        report.differences,
        vec!["badge 'Welcome' enabled: lockfile true, Roblox false"]
    );
}
