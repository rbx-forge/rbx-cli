//! What one resource Roblox lists looks like as a lockfile entry.
//!
//! Shared by `pull`, which writes these entries, by the live check, which
//! compares them with the ones already in the lockfile, and by `sync`, which
//! plans against them: one conversion, so the three cannot disagree about what
//! Roblox said. `icon_hash` is always `None` here: it hashes a local file,
//! which Roblox knows nothing of.
//!
//! `prior` is the entry this one replaces, if any. It speaks for the fields
//! Roblox does not report faithfully, so that a value nobody changed never
//! reads as changed: regional pricing always, and a price or a store page
//! Roblox leaves out.

use crate::api::models::{Badge, DeveloperProduct, GamePass};
use crate::lockfile::{BadgeLock, PassLock, ProductLock};

/// A listed pass.
pub(crate) fn pass_lock(pass: &GamePass, id: u64, prior: Option<&PassLock>) -> PassLock {
    PassLock {
        id,
        name: pass.name.as_deref().unwrap_or("unnamed").to_string(),
        // `priceInformation` is optional in the response, and the
        // specification does not say when it is left out. A price that is not
        // reported is therefore not read as a pass that lost its price: what
        // the lockfile recorded stands, or a config that still states the
        // price would plan to send it on every run.
        price: pass.price().or(prior.and_then(|p| p.price)),
        description: pass.description.clone(),
        icon_asset_id: pass.icon_asset_id,
        icon_hash: None,
        for_sale: pass.is_for_sale.unwrap_or(true),
        // Regional pricing keeps whatever the lockfile last recorded (set by
        // sync) rather than what the remote reports. The remote does report
        // it, in `priceInformation.enabledFeatures`, but it reports it as on
        // whenever managed pricing is on, since managed pricing subsumes it.
        // Recording that against a config that says `regional_pricing =
        // false` would show a change on every pull that no sync can settle.
        regional_pricing: prior.is_some_and(|p| p.regional_pricing),
        // Managed pricing takes the measured value instead, which is the
        // point of reading it: a config that declares nothing raises no diff,
        // so there is nothing to go phantom.
        managed_pricing: pass.is_managed_pricing_enabled,
    }
}

/// A badge. `enabled_when_unsaid` is what an absent `enabled` means: the
/// listing returns enabled badges only, and a badge fetched by id because the
/// listing left it out is a disabled one.
pub(crate) fn badge_lock(badge: &Badge, id: u64, enabled_when_unsaid: bool) -> BadgeLock {
    BadgeLock {
        id,
        name: badge.name.as_deref().unwrap_or("unnamed").to_string(),
        description: badge.description.clone(),
        enabled: badge.enabled.unwrap_or(enabled_when_unsaid),
        icon_asset_id: badge.icon_image_id,
        icon_hash: None,
    }
}

/// A listed developer product.
pub(crate) fn product_lock(
    product: &DeveloperProduct,
    id: u64,
    prior: Option<&ProductLock>,
) -> ProductLock {
    let for_sale = product.is_for_sale.unwrap_or(true);
    let on_store_page = product.store_page_enabled.unwrap_or(false);
    ProductLock {
        id,
        name: product.name.as_deref().unwrap_or("unnamed").to_string(),
        // As for a pass: unreported, the recorded price stands.
        price: product
            .price()
            .or(prior.map(|p| p.price))
            .unwrap_or_default(),
        description: product.description.clone(),
        icon_asset_id: product.icon_image_asset_id,
        icon_hash: None,
        for_sale,
        regional_pricing: prior.is_some_and(|p| p.regional_pricing),
        managed_pricing: product.is_managed_pricing_enabled,
        // A product off sale is never on the store page, whatever was asked:
        // `sync` sends the page state the two fields imply. So off sale, the
        // answer says nothing about `store_page` and the lockfile's stands.
        // On sale, it is the setting itself.
        store_page: match prior {
            Some(prior) if !for_sale => prior.store_page,
            _ => on_store_page,
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use serde_json::json;

    use super::*;

    fn recorded_pass() -> PassLock {
        PassLock {
            id: 1,
            name: "VIP".into(),
            price: Some(499),
            description: None,
            icon_asset_id: None,
            icon_hash: None,
            for_sale: true,
            regional_pricing: true,
            managed_pricing: Some(false),
        }
    }

    fn recorded_product() -> ProductLock {
        ProductLock {
            id: 2,
            name: "Coins".into(),
            price: 99,
            description: None,
            icon_asset_id: None,
            icon_hash: None,
            for_sale: true,
            regional_pricing: false,
            managed_pricing: Some(false),
            store_page: true,
        }
    }

    fn pass(value: serde_json::Value) -> GamePass {
        serde_json::from_value(value).unwrap()
    }

    fn product(value: serde_json::Value) -> DeveloperProduct {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_reported_price_is_the_price() {
        let listed = pass(json!({
            "gamePassId": 1, "name": "VIP", "isForSale": true,
            "priceInformation": { "defaultPriceInRobux": 599 }
        }));

        assert_eq!(
            pass_lock(&listed, 1, Some(&recorded_pass())).price,
            Some(599)
        );
    }

    /// The response carries no price: what was recorded stands, and nothing
    /// reads as changed.
    #[test]
    fn a_price_roblox_does_not_report_keeps_the_recorded_one() {
        let listed = pass(json!({ "gamePassId": 1, "name": "VIP", "isForSale": false }));
        assert_eq!(
            pass_lock(&listed, 1, Some(&recorded_pass())).price,
            Some(499)
        );
        assert_eq!(
            pass_lock(&listed, 1, None).price,
            None,
            "with nothing recorded, an unreported price is no price"
        );

        let listed = product(json!({ "productId": 2, "name": "Coins", "isForSale": false }));
        assert_eq!(
            product_lock(&listed, 2, Some(&recorded_product())).price,
            99
        );
        assert_eq!(product_lock(&listed, 2, None).price, 0);
    }

    /// Off sale, Roblox answers `storePageEnabled: false` for a product the
    /// config still wants on the page once it is back on sale.
    #[test]
    fn the_store_page_of_a_product_off_sale_keeps_the_recorded_setting() {
        let off_sale = product(json!({
            "productId": 2, "name": "Coins", "isForSale": false, "storePageEnabled": false
        }));

        assert!(product_lock(&off_sale, 2, Some(&recorded_product())).store_page);
        assert!(
            !product_lock(&off_sale, 2, None).store_page,
            "with nothing recorded, the answer is all there is"
        );
    }

    /// On sale, the answer is the setting: taken off the page on the Creator
    /// Dashboard, it reads as off.
    #[test]
    fn the_store_page_of_a_product_on_sale_is_what_roblox_says() {
        let on_sale = product(json!({
            "productId": 2, "name": "Coins", "isForSale": true, "storePageEnabled": false
        }));

        assert!(!product_lock(&on_sale, 2, Some(&recorded_product())).store_page);
    }

    #[test]
    fn regional_pricing_is_always_the_recorded_one() {
        let listed = pass(json!({
            "gamePassId": 1, "name": "VIP",
            "priceInformation": { "defaultPriceInRobux": 499, "enabledFeatures": [] }
        }));

        assert!(pass_lock(&listed, 1, Some(&recorded_pass())).regional_pricing);
        assert!(!pass_lock(&listed, 1, None).regional_pricing);
    }
}
