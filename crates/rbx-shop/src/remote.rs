//! What one resource Roblox lists looks like as a lockfile entry.
//!
//! Shared by `pull`, which writes these entries, and by the live check, which
//! compares them with the ones already in the lockfile: one conversion, so the
//! two cannot disagree about what Roblox said. `icon_hash` is always `None`
//! here: it hashes a local file, which Roblox knows nothing of.

use crate::api::models::{Badge, DeveloperProduct, GamePass};
use crate::lockfile::{BadgeLock, PassLock, ProductLock};

/// A listed pass. `prior` is the entry this one replaces, if any.
pub(crate) fn pass_lock(pass: &GamePass, id: u64, prior: Option<&PassLock>) -> PassLock {
    PassLock {
        id,
        name: pass.name.as_deref().unwrap_or("unnamed").to_string(),
        price: pass.price(),
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

/// A listed developer product. `prior` as for [`pass_lock`], for the same
/// regional-pricing reason.
pub(crate) fn product_lock(
    product: &DeveloperProduct,
    id: u64,
    prior: Option<&ProductLock>,
) -> ProductLock {
    ProductLock {
        id,
        name: product.name.as_deref().unwrap_or("unnamed").to_string(),
        price: product.price().unwrap_or(0),
        description: product.description.clone(),
        icon_asset_id: product.icon_image_asset_id,
        icon_hash: None,
        for_sale: product.is_for_sale.unwrap_or(true),
        regional_pricing: prior.is_some_and(|p| p.regional_pricing),
        managed_pricing: product.is_managed_pricing_enabled,
        store_page: product.store_page_enabled.unwrap_or(false),
    }
}
