//! Per-env overlays: the same three shapes with every field optional.
//!
//! Absent means keep what the base declared, present means replace it. The
//! `apply` impls at the bottom are that rule, once per resource.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Every table `rbxshop.toml` gives a meaning to at the top level.
///
use super::*;

/// Per-env overlay grouping the three resource maps. All fields are optional:
/// merging is done resource by resource, field by field. A resource defined
/// only in the overlay (not in base) is treated as env-exclusive.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct EnvOverlay {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passes: BTreeMap<String, PassOverlay>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub badges: BTreeMap<String, BadgeOverlay>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub products: BTreeMap<String, ProductOverlay>,
}

impl EnvOverlay {
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty() && self.badges.is_empty() && self.products.is_empty()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct PassOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_sale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regional_pricing: Option<bool>,
    /// An env can turn managed pricing on or off for this pass, but cannot
    /// put it back to unset: omitting the key here means "no override",
    /// which is the only thing omission can mean in an overlay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_pricing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_gift: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct ProductOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_sale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regional_pricing: Option<bool>,
    /// See `PassOverlay::managed_pricing`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_pricing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_page: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_gift: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl PassOverlay {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.price.is_none()
            && self.description.is_none()
            && self.icon.is_none()
            && self.for_sale.is_none()
            && self.regional_pricing.is_none()
            && self.managed_pricing.is_none()
            && self.create_gift.is_none()
            && self.path.is_none()
    }
}

impl BadgeOverlay {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.icon.is_none()
            && self.enabled.is_none()
            && self.path.is_none()
    }
}

impl ProductOverlay {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.price.is_none()
            && self.description.is_none()
            && self.icon.is_none()
            && self.for_sale.is_none()
            && self.regional_pricing.is_none()
            && self.managed_pricing.is_none()
            && self.store_page.is_none()
            && self.create_gift.is_none()
            && self.path.is_none()
    }
}

impl PassConfig {
    pub fn apply_overlay(&mut self, ov: &PassOverlay) {
        if let Some(v) = &ov.name {
            self.name = Some(v.clone());
        }
        if let Some(v) = ov.price {
            self.price = Some(v);
        }
        if let Some(v) = &ov.description {
            self.description = Some(v.clone());
        }
        if let Some(v) = &ov.icon {
            self.icon = Some(v.clone());
        }
        if let Some(v) = ov.for_sale {
            self.for_sale = v;
        }
        if let Some(v) = ov.regional_pricing {
            self.regional_pricing = v;
        }
        if let Some(v) = ov.managed_pricing {
            self.managed_pricing = Some(v);
        }
        if let Some(v) = ov.create_gift {
            self.create_gift = v;
        }
        if let Some(v) = &ov.path {
            self.path = Some(v.clone());
        }
    }

    /// Build from an overlay alone (when the pass exists only in `envs.<name>`).
    pub fn from_overlay(ov: &PassOverlay) -> Self {
        Self {
            name: ov.name.clone(),
            price: ov.price,
            description: ov.description.clone(),
            icon: ov.icon.clone(),
            for_sale: ov.for_sale.unwrap_or(true),
            regional_pricing: ov.regional_pricing.unwrap_or(false),
            // Carried as-is: unset in the overlay stays unset in the config,
            // which is a state this field has and the others do not.
            managed_pricing: ov.managed_pricing,
            create_gift: ov.create_gift.unwrap_or(false),
            path: ov.path.clone(),
        }
    }
}

impl BadgeConfig {
    pub fn apply_overlay(&mut self, ov: &BadgeOverlay) {
        if let Some(v) = &ov.name {
            self.name = Some(v.clone());
        }
        if let Some(v) = &ov.description {
            self.description = Some(v.clone());
        }
        if let Some(v) = &ov.icon {
            self.icon = Some(v.clone());
        }
        if let Some(v) = ov.enabled {
            self.enabled = v;
        }
        if let Some(v) = &ov.path {
            self.path = Some(v.clone());
        }
    }

    pub fn from_overlay(ov: &BadgeOverlay) -> Self {
        Self {
            name: ov.name.clone(),
            description: ov.description.clone(),
            icon: ov.icon.clone(),
            enabled: ov.enabled.unwrap_or(true),
            path: ov.path.clone(),
        }
    }
}

impl ProductConfig {
    pub fn apply_overlay(&mut self, ov: &ProductOverlay) {
        if let Some(v) = &ov.name {
            self.name = Some(v.clone());
        }
        if let Some(v) = ov.price {
            self.price = v;
        }
        if let Some(v) = &ov.description {
            self.description = Some(v.clone());
        }
        if let Some(v) = &ov.icon {
            self.icon = Some(v.clone());
        }
        if let Some(v) = ov.for_sale {
            self.for_sale = v;
        }
        if let Some(v) = ov.regional_pricing {
            self.regional_pricing = v;
        }
        if let Some(v) = ov.managed_pricing {
            self.managed_pricing = Some(v);
        }
        if let Some(v) = ov.store_page {
            self.store_page = v;
        }
        if let Some(v) = ov.create_gift {
            self.create_gift = v;
        }
        if let Some(v) = &ov.path {
            self.path = Some(v.clone());
        }
    }

    /// Build from an overlay alone. Errors if `price` is unset (required).
    pub fn from_overlay(key: &str, ov: &ProductOverlay) -> Result<Self> {
        let price = ov.price.ok_or_else(|| {
            anyhow::anyhow!(
                "Product '{}' is defined only in an env overlay but lacks the required `price` field. \
                 Add [envs.<name>.products.{}].price or move the product into base [products.{}].",
                key, key, key
            )
        })?;
        Ok(Self {
            name: ov.name.clone(),
            price,
            description: ov.description.clone(),
            icon: ov.icon.clone(),
            for_sale: ov.for_sale.unwrap_or(true),
            regional_pricing: ov.regional_pricing.unwrap_or(false),
            // Carried as-is. See `PassConfig::from_overlay`.
            managed_pricing: ov.managed_pricing,
            store_page: ov.store_page.unwrap_or(false),
            create_gift: ov.create_gift.unwrap_or(false),
            path: ov.path.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── `is_empty`, and why it is worth this much test ──
    //
    // `pull` and `toml_write` both drop an overlay `is_empty` calls empty. So
    // a field missing from one of these chains is not a cosmetic bug: it is an
    // `[envs.<name>]` setting that disappears out of somebody's config file
    // the next time they pull, with no diff line to notice.
    //
    // Each test below destructures the overlay **without `..`**, so adding a
    // field stops this module compiling until someone states whether
    // `is_empty` should see it. The assertions matter less than that: a list
    // maintained by hand would rot the same way the function does.

    #[test]
    fn a_pass_overlay_is_not_empty_whichever_single_field_is_set() {
        let PassOverlay {
            name,
            price,
            description,
            icon,
            for_sale,
            regional_pricing,
            managed_pricing,
            create_gift,
            path,
        } = PassOverlay {
            name: Some("VIP Pass".into()),
            price: Some(499),
            description: Some("the good one".into()),
            icon: Some("vip.png".into()),
            for_sale: Some(false),
            regional_pricing: Some(true),
            managed_pricing: Some(true),
            create_gift: Some(true),
            path: Some("shop.specials".into()),
        };

        assert!(
            PassOverlay::default().is_empty(),
            "an overlay with nothing set is the one empty case"
        );

        let one_each = [
            (
                "name",
                PassOverlay {
                    name,
                    ..Default::default()
                },
            ),
            (
                "price",
                PassOverlay {
                    price,
                    ..Default::default()
                },
            ),
            (
                "description",
                PassOverlay {
                    description,
                    ..Default::default()
                },
            ),
            (
                "icon",
                PassOverlay {
                    icon,
                    ..Default::default()
                },
            ),
            (
                "for_sale",
                PassOverlay {
                    for_sale,
                    ..Default::default()
                },
            ),
            (
                "regional_pricing",
                PassOverlay {
                    regional_pricing,
                    ..Default::default()
                },
            ),
            (
                "managed_pricing",
                PassOverlay {
                    managed_pricing,
                    ..Default::default()
                },
            ),
            (
                "create_gift",
                PassOverlay {
                    create_gift,
                    ..Default::default()
                },
            ),
            (
                "path",
                PassOverlay {
                    path,
                    ..Default::default()
                },
            ),
        ];
        for (field, overlay) in one_each {
            assert!(
                !overlay.is_empty(),
                "a pass overlay setting only `{field}` would be dropped as empty"
            );
        }
    }

    #[test]
    fn a_badge_overlay_is_not_empty_whichever_single_field_is_set() {
        let BadgeOverlay {
            name,
            description,
            icon,
            enabled,
            path,
        } = BadgeOverlay {
            name: Some("Welcome".into()),
            description: Some("first win".into()),
            icon: Some("welcome.png".into()),
            enabled: Some(false),
            path: Some("rewards".into()),
        };

        assert!(BadgeOverlay::default().is_empty());

        let one_each = [
            (
                "name",
                BadgeOverlay {
                    name,
                    ..Default::default()
                },
            ),
            (
                "description",
                BadgeOverlay {
                    description,
                    ..Default::default()
                },
            ),
            (
                "icon",
                BadgeOverlay {
                    icon,
                    ..Default::default()
                },
            ),
            (
                "enabled",
                BadgeOverlay {
                    enabled,
                    ..Default::default()
                },
            ),
            (
                "path",
                BadgeOverlay {
                    path,
                    ..Default::default()
                },
            ),
        ];
        for (field, overlay) in one_each {
            assert!(
                !overlay.is_empty(),
                "a badge overlay setting only `{field}` would be dropped as empty"
            );
        }
    }

    #[test]
    fn a_product_overlay_is_not_empty_whichever_single_field_is_set() {
        let ProductOverlay {
            name,
            price,
            description,
            icon,
            for_sale,
            regional_pricing,
            managed_pricing,
            store_page,
            create_gift,
            path,
        } = ProductOverlay {
            name: Some("100 Coins".into()),
            price: Some(99),
            description: Some("coins".into()),
            icon: Some("coins.png".into()),
            for_sale: Some(false),
            regional_pricing: Some(true),
            managed_pricing: Some(true),
            store_page: Some(true),
            create_gift: Some(true),
            path: Some("shop.items".into()),
        };

        assert!(ProductOverlay::default().is_empty());

        let one_each = [
            (
                "name",
                ProductOverlay {
                    name,
                    ..Default::default()
                },
            ),
            (
                "price",
                ProductOverlay {
                    price,
                    ..Default::default()
                },
            ),
            (
                "description",
                ProductOverlay {
                    description,
                    ..Default::default()
                },
            ),
            (
                "icon",
                ProductOverlay {
                    icon,
                    ..Default::default()
                },
            ),
            (
                "for_sale",
                ProductOverlay {
                    for_sale,
                    ..Default::default()
                },
            ),
            (
                "regional_pricing",
                ProductOverlay {
                    regional_pricing,
                    ..Default::default()
                },
            ),
            (
                "managed_pricing",
                ProductOverlay {
                    managed_pricing,
                    ..Default::default()
                },
            ),
            (
                "store_page",
                ProductOverlay {
                    store_page,
                    ..Default::default()
                },
            ),
            (
                "create_gift",
                ProductOverlay {
                    create_gift,
                    ..Default::default()
                },
            ),
            (
                "path",
                ProductOverlay {
                    path,
                    ..Default::default()
                },
            ),
        ];
        for (field, overlay) in one_each {
            assert!(
                !overlay.is_empty(),
                "a product overlay setting only `{field}` would be dropped as empty"
            );
        }
    }

    // ── `apply_overlay` ──
    //
    // Ten branches that each assign one field. The failure mode is a
    // copy-paste: a branch reading one field and writing another. Nothing
    // about that is visible at the call site, and the result is a sync sending
    // the wrong value for one env only.

    #[test]
    fn every_pass_overlay_field_lands_on_its_own_target() {
        let mut cfg = PassConfig {
            name: None,
            price: None,
            description: None,
            icon: None,
            for_sale: true,
            regional_pricing: false,
            managed_pricing: None,
            create_gift: false,
            path: None,
        };

        cfg.apply_overlay(&PassOverlay {
            name: Some("VIP Pass".into()),
            price: Some(499),
            description: Some("the good one".into()),
            icon: Some("vip.png".into()),
            // The three booleans are set away from their defaults, or an
            // assignment to the wrong one would still read as correct.
            for_sale: Some(false),
            regional_pricing: Some(true),
            managed_pricing: Some(true),
            create_gift: Some(true),
            path: Some("shop.specials".into()),
        });

        assert_eq!(cfg.name.as_deref(), Some("VIP Pass"));
        assert_eq!(cfg.price, Some(499));
        assert_eq!(cfg.description.as_deref(), Some("the good one"));
        assert_eq!(cfg.icon.as_deref(), Some(Path::new("vip.png")));
        assert!(!cfg.for_sale);
        assert!(cfg.regional_pricing);
        assert_eq!(cfg.managed_pricing, Some(true));
        assert!(cfg.create_gift);
        assert_eq!(cfg.path.as_deref(), Some("shop.specials"));
    }

    #[test]
    fn every_product_overlay_field_lands_on_its_own_target() {
        let mut cfg = ProductConfig {
            name: None,
            price: 1,
            description: None,
            icon: None,
            for_sale: true,
            regional_pricing: false,
            managed_pricing: None,
            store_page: false,
            create_gift: false,
            path: None,
        };

        cfg.apply_overlay(&ProductOverlay {
            name: Some("100 Coins".into()),
            price: Some(99),
            description: Some("coins".into()),
            icon: Some("coins.png".into()),
            for_sale: Some(false),
            regional_pricing: Some(true),
            managed_pricing: Some(true),
            store_page: Some(true),
            create_gift: Some(true),
            path: Some("shop.items".into()),
        });

        assert_eq!(cfg.name.as_deref(), Some("100 Coins"));
        assert_eq!(cfg.price, 99);
        assert_eq!(cfg.description.as_deref(), Some("coins"));
        assert_eq!(cfg.icon.as_deref(), Some(Path::new("coins.png")));
        assert!(!cfg.for_sale);
        assert!(cfg.regional_pricing);
        assert_eq!(cfg.managed_pricing, Some(true));
        assert!(cfg.store_page);
        assert!(cfg.create_gift);
        assert_eq!(cfg.path.as_deref(), Some("shop.items"));
    }

    #[test]
    fn every_badge_overlay_field_lands_on_its_own_target() {
        let mut cfg = BadgeConfig {
            name: None,
            description: None,
            icon: None,
            enabled: true,
            path: None,
        };

        cfg.apply_overlay(&BadgeOverlay {
            name: Some("Welcome".into()),
            description: Some("first win".into()),
            icon: Some("welcome.png".into()),
            enabled: Some(false),
            path: Some("rewards".into()),
        });

        assert_eq!(cfg.name.as_deref(), Some("Welcome"));
        assert_eq!(cfg.description.as_deref(), Some("first win"));
        assert_eq!(cfg.icon.as_deref(), Some(Path::new("welcome.png")));
        assert!(!cfg.enabled);
        assert_eq!(cfg.path.as_deref(), Some("rewards"));
    }

    /// An overlay that states nothing leaves every field as it was. The
    /// `if let Some(v)` guards are what make a partial overlay partial, and
    /// without this the tests above would pass just as well if each branch
    /// assigned unconditionally.
    #[test]
    fn an_empty_overlay_changes_nothing() {
        let mut cfg = PassConfig {
            name: Some("kept".into()),
            price: Some(1),
            description: Some("kept".into()),
            icon: Some("kept.png".into()),
            for_sale: false,
            regional_pricing: true,
            managed_pricing: Some(false),
            create_gift: true,
            path: Some("kept".into()),
        };

        cfg.apply_overlay(&PassOverlay::default());

        assert_eq!(cfg.name.as_deref(), Some("kept"));
        assert_eq!(cfg.price, Some(1));
        assert_eq!(cfg.description.as_deref(), Some("kept"));
        assert_eq!(cfg.icon.as_deref(), Some(Path::new("kept.png")));
        assert!(!cfg.for_sale);
        assert!(cfg.regional_pricing);
        assert_eq!(cfg.managed_pricing, Some(false));
        assert!(cfg.create_gift);
        assert_eq!(cfg.path.as_deref(), Some("kept"));
    }
}
