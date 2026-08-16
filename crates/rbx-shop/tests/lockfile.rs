#![allow(clippy::unwrap_used)]
use std::collections::BTreeMap;

use rbx_shop::lockfile::{BadgeLock, EnvLock, Lockfile, PassLock, ProductLock, LOCKFILE_VERSION};
use tempfile::tempdir;

#[test]
fn load_returns_default_when_missing() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxshop.lock.toml");
    let lockfile = Lockfile::load(&path).unwrap();
    assert_eq!(lockfile.version, LOCKFILE_VERSION);
    assert!(lockfile.envs.is_empty());
}

#[test]
fn v2_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxshop.lock.toml");

    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".to_string(),
        EnvLock {
            universe_id: 100,
            passes: BTreeMap::from([(
                "VIP".to_string(),
                PassLock {
                    id: 1,
                    name: "VIP".to_string(),
                    price: Some(499),
                    description: None,
                    icon_asset_id: None,
                    icon_hash: None,
                    for_sale: true,
                    regional_pricing: false,
                },
            )]),
            badges: BTreeMap::new(),
            products: BTreeMap::new(),
        },
    );

    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs,
    };
    lockfile.save(&path).unwrap();

    let reloaded = Lockfile::load(&path).unwrap();
    assert_eq!(reloaded, lockfile);
}

#[test]
fn env_mut_inserts_when_missing() {
    let mut lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::new(),
    };
    let env = lockfile.env_mut("dev", 12345);
    assert_eq!(env.universe_id, 12345);
    assert!(env.passes.is_empty());

    // Inserting again updates universe_id.
    let env = lockfile.env_mut("dev", 99999);
    assert_eq!(env.universe_id, 99999);
}

#[test]
fn product_lock_round_trip() {
    let lock = ProductLock {
        id: 1,
        name: "Coins".to_string(),
        price: 99,
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
        store_page: false,
    };
    let serialized = toml::to_string(&lock).unwrap();
    let parsed: ProductLock = toml::from_str(&serialized).unwrap();
    assert_eq!(parsed, lock);
}

#[test]
fn badge_lock_round_trip() {
    let lock = BadgeLock {
        id: 1,
        name: "Welcome".to_string(),
        description: Some("Welcome!".to_string()),
        enabled: true,
        icon_asset_id: None,
        icon_hash: None,
    };
    let serialized = toml::to_string(&lock).unwrap();
    let parsed: BadgeLock = toml::from_str(&serialized).unwrap();
    assert_eq!(parsed, lock);
}
