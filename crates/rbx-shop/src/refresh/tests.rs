#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use super::*;
use crate::live::changed_fields;

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
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
        managed_pricing: Some(false),
        store_page: false,
    }
}

fn recorded(passes: Vec<PassLock>, products: Vec<ProductLock>) -> EnvLock {
    EnvLock {
        universe_id: 1,
        passes: passes.into_iter().map(|p| (p.name.clone(), p)).collect(),
        products: products.into_iter().map(|p| (p.name.clone(), p)).collect(),
        ..Default::default()
    }
}

/// What `remote::*_lock` produces: keyed by id, and no icon hash, ever.
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
        products: products.into_iter().map(|p| (p.id, p)).collect(),
        badges: BTreeMap::new(),
    }
}

fn lines(recorded: &EnvLock, seen: &EnvLock) -> Vec<String> {
    changed_fields(recorded, seen)
        .iter()
        .map(|d| d.to_string())
        .collect()
}

/// The case this module exists for: a price edited on the Creator Dashboard,
/// with the two local files in agreement.
#[test]
fn a_price_edited_on_roblox_is_what_the_plan_is_built_against() {
    let before = recorded(vec![pass(1, 499)], vec![]);

    let refreshed = refresh(
        &before,
        &listed(vec![pass(1, 599)], vec![]),
        &ResourceKind::ALL,
    );

    assert_eq!(refreshed.lock.passes["VIP"].price, Some(599));
    assert_eq!(
        lines(&before, &refreshed.lock),
        vec!["pass 'VIP' price: lockfile 499, Roblox 599"]
    );
    assert!(refreshed.gone.is_empty(), "{:?}", refreshed.gone);
}

/// A resource is the one its id says it is. Renamed on Roblox, it is still
/// tracked under the config's key, with the new name as a changed field.
#[test]
fn a_resource_renamed_on_roblox_stays_under_its_key() {
    let before = recorded(vec![pass(1, 499)], vec![]);
    let renamed = PassLock {
        name: "VIP Gold".into(),
        ..pass(1, 499)
    };

    let refreshed = refresh(&before, &listed(vec![renamed], vec![]), &ResourceKind::ALL);

    assert_eq!(refreshed.lock.passes["VIP"].name, "VIP Gold");
    assert_eq!(
        lines(&before, &refreshed.lock),
        vec![r#"pass 'VIP' name: lockfile "VIP", Roblox "VIP Gold""#]
    );
}

/// The icon hash describes a local file and the id it was tied to, which a
/// listing cannot speak for: both stay as recorded, whatever Roblox lists.
#[test]
fn the_icon_is_left_as_recorded() {
    let before = recorded(vec![pass(1, 499)], vec![]);
    let another_icon = PassLock {
        icon_asset_id: Some(99),
        ..pass(1, 499)
    };

    let refreshed = refresh(
        &before,
        &listed(vec![another_icon], vec![]),
        &ResourceKind::ALL,
    );

    let entry = &refreshed.lock.passes["VIP"];
    assert_eq!(entry.icon_asset_id, Some(7));
    assert_eq!(entry.icon_hash.as_deref(), Some("local-file-hash"));
    assert!(lines(&before, &refreshed.lock).is_empty());
}

/// Made by hand on Roblox and tracked by no entry: not `sync`'s to touch, so
/// it does not enter the section the plan is built against.
#[test]
fn a_resource_made_by_hand_is_not_brought_in() {
    let before = recorded(vec![pass(1, 499)], vec![]);

    let refreshed = refresh(
        &before,
        &listed(vec![pass(1, 499)], vec![product(9)]),
        &ResourceKind::ALL,
    );

    assert!(refreshed.lock.products.is_empty());
    assert_eq!(refreshed.lock, before);
}

/// An id Roblox stopped listing keeps its entry. Dropped, the next plan would
/// create a second resource, and a pass or a product cannot be deleted.
#[test]
fn a_resource_roblox_does_not_list_keeps_its_entry_and_is_reported() {
    let before = recorded(vec![pass(1, 499)], vec![product(2)]);

    let refreshed = refresh(&before, &listed(vec![], vec![]), &ResourceKind::ALL);

    assert_eq!(refreshed.lock, before);
    assert_eq!(
        refreshed.gone,
        vec![
            "pass 'VIP' (1): in the lockfile, not on Roblox",
            "product 'Coins' (2): in the lockfile, not on Roblox",
        ]
    );
}

/// `--only passes` lists passes alone, so the products section is what the
/// lockfile recorded and none of it reads as gone.
#[test]
fn a_kind_that_was_not_listed_stays_as_recorded() {
    let before = recorded(vec![pass(1, 499)], vec![product(2)]);

    let refreshed = refresh(
        &before,
        &listed(vec![pass(1, 599)], vec![]),
        &[ResourceKind::Pass],
    );

    assert_eq!(refreshed.lock.passes["VIP"].price, Some(599));
    assert_eq!(refreshed.lock.products, before.products);
    assert!(refreshed.gone.is_empty(), "{:?}", refreshed.gone);
}

/// Roblox as recorded changes nothing: the run that finds this writes no
/// lockfile.
#[test]
fn roblox_as_recorded_leaves_the_section_identical() {
    let before = recorded(vec![pass(1, 499)], vec![product(2)]);

    let refreshed = refresh(
        &before,
        &listed(vec![pass(1, 499)], vec![product(2)]),
        &ResourceKind::ALL,
    );

    assert_eq!(refreshed.lock, before);
    assert!(lines(&before, &refreshed.lock).is_empty());
}
