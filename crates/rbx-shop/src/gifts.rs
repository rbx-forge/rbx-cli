//! Derived "gift" developer products.
//!
//! Any pass/product with `create_gift = true` gets an extra developer
//! product derived automatically — same price/description/icon, display name
//! prefixed with `[gifts].label`. The derived resource is never written into
//! `rbxshop.toml`: it only exists inside the in-memory `ResolvedResources`
//! produced by `Config::resolve_env`, so it flows through diff/sync/lockfile/
//! codegen exactly like any other product with zero changes to those
//! modules. This also means editing the source (price, icon, description, or
//! `[gifts].label`) is the only thing you ever need to touch — the twin
//! follows automatically on the next `sync`.
//!
//! Two call sites outside `resolve_env` need to know about this convention:
//! - `commands::rename` must rename the derived lockfile entry alongside its
//!   source (the config has no entry to rename — see `gift_key`).
//! - `commands::pull` must not materialize the remote gift twin as a real
//!   `[products.GiftX]` entry when it lists remote developer products (see
//!   `is_gift_key`).

use std::collections::BTreeMap;

use anyhow::{bail, Result};

use crate::config::{
    resolve_name, Config, EnvOverlay, PassConfig, ProductConfig, ResolvedResources, ResourceKind,
};
use crate::lockfile::ProductLock;

/// Default value of `[gifts].key_prefix` — used by `init --from-remote`
/// (which has no existing config to read a custom prefix from yet).
pub const DEFAULT_GIFT_KEY_PREFIX: &str = "Gift";

/// Derive the resolved-map key for a source resource's gift twin. `prefix` is
/// `[gifts].key_prefix`. The *TOML* key (`[passes.<key>]`) is never
/// transformed anywhere else in this tool — but a raw concatenation like
/// `"gift" + "vipPass"` reads as `giftvipPass`, which looks broken rather
/// than like a compound identifier. When `capitalize` (`[gifts].
/// capitalize_key`) is set, only the copy of the key used in *this derived
/// string* gets its first letter uppercased, giving `giftVipPass` — the
/// pass/product's own key, as written in the config, is untouched either
/// way.
pub fn gift_key(prefix: &str, capitalize: bool, source_key: &str) -> String {
    if capitalize {
        let mut chars = source_key.chars();
        match chars.next() {
            Some(c) => format!("{prefix}{}{}", c.to_ascii_uppercase(), chars.as_str()),
            None => prefix.to_string(),
        }
    } else {
        format!("{prefix}{source_key}")
    }
}

/// Insert a derived gift product for every pass/product with `create_gift =
/// true`. Called once per `resolve_env`, after overlays are merged. Errors if
/// `key_prefix` is empty, or if two sources would derive the same key (e.g. a
/// pass and a product sharing the same base key both have `create_gift =
/// true`, or a real product is literally named `<key_prefix><key>`).
pub fn apply_gifts(
    resources: &mut ResolvedResources,
    label: &str,
    key_prefix: &str,
    capitalize_key: bool,
) -> Result<()> {
    if key_prefix.is_empty() {
        bail!("[gifts].key_prefix must not be empty.");
    }

    let mut derived: Vec<(String, ProductConfig)> = Vec::new();

    for (key, pass) in &resources.passes {
        if pass.create_gift {
            let name = format!("{label}{}", resolve_name(pass.name.as_deref(), key));
            derived.push((
                gift_key(key_prefix, capitalize_key, key),
                ProductConfig {
                    name: Some(name),
                    price: pass.price.unwrap_or(0),
                    description: pass.description.clone(),
                    icon: pass.icon.clone(),
                    for_sale: pass.for_sale,
                    regional_pricing: pass.regional_pricing,
                    store_page: false,
                    create_gift: false,
                    path: None,
                },
            ));
        }
    }

    for (key, product) in &resources.products {
        if product.create_gift {
            let name = format!("{label}{}", resolve_name(product.name.as_deref(), key));
            derived.push((
                gift_key(key_prefix, capitalize_key, key),
                ProductConfig {
                    name: Some(name),
                    price: product.price,
                    description: product.description.clone(),
                    icon: product.icon.clone(),
                    for_sale: product.for_sale,
                    regional_pricing: product.regional_pricing,
                    store_page: false,
                    create_gift: false,
                    path: None,
                },
            ));
        }
    }

    for (key, product) in derived {
        if resources.products.contains_key(&key) {
            bail!(
                "Gift product key '{key}' collides with an existing product (or another \
                 gift-enabled resource sharing the same key) — rename one of them or disable \
                 `create_gift` to resolve the conflict."
            );
        }
        resources.products.insert(key, product);
    }

    Ok(())
}

/// Whether `key` names a gift twin derived from a currently gift-enabled
/// source, checking base config plus an optional env overlay. Used by `pull`
/// to recognize the twin among freshly-listed remote developer products and
/// skip writing it into `rbxshop.toml` as a real entry.
///
/// This scans every pass/product rather than stripping the prefix and
/// looking the remainder up directly: with `capitalize_key` set, deriving
/// `key` from its source uppercased the source's first letter, and that
/// isn't reversible in general (a source that already started uppercase is
/// indistinguishable from one that got capitalized) — recomputing the
/// derived key for each candidate and comparing is unambiguous instead.
pub fn is_gift_key(config: &Config, overlay: Option<&EnvOverlay>, key: &str) -> bool {
    if config.gifts.key_prefix.is_empty() || !key.starts_with(config.gifts.key_prefix.as_str()) {
        return false;
    }
    let prefix = config.gifts.key_prefix.as_str();
    let capitalize = config.gifts.capitalize_key;

    let pass_gift = config.passes.keys().any(|source| {
        gift_key(prefix, capitalize, source) == key
            && overlay
                .and_then(|ov| ov.passes.get(source))
                .and_then(|ov| ov.create_gift)
                .unwrap_or_else(|| config.passes[source].create_gift)
    });
    if pass_gift {
        return true;
    }

    config.products.keys().any(|source| {
        gift_key(prefix, capitalize, source) == key
            && overlay
                .and_then(|ov| ov.products.get(source))
                .and_then(|ov| ov.create_gift)
                .unwrap_or_else(|| config.products[source].create_gift)
    })
}

// ---------------------------------------------------------------------------
// Detection of pre-existing gift twins (used by `init --from-remote --gift-label`)
// ---------------------------------------------------------------------------

/// A pre-existing developer product recognized as another resource's gift
/// twin and folded into the `create_gift` convention.
#[derive(Debug, PartialEq)]
pub struct GiftMerge {
    pub source_key: String,
    pub source_kind: ResourceKind,
    pub twin_key: String,
    pub gift_lock_key: String,
}

/// A developer product whose name matches the expected gift-twin pattern for
/// some source, but whose price diverges — too risky to merge automatically.
#[derive(Debug, PartialEq)]
pub struct GiftPriceMismatch {
    pub source_key: String,
    pub twin_key: String,
    pub source_price: u64,
    pub twin_price: u64,
}

#[derive(Debug, Default, PartialEq)]
pub struct GiftDetectionReport {
    pub merges: Vec<GiftMerge>,
    pub mismatches: Vec<GiftPriceMismatch>,
}

/// Scan freshly-imported passes/products for developer products that look
/// like pre-existing, manually-created gift twins (name == `label` + source's
/// name, same price) and fold them into the `create_gift` convention:
/// `create_gift = true` is set on the source, the twin's literal config entry
/// is removed, and its lockfile entry is rekeyed to
/// `gift_key(key_prefix, capitalize_key, source_key)` so the next `sync`
/// recognizes the existing remote product instead of creating a duplicate.
///
/// A name match with a diverging price is reported but left untouched —
/// matching by name alone risks merging two unrelated products that happen
/// to share the naming convention by coincidence.
pub fn detect_and_merge_gift_twins(
    passes: &mut BTreeMap<String, PassConfig>,
    products: &mut BTreeMap<String, ProductConfig>,
    product_locks: &mut BTreeMap<String, ProductLock>,
    label: &str,
    key_prefix: &str,
    capitalize_key: bool,
) -> GiftDetectionReport {
    let mut report = GiftDetectionReport::default();

    // Snapshot candidate sources before mutating anything: (key, kind, price).
    // `name` is always the key itself here — this only runs right after
    // `init --from-remote` builds these maps, before any `name` override
    // could diverge from the key.
    let sources: Vec<(String, ResourceKind, u64)> = passes
        .iter()
        .map(|(key, p)| (key.clone(), ResourceKind::Pass, p.price.unwrap_or(0)))
        .chain(
            products
                .iter()
                .map(|(key, p)| (key.clone(), ResourceKind::Product, p.price)),
        )
        .collect();

    for (source_key, kind, source_price) in sources {
        let twin_key = format!("{label}{source_key}");
        if twin_key == source_key {
            continue; // empty label — nothing distinguishes a twin
        }
        let Some(twin) = products.get(&twin_key) else {
            continue;
        };

        if twin.price != source_price {
            report.mismatches.push(GiftPriceMismatch {
                source_key,
                twin_key,
                source_price,
                twin_price: twin.price,
            });
            continue;
        }

        // `create_gift` lives on the source, which is a pass or a product —
        // never a badge, which has no price to twin.
        let flagged = match kind {
            ResourceKind::Pass => passes.get_mut(&source_key).map(|p| &mut p.create_gift),
            ResourceKind::Product => products.get_mut(&source_key).map(|p| &mut p.create_gift),
            ResourceKind::Badge => None,
        };
        match flagged {
            Some(flag) => *flag = true,
            // Already consumed by an earlier ambiguous match.
            None => continue,
        }

        products.remove(&twin_key);
        let gift_lock_key = gift_key(key_prefix, capitalize_key, &source_key);
        if let Some(lock) = product_locks.remove(&twin_key) {
            product_locks.insert(gift_lock_key.clone(), lock);
        }

        report.merges.push(GiftMerge {
            source_key,
            source_kind: kind,
            twin_key,
            gift_lock_key,
        });
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BadgeConfig;

    fn pass(price: u64, create_gift: bool) -> PassConfig {
        PassConfig {
            name: None,
            price: Some(price),
            description: Some("desc".into()),
            icon: None,
            for_sale: true,
            regional_pricing: false,
            create_gift,
            path: None,
        }
    }

    fn product(price: u64, create_gift: bool) -> ProductConfig {
        ProductConfig {
            name: None,
            price,
            description: Some("desc".into()),
            icon: None,
            for_sale: true,
            regional_pricing: false,
            store_page: false,
            create_gift,
            path: None,
        }
    }

    fn resources_with_pass(key: &str, p: PassConfig) -> ResolvedResources {
        ResolvedResources {
            passes: BTreeMap::from([(key.to_string(), p)]),
            badges: BTreeMap::new(),
            products: BTreeMap::new(),
        }
    }

    #[test]
    fn no_gift_flag_derives_nothing() {
        let mut resources = resources_with_pass("VIP", pass(499, false));
        apply_gifts(&mut resources, "[GIFT] ", "Gift", false).unwrap();
        assert!(resources.products.is_empty());
    }

    #[test]
    fn derives_gift_product_from_pass() {
        let mut resources = resources_with_pass("VIP", pass(499, true));
        apply_gifts(&mut resources, "[GIFT] ", "Gift", false).unwrap();
        let gift = resources.products.get("GiftVIP").unwrap();
        assert_eq!(gift.name.as_deref(), Some("[GIFT] VIP"));
        assert_eq!(gift.price, 499);
        assert_eq!(gift.description.as_deref(), Some("desc"));
        assert!(!gift.store_page);
        assert!(!gift.create_gift);
    }

    #[test]
    fn derives_gift_product_from_product() {
        let mut resources = ResolvedResources {
            passes: BTreeMap::new(),
            badges: BTreeMap::new(),
            products: BTreeMap::from([("Coins100".to_string(), product(99, true))]),
        };
        apply_gifts(&mut resources, "[GIFT] ", "Gift", false).unwrap();
        let gift = resources.products.get("GiftCoins100").unwrap();
        assert_eq!(gift.price, 99);
    }

    #[test]
    fn free_pass_derives_zero_price_gift() {
        let mut resources = resources_with_pass("Free", pass(0, true));
        resources.passes.get_mut("Free").unwrap().price = None;
        apply_gifts(&mut resources, "[GIFT] ", "Gift", false).unwrap();
        assert_eq!(resources.products.get("GiftFree").unwrap().price, 0);
    }

    #[test]
    fn label_change_reflects_in_derived_name() {
        let mut resources = resources_with_pass("VIP", pass(499, true));
        apply_gifts(&mut resources, "GIFT - ", "Gift", false).unwrap();
        assert_eq!(
            resources.products.get("GiftVIP").unwrap().name.as_deref(),
            Some("GIFT - VIP")
        );
    }

    #[test]
    fn errors_on_collision_with_real_product() {
        let mut resources = resources_with_pass("VIP", pass(499, true));
        resources
            .products
            .insert("GiftVIP".to_string(), product(1, false));
        let err = apply_gifts(&mut resources, "[GIFT] ", "Gift", false).unwrap_err();
        assert!(err.to_string().contains("GiftVIP"));
    }

    #[test]
    fn errors_when_pass_and_product_share_key_both_gifted() {
        let mut resources = resources_with_pass("VIP", pass(499, true));
        resources
            .products
            .insert("VIP".to_string(), product(1, true));
        let err = apply_gifts(&mut resources, "[GIFT] ", "Gift", false).unwrap_err();
        assert!(err.to_string().contains("GiftVIP"));
    }

    #[test]
    fn is_gift_key_recognizes_base_flag() {
        let config = Config {
            experience: None,
            owner: None,
            codegen: Default::default(),
            icons: Default::default(),
            gifts: Default::default(),
            include: Default::default(),
            passes: BTreeMap::from([("VIP".to_string(), pass(499, true))]),
            badges: BTreeMap::<String, BadgeConfig>::new(),
            products: BTreeMap::new(),
            envs: BTreeMap::new(),
        };
        assert!(is_gift_key(&config, None, "GiftVIP"));
        assert!(!is_gift_key(&config, None, "VIP"));
        assert!(!is_gift_key(&config, None, "GiftOther"));
    }

    #[test]
    fn is_gift_key_false_when_flag_off() {
        let config = Config {
            experience: None,
            owner: None,
            codegen: Default::default(),
            icons: Default::default(),
            gifts: Default::default(),
            include: Default::default(),
            passes: BTreeMap::from([("VIP".to_string(), pass(499, false))]),
            badges: BTreeMap::<String, BadgeConfig>::new(),
            products: BTreeMap::new(),
            envs: BTreeMap::new(),
        };
        assert!(!is_gift_key(&config, None, "GiftVIP"));
    }

    fn product_lock(id: u64, name: &str, price: u64) -> ProductLock {
        ProductLock {
            id,
            name: name.into(),
            price,
            description: None,
            icon_asset_id: None,
            icon_hash: None,
            for_sale: true,
            regional_pricing: false,
            store_page: false,
        }
    }

    #[test]
    fn detects_and_merges_pre_existing_gift_twin_of_a_pass() {
        let mut passes = BTreeMap::from([("VIP".to_string(), pass(499, false))]);
        let mut products = BTreeMap::from([("GIFT - VIP".to_string(), product(499, false))]);
        let mut product_locks =
            BTreeMap::from([("GIFT - VIP".to_string(), product_lock(2, "GIFT - VIP", 499))]);

        let report = detect_and_merge_gift_twins(
            &mut passes,
            &mut products,
            &mut product_locks,
            "GIFT - ",
            "Gift",
            false,
        );

        assert_eq!(report.mismatches, vec![]);
        assert_eq!(report.merges.len(), 1);
        assert_eq!(report.merges[0].source_key, "VIP");
        assert_eq!(report.merges[0].source_kind, ResourceKind::Pass);
        assert_eq!(report.merges[0].gift_lock_key, "GiftVIP");

        assert!(passes["VIP"].create_gift);
        assert!(!products.contains_key("GIFT - VIP"));
        assert!(!product_locks.contains_key("GIFT - VIP"));
        assert_eq!(product_locks["GiftVIP"].id, 2);
    }

    #[test]
    fn detects_and_merges_pre_existing_gift_twin_of_a_product() {
        let mut passes = BTreeMap::new();
        let mut products = BTreeMap::from([
            ("Coins100".to_string(), product(99, false)),
            ("GIFT - Coins100".to_string(), product(99, false)),
        ]);
        let mut product_locks = BTreeMap::from([
            ("Coins100".to_string(), product_lock(1, "Coins100", 99)),
            (
                "GIFT - Coins100".to_string(),
                product_lock(2, "GIFT - Coins100", 99),
            ),
        ]);

        let report = detect_and_merge_gift_twins(
            &mut passes,
            &mut products,
            &mut product_locks,
            "GIFT - ",
            "Gift",
            false,
        );

        assert_eq!(report.merges.len(), 1);
        assert_eq!(report.merges[0].source_kind, ResourceKind::Product);
        assert!(products["Coins100"].create_gift);
        assert!(!products.contains_key("GIFT - Coins100"));
        assert!(product_locks.contains_key("GiftCoins100"));
        assert!(product_locks.contains_key("Coins100"));
    }

    #[test]
    fn price_mismatch_is_reported_but_not_merged() {
        let mut passes = BTreeMap::from([("VIP".to_string(), pass(499, false))]);
        let mut products = BTreeMap::from([("GIFT - VIP".to_string(), product(599, false))]);
        let mut product_locks =
            BTreeMap::from([("GIFT - VIP".to_string(), product_lock(2, "GIFT - VIP", 599))]);

        let report = detect_and_merge_gift_twins(
            &mut passes,
            &mut products,
            &mut product_locks,
            "GIFT - ",
            "Gift",
            false,
        );

        assert!(report.merges.is_empty());
        assert_eq!(report.mismatches.len(), 1);
        assert_eq!(report.mismatches[0].source_price, 499);
        assert_eq!(report.mismatches[0].twin_price, 599);

        // Nothing touched.
        assert!(!passes["VIP"].create_gift);
        assert!(products.contains_key("GIFT - VIP"));
        assert!(product_locks.contains_key("GIFT - VIP"));
    }

    #[test]
    fn no_matching_twin_is_a_no_op() {
        let mut passes = BTreeMap::from([("VIP".to_string(), pass(499, false))]);
        let mut products = BTreeMap::new();
        let mut product_locks = BTreeMap::new();

        let report = detect_and_merge_gift_twins(
            &mut passes,
            &mut products,
            &mut product_locks,
            "GIFT - ",
            "Gift",
            false,
        );

        assert!(report.merges.is_empty());
        assert!(report.mismatches.is_empty());
    }

    #[test]
    fn empty_label_never_matches() {
        let mut passes = BTreeMap::from([("VIP".to_string(), pass(499, false))]);
        let mut products = BTreeMap::from([("VIP".to_string(), product(499, false))]);
        let mut product_locks = BTreeMap::new();

        // With an empty label, "VIP" would trivially "match itself" — must be a no-op.
        let report = detect_and_merge_gift_twins(
            &mut passes,
            &mut products,
            &mut product_locks,
            "",
            "Gift",
            false,
        );

        assert!(report.merges.is_empty());
        assert!(report.mismatches.is_empty());
        assert!(products.contains_key("VIP"));
    }

    #[test]
    fn gift_key_capitalize_uppercases_only_the_derived_copy() {
        assert_eq!(gift_key("gift", true, "vipPass"), "giftVipPass");
        // Source already starts uppercase — no visible change either way.
        assert_eq!(gift_key("gift", true, "VIP"), "giftVIP");
        assert_eq!(gift_key("Gift", false, "vip_pass"), "Giftvip_pass");
    }

    #[test]
    fn gift_key_capitalize_handles_empty_source() {
        assert_eq!(gift_key("gift", true, ""), "gift");
    }

    #[test]
    fn apply_gifts_with_capitalize_key_produces_readable_compound_key() {
        let mut resources = resources_with_pass("vipPass", pass(499, true));
        apply_gifts(&mut resources, "[GIFT] ", "gift", true).unwrap();
        assert!(resources.products.contains_key("giftVipPass"));
        assert!(!resources.products.contains_key("giftvipPass"));
    }

    #[test]
    fn is_gift_key_recognizes_capitalized_derived_key() {
        let config = Config {
            experience: None,
            owner: None,
            codegen: Default::default(),
            icons: Default::default(),
            gifts: crate::config::GiftsConfig {
                label: "[GIFT] ".to_string(),
                key_prefix: "gift".to_string(),
                capitalize_key: true,
            },
            include: Default::default(),
            passes: BTreeMap::from([("vipPass".to_string(), pass(499, true))]),
            badges: BTreeMap::<String, BadgeConfig>::new(),
            products: BTreeMap::new(),
            envs: BTreeMap::new(),
        };
        assert!(is_gift_key(&config, None, "giftVipPass"));
        assert!(!is_gift_key(&config, None, "giftvipPass"));
        assert!(!is_gift_key(&config, None, "vipPass"));
    }
}
