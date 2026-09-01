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
            store_page: ov.store_page.unwrap_or(false),
            create_gift: ov.create_gift.unwrap_or(false),
            path: ov.path.clone(),
        })
    }
}
