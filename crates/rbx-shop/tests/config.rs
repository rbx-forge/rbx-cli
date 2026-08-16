#![allow(clippy::unwrap_used)]
use std::collections::BTreeMap;

use rbx_shop::config::{
    BadgeConfig, BadgeOverlay, Config, EnvOverlay, PassConfig, PassOverlay, ProductOverlay,
};
use tempfile::tempdir;

fn write_config(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxshop.toml");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

#[test]
fn loads_minimal_config_without_experience() {
    let (_dir, path) = write_config(
        r#"
[owner]
type = "user"
id = 123

[passes.VIP]
price = 499
"#,
    );

    let config = Config::load(&path).unwrap();
    assert!(config.experience.is_none());
    assert!(config.owner.is_some());
    assert!(config.passes.contains_key("VIP"));
    assert!(config.envs.is_empty());
}

#[test]
fn loads_standalone_with_experience() {
    let (_dir, path) = write_config(
        r#"
[experience]
universe_id = 42

[owner]
type = "group"
id = 7

[passes.VIP]
price = 999
"#,
    );

    let config = Config::load(&path).unwrap();
    let exp = config.experience.as_ref().unwrap();
    assert_eq!(exp.universe_id, 42);
    let owner = config.owner.as_ref().unwrap();
    assert_eq!(owner.id, 7);
}

#[test]
fn resolve_env_returns_base_when_no_overlay() {
    let config = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::from([(
            "VIP".to_string(),
            PassConfig {
                name: None,
                price: Some(499),
                description: None,
                icon: None,
                for_sale: true,
                regional_pricing: false,
                create_gift: false,
                path: None,
            },
        )]),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs: BTreeMap::new(),
    };
    let resolved = config.resolve_env(Some("dev")).unwrap();
    assert_eq!(resolved.passes.get("VIP").unwrap().price, Some(499));
}

#[test]
fn resolve_env_applies_overlay_on_existing_pass() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "prod".to_string(),
        EnvOverlay {
            passes: BTreeMap::from([(
                "VIP".to_string(),
                PassOverlay {
                    price: Some(999),
                    ..Default::default()
                },
            )]),
            badges: BTreeMap::new(),
            products: BTreeMap::new(),
        },
    );

    let config = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::from([(
            "VIP".to_string(),
            PassConfig {
                name: None,
                price: Some(499),
                description: None,
                icon: None,
                for_sale: true,
                regional_pricing: false,
                create_gift: false,
                path: None,
            },
        )]),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs,
    };

    let dev = config.resolve_env(Some("dev")).unwrap();
    assert_eq!(dev.passes.get("VIP").unwrap().price, Some(499));

    let prod = config.resolve_env(Some("prod")).unwrap();
    assert_eq!(prod.passes.get("VIP").unwrap().price, Some(999));
}

#[test]
fn resolve_env_adds_env_exclusive_pass() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".to_string(),
        EnvOverlay {
            passes: BTreeMap::from([(
                "BetaPass".to_string(),
                PassOverlay {
                    price: Some(0),
                    description: Some("Beta only".to_string()),
                    ..Default::default()
                },
            )]),
            badges: BTreeMap::new(),
            products: BTreeMap::new(),
        },
    );
    let config = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::new(),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs,
    };

    let prod = config.resolve_env(Some("prod")).unwrap();
    assert!(prod.passes.is_empty());

    let dev = config.resolve_env(Some("dev")).unwrap();
    assert_eq!(dev.passes.get("BetaPass").unwrap().price, Some(0));
}

#[test]
fn resolve_env_errors_on_overlay_only_product_without_price() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".to_string(),
        EnvOverlay {
            passes: BTreeMap::new(),
            badges: BTreeMap::new(),
            products: BTreeMap::from([(
                "Skin".to_string(),
                ProductOverlay {
                    description: Some("forgot price".into()),
                    ..Default::default()
                },
            )]),
        },
    );
    let config = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::new(),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs,
    };
    let err = config.resolve_env(Some("dev")).unwrap_err();
    assert!(err.to_string().contains("price"));
}

#[test]
fn badge_overlay_enabled_override() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".to_string(),
        EnvOverlay {
            passes: BTreeMap::new(),
            badges: BTreeMap::from([(
                "Welcome".to_string(),
                BadgeOverlay {
                    enabled: Some(false),
                    ..Default::default()
                },
            )]),
            products: BTreeMap::new(),
        },
    );
    let config = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::new(),
        badges: BTreeMap::from([(
            "Welcome".to_string(),
            BadgeConfig {
                name: None,
                description: Some("Hi".into()),
                icon: None,
                enabled: true,
                path: None,
            },
        )]),
        products: BTreeMap::new(),
        envs,
    };

    let prod = config.resolve_env(Some("prod")).unwrap();
    assert!(prod.badges.get("Welcome").unwrap().enabled);

    let dev = config.resolve_env(Some("dev")).unwrap();
    assert!(!dev.badges.get("Welcome").unwrap().enabled);
}

#[test]
fn default_template_parses() {
    let (_dir, path) = write_config(&Config::default_template());
    let _ = Config::load(&path).unwrap();
}

#[test]
fn create_gift_derives_a_product_at_resolve_time() {
    let (_dir, path) = write_config(
        r#"
[passes.VIP]
name = "VIP Pass"
price = 499
description = "VIP access"
create_gift = true
"#,
    );
    let config = Config::load(&path).unwrap();
    let resolved = config.resolve_env(None).unwrap();

    assert!(resolved.passes["VIP"].create_gift);
    let gift = resolved
        .products
        .get("GiftVIP")
        .expect("gift product derived");
    assert_eq!(gift.name.as_deref(), Some("[GIFT] VIP Pass"));
    assert_eq!(gift.price, 499);
    assert_eq!(gift.description.as_deref(), Some("VIP access"));
}

#[test]
fn custom_gifts_label_is_used_as_prefix() {
    let (_dir, path) = write_config(
        r#"
[gifts]
label = "GIFT - "

[products.Coins100]
price = 99
create_gift = true
"#,
    );
    let config = Config::load(&path).unwrap();
    let resolved = config.resolve_env(None).unwrap();
    let gift = resolved.products.get("GiftCoins100").unwrap();
    assert_eq!(gift.name.as_deref(), Some("GIFT - Coins100"));
}

#[test]
fn create_gift_can_be_env_exclusive_via_overlay() {
    let (_dir, path) = write_config(
        r#"
[passes.VIP]
price = 499

[envs.dev.passes.VIP]
create_gift = true
"#,
    );
    let config = Config::load(&path).unwrap();

    let prod = config.resolve_env(Some("prod")).unwrap();
    assert!(!prod.products.contains_key("GiftVIP"));

    let dev = config.resolve_env(Some("dev")).unwrap();
    assert!(dev.products.contains_key("GiftVIP"));
}

#[test]
fn create_gift_can_be_disabled_per_env_override() {
    let (_dir, path) = write_config(
        r#"
[passes.VIP]
price = 499
create_gift = true

[envs.dev.passes.VIP]
create_gift = false
"#,
    );
    let config = Config::load(&path).unwrap();

    let prod = config.resolve_env(Some("prod")).unwrap();
    assert!(prod.products.contains_key("GiftVIP"));

    let dev = config.resolve_env(Some("dev")).unwrap();
    assert!(!dev.products.contains_key("GiftVIP"));
}

#[test]
fn create_gift_collision_with_real_product_errors() {
    let (_dir, path) = write_config(
        r#"
[passes.VIP]
price = 499
create_gift = true

[products.GiftVIP]
price = 1
"#,
    );
    let config = Config::load(&path).unwrap();
    let err = config.resolve_env(None).unwrap_err();
    assert!(err.to_string().contains("GiftVIP"));
}

#[test]
fn create_gift_on_a_badge_is_rejected_rather_than_silently_ignored() {
    let (_dir, path) = write_config(
        r#"
[badges.Welcome]
description = "hi"
create_gift = true
"#,
    );
    let err = Config::load(&path).unwrap_err();
    assert!(format!("{err:?}").contains("unknown field"));
}

#[test]
fn load_merged_pulls_in_resources_from_included_files() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("rbxshop.toml");
    std::fs::write(
        &main_path,
        r#"
[include]
files = ["rbxshop.badges.toml"]

[passes.VIP]
price = 499
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("rbxshop.badges.toml"),
        r#"
[badges.Welcome]
description = "hi"
"#,
    )
    .unwrap();

    let config = Config::load_merged(&main_path).unwrap();
    assert!(config.passes.contains_key("VIP"));
    assert!(config.badges.contains_key("Welcome"));
}

#[test]
fn load_merged_is_a_no_op_without_include() {
    let (_dir, path) = write_config(
        r#"
[passes.VIP]
price = 499
"#,
    );
    let plain = Config::load(&path).unwrap();
    let merged = Config::load_merged(&path).unwrap();
    assert_eq!(plain.passes.len(), merged.passes.len());
}

#[test]
fn load_merged_errors_on_duplicate_key_across_files() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("rbxshop.toml");
    std::fs::write(
        &main_path,
        r#"
[include]
files = ["rbxshop.extra.toml"]

[passes.VIP]
price = 499
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("rbxshop.extra.toml"),
        r#"
[passes.VIP]
price = 999
"#,
    )
    .unwrap();

    let err = Config::load_merged(&main_path).unwrap_err();
    assert!(err.to_string().contains("VIP"));
}

#[test]
fn load_merged_rejects_non_resource_sections_in_included_file() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("rbxshop.toml");
    std::fs::write(
        &main_path,
        r#"
[include]
files = ["rbxshop.extra.toml"]
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("rbxshop.extra.toml"),
        r#"
[owner]
type = "user"
id = 1
"#,
    )
    .unwrap();

    let err = Config::load_merged(&main_path).unwrap_err();
    assert!(err.to_string().contains("may only contain"));
}

#[test]
fn gift_key_prefix_is_configurable_and_the_source_key_is_never_transformed() {
    let (_dir, path) = write_config(
        r#"
[gifts]
key_prefix = "gift"

[passes.vip_pass]
price = 499
create_gift = true
"#,
    );
    let config = Config::load(&path).unwrap();
    let resolved = config.resolve_env(None).unwrap();
    // Lowercase prefix, source key untouched (no case conversion).
    assert!(resolved.products.contains_key("giftvip_pass"));
}

#[test]
fn empty_gift_key_prefix_is_rejected() {
    let (_dir, path) = write_config(
        r#"
[gifts]
key_prefix = ""

[passes.VIP]
price = 499
create_gift = true
"#,
    );
    let config = Config::load(&path).unwrap();
    let err = config.resolve_env(None).unwrap_err();
    assert!(err.to_string().contains("key_prefix"));
}

#[test]
fn load_merged_pulls_in_env_overlays_from_included_files() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("rbxshop.toml");
    std::fs::write(
        &main_path,
        r#"
[include]
files = ["rbxshop.passes.toml"]
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("rbxshop.passes.toml"),
        r#"
[passes.VIP]
price = 499

[envs.prod.passes.VIP]
price = 999
"#,
    )
    .unwrap();

    let config = Config::load_merged(&main_path).unwrap();
    let prod = config.resolve_env(Some("prod")).unwrap();
    assert_eq!(prod.passes["VIP"].price, Some(999));
    let dev = config.resolve_env(Some("dev")).unwrap();
    assert_eq!(dev.passes["VIP"].price, Some(499));
}

#[test]
fn load_merged_errors_on_duplicate_overlay_across_files() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("rbxshop.toml");
    std::fs::write(
        &main_path,
        r#"
[include]
files = ["rbxshop.extra.toml"]

[envs.prod.passes.VIP]
price = 999
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("rbxshop.extra.toml"),
        r#"
[envs.prod.passes.VIP]
price = 111
"#,
    )
    .unwrap();

    let err = Config::load_merged(&main_path).unwrap_err();
    assert!(err.to_string().contains("prod"));
    assert!(err.to_string().contains("VIP"));
}

#[test]
fn gift_capitalize_key_produces_a_readable_compound_identifier() {
    let (_dir, path) = write_config(
        r#"
[gifts]
key_prefix = "gift"
capitalize_key = true

[passes.vipPass]
price = 499
create_gift = true
"#,
    );
    let config = Config::load(&path).unwrap();
    let resolved = config.resolve_env(None).unwrap();
    assert!(resolved.products.contains_key("giftVipPass"));
    assert!(!resolved.products.contains_key("giftvipPass"));
}
