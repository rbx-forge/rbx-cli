//! `build_sync_plan`: the function that decides what gets created, updated,
//! or left alone on a live Roblox account.
//!
//! It is pure (config + lockfile + a directory of icons in, plan out), so
//! every test here asserts the *exact* plan rather than its cardinality: a
//! count-only assertion passes just as happily when a field change lands on
//! the wrong resource, or when a "price" diff quietly becomes a "name" diff.
//!
//! Plans are rendered to strings via `render` below so that the assertion
//! shows the whole plan when it fails, including the per-field changes and
//! the order they were pushed in.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;

use rbx_shop::config::{BadgeConfig, PassConfig, ProductConfig, ResolvedResources};
use rbx_shop::diff::{build_sync_plan, Action, ResourceAction, SyncPlan};
use rbx_shop::lockfile::{BadgeLock, EnvLock, PassLock, ProductLock};

// ── rendering ──

/// One line per action: `create VIP`, `skip VIP`, or
/// `update VIP: price: Some(100) -> Some(200); name: a -> b`.
fn render(actions: &[ResourceAction]) -> Vec<String> {
    actions
        .iter()
        .map(|a| match &a.action {
            Action::Create => format!("create {}", a.name),
            Action::Skip => format!("skip {}", a.name),
            Action::Update { changes } => format!(
                "update {}: {}",
                a.name,
                changes
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        })
        .collect()
}

/// The whole plan as `(passes, badges, products, warnings)`, ready to compare
/// against literals.
fn rendered(plan: &SyncPlan) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    (
        render(&plan.passes),
        render(&plan.badges),
        render(&plan.products),
        plan.warnings.clone(),
    )
}

// ── fixtures ──

fn pass(price: Option<u64>) -> PassConfig {
    PassConfig {
        name: None,
        price,
        description: None,
        icon: None,
        for_sale: true,
        regional_pricing: false,
        create_gift: false,
        path: None,
    }
}

fn badge() -> BadgeConfig {
    BadgeConfig {
        name: None,
        description: None,
        icon: None,
        enabled: true,
        path: None,
    }
}

fn product(price: u64) -> ProductConfig {
    ProductConfig {
        name: None,
        price,
        description: None,
        icon: None,
        for_sale: true,
        regional_pricing: false,
        store_page: false,
        create_gift: false,
        path: None,
    }
}

fn pass_lock(price: Option<u64>) -> PassLock {
    PassLock {
        id: 1,
        name: "VIP".into(),
        price,
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
    }
}

fn badge_lock() -> BadgeLock {
    BadgeLock {
        id: 2,
        name: "Welcome".into(),
        description: None,
        enabled: true,
        icon_asset_id: None,
        icon_hash: None,
    }
}

fn product_lock(price: u64) -> ProductLock {
    ProductLock {
        id: 3,
        name: "Coins".into(),
        price,
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
        store_page: false,
    }
}

fn resources(
    passes: Vec<(&str, PassConfig)>,
    badges: Vec<(&str, BadgeConfig)>,
    products: Vec<(&str, ProductConfig)>,
) -> ResolvedResources {
    ResolvedResources {
        passes: passes
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        badges: badges
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        products: products
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn lock(
    passes: Vec<(&str, PassLock)>,
    badges: Vec<(&str, BadgeLock)>,
    products: Vec<(&str, ProductLock)>,
) -> EnvLock {
    EnvLock {
        universe_id: 555,
        passes: passes
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        badges: badges
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        products: products
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn empty_lock() -> EnvLock {
    EnvLock {
        universe_id: 555,
        ..Default::default()
    }
}

/// Write `contents` to `<dir>/<name>` and return the blake3 hash the diff will
/// compute for it. The diff hashes the file bytes as they sit on disk (not the
/// re-encoded PNG the upload sends), so any bytes will do.
fn icon(dir: &Path, name: &str, contents: &[u8]) -> String {
    std::fs::write(dir.join(name), contents).unwrap();
    rbx_core::image::hash_bytes(contents)
}

// ── creates ──

#[test]
fn a_resource_absent_from_the_lockfile_is_created() {
    let plan = build_sync_plan(
        &resources(
            vec![("VIP", pass(Some(499)))],
            vec![("Welcome", badge())],
            vec![("Coins", product(99))],
        ),
        &empty_lock(),
        Path::new("."),
    )
    .unwrap();

    let (passes, badges, products, warnings) = rendered(&plan);
    assert_eq!(passes, ["create VIP"]);
    assert_eq!(badges, ["create Welcome"]);
    assert_eq!(products, ["create Coins"]);
    assert!(warnings.is_empty());
    assert!(plan.has_changes());
    assert_eq!(plan.summary(), "3 to create, 0 to update, 0 unchanged");
}

// ── skips ──

#[test]
fn a_resource_matching_its_lock_entry_is_skipped() {
    let plan = build_sync_plan(
        &resources(
            vec![("VIP", pass(Some(499)))],
            vec![("Welcome", badge())],
            vec![("Coins", product(99))],
        ),
        &lock(
            vec![("VIP", pass_lock(Some(499)))],
            vec![("Welcome", badge_lock())],
            vec![("Coins", product_lock(99))],
        ),
        Path::new("."),
    )
    .unwrap();

    let (passes, badges, products, warnings) = rendered(&plan);
    assert_eq!(passes, ["skip VIP"]);
    assert_eq!(badges, ["skip Welcome"]);
    assert_eq!(products, ["skip Coins"]);
    assert!(warnings.is_empty());
    assert!(!plan.has_changes());
    assert_eq!(plan.summary(), "0 to create, 0 to update, 3 unchanged");
}

/// The lock entry's `name` is the *display* name, so a config with no explicit
/// `name` matches when the TOML key equals it. This is the case that makes
/// `resolve_name` load-bearing: read the key instead and every resource
/// without an explicit name diffs forever.
#[test]
fn an_implicit_display_name_matching_the_lock_is_not_a_change() {
    let mut cfg = pass(Some(499));
    cfg.name = None;
    let plan = build_sync_plan(
        &resources(vec![("VIP", cfg)], vec![], vec![]),
        &lock(vec![("VIP", pass_lock(Some(499)))], vec![], vec![]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(render(&plan.passes), ["skip VIP"]);
}

// ── field-by-field updates ──

#[test]
fn every_pass_field_that_diverges_is_named_in_the_update() {
    let mut cfg = pass(Some(999));
    cfg.name = Some("VIP Deluxe".into());
    cfg.description = Some("all the perks".into());
    cfg.for_sale = false;
    cfg.regional_pricing = true;

    let plan = build_sync_plan(
        &resources(vec![("VIP", cfg)], vec![], vec![]),
        &lock(vec![("VIP", pass_lock(Some(499)))], vec![], vec![]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(
        render(&plan.passes),
        ["update VIP: name: VIP -> VIP Deluxe; \
          price: Some(499) -> Some(999); \
          description:  -> all the perks; \
          for_sale: true -> false; \
          regional_pricing: false -> true"]
    );
    assert_eq!(plan.summary(), "0 to create, 1 to update, 0 unchanged");
}

#[test]
fn every_badge_field_that_diverges_is_named_in_the_update() {
    let mut cfg = badge();
    cfg.name = Some("Welcome Aboard".into());
    cfg.description = Some("first login".into());
    cfg.enabled = false;

    let plan = build_sync_plan(
        &resources(vec![], vec![("Welcome", cfg)], vec![]),
        &lock(vec![], vec![("Welcome", badge_lock())], vec![]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(
        render(&plan.badges),
        ["update Welcome: name: Welcome -> Welcome Aboard; \
          description:  -> first login; \
          enabled: true -> false"]
    );
}

#[test]
fn every_product_field_that_diverges_is_named_in_the_update() {
    let mut cfg = product(199);
    cfg.name = Some("100 Coins".into());
    cfg.description = Some("a pile".into());
    cfg.for_sale = false;
    cfg.regional_pricing = true;
    cfg.store_page = true;

    let plan = build_sync_plan(
        &resources(vec![], vec![], vec![("Coins", cfg)]),
        &lock(vec![], vec![], vec![("Coins", product_lock(99))]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(
        render(&plan.products),
        ["update Coins: name: Coins -> 100 Coins; \
          price: 99 -> 199; \
          description:  -> a pile; \
          for_sale: true -> false; \
          regional_pricing: false -> true; \
          store_page: false -> true"]
    );
}

/// A single field changing must produce a single-field update, not a blanket
/// "everything changed": the printed diff is what a user reads before
/// approving a sync that spends money.
#[test]
fn one_diverging_field_yields_a_one_field_update() {
    let plan = build_sync_plan(
        &resources(vec![("VIP", pass(Some(999)))], vec![], vec![]),
        &lock(vec![("VIP", pass_lock(Some(499)))], vec![], vec![]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(
        render(&plan.passes),
        ["update VIP: price: Some(499) -> Some(999)"]
    );
}

/// `None` price and `Some(0)` are different states: a pass with no price is
/// not a free pass, and collapsing them would silently put a paid pass on sale
/// for nothing.
#[test]
fn dropping_a_price_to_none_is_a_change() {
    let plan = build_sync_plan(
        &resources(vec![("VIP", pass(None))], vec![], vec![]),
        &lock(vec![("VIP", pass_lock(Some(499)))], vec![], vec![]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(
        render(&plan.passes),
        ["update VIP: price: Some(499) -> None"]
    );
}

/// An absent description and an empty one are the same thing to Roblox; the
/// diff must not churn on `None` vs `Some("")`.
#[test]
fn an_absent_description_equals_an_empty_one() {
    let mut cfg = pass(Some(499));
    cfg.description = Some(String::new());
    let mut locked = pass_lock(Some(499));
    locked.description = None;

    let plan = build_sync_plan(
        &resources(vec![("VIP", cfg)], vec![], vec![]),
        &lock(vec![("VIP", locked)], vec![], vec![]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(render(&plan.passes), ["skip VIP"]);
}

// ── icon hash comparison ──
//
// Both directions matter and both are silent when wrong: a false negative
// re-uploads an unchanged icon on every sync, a false positive leaves a stale
// icon live forever.

#[test]
fn an_icon_whose_hash_matches_the_lock_is_not_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let hash = icon(dir.path(), "vip.png", b"pixels");

    let mut cfg = pass(Some(499));
    cfg.icon = Some("vip.png".into());
    let mut locked = pass_lock(Some(499));
    locked.icon_hash = Some(hash);

    let plan = build_sync_plan(
        &resources(vec![("VIP", cfg)], vec![], vec![]),
        &lock(vec![("VIP", locked)], vec![], vec![]),
        dir.path(),
    )
    .unwrap();

    assert_eq!(render(&plan.passes), ["skip VIP"]);
}

#[test]
fn an_icon_whose_bytes_changed_is_an_icon_update() {
    let dir = tempfile::tempdir().unwrap();
    let old_hash = rbx_core::image::hash_bytes(b"old pixels");
    icon(dir.path(), "vip.png", b"new pixels");

    let mut cfg = pass(Some(499));
    cfg.icon = Some("vip.png".into());
    let mut locked = pass_lock(Some(499));
    locked.icon_hash = Some(old_hash.clone());

    let plan = build_sync_plan(
        &resources(vec![("VIP", cfg)], vec![], vec![]),
        &lock(vec![("VIP", locked)], vec![], vec![]),
        dir.path(),
    )
    .unwrap();

    // Truncated to eight chars, which is all the user is shown.
    let expected_old: String = old_hash.chars().take(8).collect();
    let expected_new: String = rbx_core::image::hash_bytes(b"new pixels")
        .chars()
        .take(8)
        .collect();
    assert_eq!(
        render(&plan.passes),
        [format!(
            "update VIP: icon: {}... -> {}...",
            expected_old, expected_new
        )]
    );
}

/// No hash in the lock means the icon was never uploaded, so it must be sent:
/// the empty-string fallback has to compare unequal to any real hash.
#[test]
fn an_icon_with_no_hash_in_the_lock_is_an_icon_update() {
    let dir = tempfile::tempdir().unwrap();
    let hash = icon(dir.path(), "vip.png", b"pixels");

    let mut cfg = pass(Some(499));
    cfg.icon = Some("vip.png".into());
    let locked = pass_lock(Some(499)); // icon_hash: None

    let plan = build_sync_plan(
        &resources(vec![("VIP", cfg)], vec![], vec![]),
        &lock(vec![("VIP", locked)], vec![], vec![]),
        dir.path(),
    )
    .unwrap();

    let short: String = hash.chars().take(8).collect();
    assert_eq!(
        render(&plan.passes),
        [format!("update VIP: icon: ... -> {}...", short)]
    );
}

/// Dropping the `icon` key does not schedule an icon change: the diff only
/// looks at icons the config still declares. Asserted so that the day it
/// becomes a delete, it is a deliberate decision and not a surprise.
#[test]
fn removing_the_icon_key_leaves_the_remote_icon_alone() {
    let mut locked = pass_lock(Some(499));
    locked.icon_hash = Some("deadbeef".into());

    let plan = build_sync_plan(
        &resources(vec![("VIP", pass(Some(499)))], vec![], vec![]),
        &lock(vec![("VIP", locked)], vec![], vec![]),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(render(&plan.passes), ["skip VIP"]);
}

#[test]
fn badges_and_products_compare_icon_hashes_too() {
    let dir = tempfile::tempdir().unwrap();
    let hash = icon(dir.path(), "shared.png", b"pixels");
    let short: String = hash.chars().take(8).collect();

    let mut badge_cfg = badge();
    badge_cfg.icon = Some("shared.png".into());
    let mut product_cfg = product(99);
    product_cfg.icon = Some("shared.png".into());

    // Badge already carries the matching hash, product carries none.
    let mut badge_locked = badge_lock();
    badge_locked.icon_hash = Some(hash);

    let plan = build_sync_plan(
        &resources(
            vec![],
            vec![("Welcome", badge_cfg)],
            vec![("Coins", product_cfg)],
        ),
        &lock(
            vec![],
            vec![("Welcome", badge_locked)],
            vec![("Coins", product_lock(99))],
        ),
        dir.path(),
    )
    .unwrap();

    assert_eq!(render(&plan.badges), ["skip Welcome"]);
    assert_eq!(
        render(&plan.products),
        [format!("update Coins: icon: ... -> {}...", short)]
    );
}

/// An icon path that does not exist is an error, not a silent skip. `sync`
/// calls `validate_icon_paths` first, but `check` reaches the diff directly.
#[test]
fn a_missing_icon_file_fails_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = pass(Some(499));
    cfg.icon = Some("nope.png".into());

    let err = build_sync_plan(
        &resources(vec![("VIP", cfg)], vec![], vec![]),
        &lock(vec![("VIP", pass_lock(Some(499)))], vec![], vec![]),
        dir.path(),
    )
    .unwrap_err();

    // Matched on the io kind rather than the message, which is localized.
    assert_eq!(
        err.downcast_ref::<std::io::Error>().map(|e| e.kind()),
        Some(std::io::ErrorKind::NotFound),
        "expected a not-found io error, got: {err}"
    );
}

// ── warnings ──

/// Removing a resource from the config never deletes it remotely: it just
/// falls out of management. The warning is the only signal the user gets, so
/// it must name the resource and say deletion is not happening.
#[test]
fn a_lock_entry_with_no_config_entry_warns_once_per_resource() {
    let plan = build_sync_plan(
        &ResolvedResources::default(),
        &lock(
            vec![("VIP", pass_lock(Some(499)))],
            vec![("Welcome", badge_lock())],
            vec![("Coins", product_lock(99))],
        ),
        Path::new("."),
    )
    .unwrap();

    let (passes, badges, products, warnings) = rendered(&plan);
    assert_eq!(
        warnings,
        [
            "Pass 'VIP' exists in lockfile but not in resolved config (will not be deleted)",
            "Badge 'Welcome' exists in lockfile but not in resolved config (will not be deleted)",
            "Product 'Coins' exists in lockfile but not in resolved config (will not be deleted)",
        ]
    );
    // A warning is not an action: nothing is planned for the orphans.
    assert!(passes.is_empty());
    assert!(badges.is_empty());
    assert!(products.is_empty());
    assert!(!plan.has_changes());
}

/// A warning must not suppress the work the rest of the plan still has to do.
#[test]
fn warnings_and_actions_coexist() {
    let mut locked = lock(
        vec![("VIP", pass_lock(Some(499)))],
        vec![],
        vec![("Coins", product_lock(99))],
    );
    locked.passes.insert("Retired".into(), pass_lock(Some(1)));

    let plan = build_sync_plan(
        &resources(
            vec![("VIP", pass(Some(999)))],
            vec![("Welcome", badge())],
            vec![("Coins", product(99))],
        ),
        &locked,
        Path::new("."),
    )
    .unwrap();

    let (passes, badges, products, warnings) = rendered(&plan);
    assert_eq!(
        warnings,
        ["Pass 'Retired' exists in lockfile but not in resolved config (will not be deleted)"]
    );
    assert_eq!(passes, ["update VIP: price: Some(499) -> Some(999)"]);
    assert_eq!(badges, ["create Welcome"]);
    assert_eq!(products, ["skip Coins"]);
    assert_eq!(plan.summary(), "1 to create, 1 to update, 1 unchanged");
}

// ── ordering ──

/// Actions follow the config's `BTreeMap` order, so the printed plan is stable
/// across runs. Without this, reviewing a `--dry-run` diff between two
/// branches is noise.
#[test]
fn actions_come_out_in_key_order() {
    let mut passes = BTreeMap::new();
    for key in ["zeta", "alpha", "mid"] {
        passes.insert(key.to_string(), pass(Some(1)));
    }

    let plan = build_sync_plan(
        &ResolvedResources {
            passes,
            ..Default::default()
        },
        &empty_lock(),
        Path::new("."),
    )
    .unwrap();

    assert_eq!(
        render(&plan.passes),
        ["create alpha", "create mid", "create zeta"]
    );
}
