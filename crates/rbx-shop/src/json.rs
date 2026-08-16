//! What `rbx shop show --json` and `rbx shop list --json` write to stdout.
//!
//! Two documents about the same three nouns, from opposite sides, and the
//! whole point of this module is that a reader can tell them apart:
//!
//! - `show` reports **what the repo declares**: `rbxshop.toml` with serde
//!   defaults filled in and the per-env overlay applied. Keyed by TOML key,
//!   because that is what `--env` overlays, `rename` moves and codegen emits.
//! - `list` reports **what Roblox currently has**: an array of remote
//!   resources, keyed by nothing, because a remote resource has an id and no
//!   config key at all.
//!
//! Neither is a drift report. `rbx check --json` already owns that word: its
//! rows carry `outcome`, `summary` and `details` and say whether declared and
//! recorded state agree. Nothing here is named any of those, so a filter
//! written against one document cannot half-read the other. What the two do
//! share is deliberate: `schema_version` comes from the same constant, and
//! `env` is the same env name omitted under the same rule.
//!
//! Field names are documented in `docs/shop.md` and are the compatibility
//! surface.

use std::collections::BTreeMap;

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::api::models::{Badge as RemoteBadge, DeveloperProduct, GamePass};
use crate::config::{BadgeConfig, PassConfig, ProductConfig, ResolvedResources, ResourceKind};

/// One `shop show` invocation: the declared state of one env.
#[derive(Debug, Serialize)]
pub struct ShowDocument {
    pub schema_version: u32,
    /// The `rbxshop.toml` this was read from, as given on the command line or
    /// defaulted. Present because a `--config` mistake otherwise looks like an
    /// empty shop.
    ///
    /// Its absence is meaningful in the other direction too: no document in
    /// this module carries `config_file` unless a local file was actually
    /// read, so `shop list` does not have one.
    pub config_file: String,
    /// The env whose overlay was applied. **Absent** for the base view — no
    /// `--env`, or `--env all`, which has no single overlay to resolve. Same
    /// omission rule `rbx check --json` uses for its own `env`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// The `experience` section of `rbxshop.toml`, as the file spells it.
    /// **Absent** when the file has no such section.
    ///
    /// Nested rather than flattened to a top-level `universe_id` on purpose:
    /// this is what the file declares as a fallback target, which is not
    /// necessarily the universe `--env` resolves to. `shop list --json` has a
    /// bare `universe_id` and that one *is* the universe that was queried;
    /// giving the two different shapes is what stops them being read as the
    /// same fact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experience: Option<Experience>,
    /// Declared game passes, keyed by TOML key. An object, not an array: the
    /// key is the handle every other command names, so a consumer looks one up
    /// rather than walking a list.
    ///
    /// No `totals` object anywhere in this module. `rbx check --json` has one
    /// and it counts outcomes; one here would count rows under the same name.
    /// `.passes | length` is the count, and it cannot be misread.
    pub passes: BTreeMap<String, Pass>,
    pub badges: BTreeMap<String, Badge>,
    pub products: BTreeMap<String, Product>,
}

/// The `experience` section of `rbxshop.toml`.
#[derive(Debug, Serialize)]
pub struct Experience {
    /// A string, like every other id in every other document here. Ids
    /// identify rather than count, they already exceed 2^53 for places, and a
    /// consumer parsing JSON with doubles rounds anything past it. Prices stay
    /// numbers, because Robux is a quantity.
    pub universe_id: String,
}

/// One declared game pass.
///
/// Optional fields are omitted rather than emitted as `null`, and every
/// omission means something the human view spells out in words.
#[derive(Debug, Serialize)]
pub struct Pass {
    /// The `name` override. **Absent** when the file sets none, in which case
    /// the TOML key is the display name — `.passes | to_entries[] |
    /// (.value.name // .key)` reproduces what the table prints. Reporting the
    /// resolved name would make the document unable to say "this one inherits
    /// its key".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Robux. **Absent** when the file sets no price, which for a pass means
    /// free — the word the human view prints in that cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    pub for_sale: bool,
    pub regional_pricing: bool,
    pub create_gift: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Icon path, relative to the config file, as the file spells it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The codegen path override. **Absent** when this resource lands wherever
    /// the `codegen` section puts its kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One declared badge. No price: badges are not sold.
#[derive(Debug, Serialize)]
pub struct Badge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One declared developer product.
#[derive(Debug, Serialize)]
pub struct Product {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Robux. Always present, unlike a pass's: the field is required in the
    /// file, so there is no "unset" state to report.
    pub price: u64,
    pub for_sale: bool,
    pub regional_pricing: bool,
    pub store_page: bool,
    pub create_gift: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Render a path the way the file spells it, not the way this OS would.
fn icon(path: Option<&std::path::Path>) -> Option<String> {
    path.map(|p| p.display().to_string())
}

impl From<&PassConfig> for Pass {
    fn from(pass: &PassConfig) -> Self {
        Self {
            name: pass.name.clone(),
            price: pass.price,
            for_sale: pass.for_sale,
            regional_pricing: pass.regional_pricing,
            create_gift: pass.create_gift,
            description: pass.description.clone(),
            icon: icon(pass.icon.as_deref()),
            path: pass.path.clone(),
        }
    }
}

impl From<&BadgeConfig> for Badge {
    fn from(badge: &BadgeConfig) -> Self {
        Self {
            name: badge.name.clone(),
            enabled: badge.enabled,
            description: badge.description.clone(),
            icon: icon(badge.icon.as_deref()),
            path: badge.path.clone(),
        }
    }
}

impl From<&ProductConfig> for Product {
    fn from(product: &ProductConfig) -> Self {
        Self {
            name: product.name.clone(),
            price: product.price,
            for_sale: product.for_sale,
            regional_pricing: product.regional_pricing,
            store_page: product.store_page,
            create_gift: product.create_gift,
            description: product.description.clone(),
            icon: icon(product.icon.as_deref()),
            path: product.path.clone(),
        }
    }
}

impl ShowDocument {
    /// Build the document from what `show` already resolved.
    ///
    /// Pure, and deliberately so: the renderer prints, this decides what the
    /// document says, and a test can therefore assert the shape without a
    /// process to capture.
    pub fn new(
        config_path: &std::path::Path,
        env: Option<&str>,
        universe_id: Option<u64>,
        resources: &ResolvedResources,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            config_file: config_path.display().to_string(),
            env: env.map(str::to_string),
            experience: universe_id.map(|universe_id| Experience {
                universe_id: universe_id.to_string(),
            }),
            passes: resources
                .passes
                .iter()
                .map(|(key, pass)| (key.clone(), Pass::from(pass)))
                .collect(),
            badges: resources
                .badges
                .iter()
                .map(|(key, badge)| (key.clone(), Badge::from(badge)))
                .collect(),
            products: resources
                .products
                .iter()
                .map(|(key, product)| (key.clone(), Product::from(product)))
                .collect(),
        }
    }
}

/// One `shop list` invocation: what Roblox has, for one kind, right now.
///
/// Nothing here is keyed by a config key, because a remote resource does not
/// have one until somebody writes it into `rbxshop.toml`. That is the
/// difference from `ShowDocument` in one sentence, and it is why this is an
/// array and that is an object.
#[derive(Debug, Serialize)]
pub struct ListDocument {
    pub schema_version: u32,
    /// The env named on the command line. **Absent** when there was none and
    /// the target came from `experience` in `rbxshop.toml` instead — the
    /// internal `default` placeholder is not an env name and is never emitted
    /// as one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// The universe that was queried. Bare, unlike `show`'s nested
    /// `experience`: this one is the resolved target, not a declaration.
    /// A string, for the reason on `Experience::universe_id`.
    pub universe_id: String,
    /// Which kind was asked for, in its canonical CLI spelling: `passes`,
    /// `badges`, `products`. On the envelope rather than repeated on every
    /// row, because one invocation lists exactly one kind.
    pub resource: &'static str,
    /// One object per remote resource, in the order Roblox returned them.
    pub resources: Vec<Resource>,
}

/// One remote resource.
///
/// One struct for all three kinds, with the fields that do not apply simply
/// absent: a badge has no `price`, a pass has no `enabled`. Everything is
/// optional because Roblox is free to omit any of it, and an absent field is
/// not the same fact as a zero one.
#[derive(Debug, Default, Serialize)]
pub struct Resource {
    /// The Roblox id. This is the handle for a remote resource, the way the
    /// TOML key is the handle for a declared one. A string, for the reason on
    /// `Experience::universe_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Robux, as Roblox reports it. **Absent** when it reported no price,
    /// which the human table renders as `Free` for a pass and `-` for a
    /// product.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_sale: Option<bool>,
    /// Badges only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Developer products only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_page: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_asset_id: Option<String>,
}

impl From<&GamePass> for Resource {
    fn from(pass: &GamePass) -> Self {
        Self {
            id: pass.id.map(|id| id.to_string()),
            name: pass.name.clone(),
            description: pass.description.clone(),
            price: pass.price(),
            for_sale: pass.is_for_sale,
            icon_asset_id: pass.icon_asset_id.map(|id| id.to_string()),
            ..Self::default()
        }
    }
}

impl From<&RemoteBadge> for Resource {
    fn from(badge: &RemoteBadge) -> Self {
        Self {
            id: badge.id.map(|id| id.to_string()),
            name: badge.name.clone(),
            description: badge.description.clone(),
            enabled: badge.enabled,
            icon_asset_id: badge.icon_image_id.map(|id| id.to_string()),
            ..Self::default()
        }
    }
}

impl From<&DeveloperProduct> for Resource {
    fn from(product: &DeveloperProduct) -> Self {
        Self {
            id: product.id.map(|id| id.to_string()),
            name: product.name.clone(),
            description: product.description.clone(),
            price: product.price(),
            for_sale: product.is_for_sale,
            store_page: product.store_page_enabled,
            icon_asset_id: product.icon_image_asset_id.map(|id| id.to_string()),
            ..Self::default()
        }
    }
}

impl ListDocument {
    pub fn new(
        env: Option<&str>,
        universe_id: u64,
        kind: ResourceKind,
        resources: Vec<Resource>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.map(str::to_string),
            universe_id: universe_id.to_string(),
            resource: kind.section(),
            resources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::Config;

    fn parsed(document: &impl Serialize) -> serde_json::Value {
        let mut buf = Vec::new();
        rbx_core::output::write_json(&mut buf, document).expect("write");
        serde_json::from_slice(&buf).expect("the document must be valid JSON")
    }

    const SAMPLE: &str = r#"
[experience]
universe_id = 100

[passes.vip]
name = "VIP Pass"
price = 199
description = "the good one"
icon = "icons/vip.png"

[passes.starter]
for_sale = false

[badges.first_win]
name = "First Win"

[products.coins_100]
price = 50
store_page = true

[envs.prod.passes.vip]
price = 299

[envs.prod.passes.prod_only]
price = 10
"#;

    fn document(env: Option<&str>) -> serde_json::Value {
        let config: Config = toml::from_str(SAMPLE).expect("sample must parse");
        let resources = config.resolve_env(env).expect("env must resolve");
        parsed(&ShowDocument::new(
            std::path::Path::new("rbxshop.toml"),
            env,
            config.experience.as_ref().map(|e| e.universe_id),
            &resources,
        ))
    }

    #[test]
    fn the_show_envelope_carries_the_documented_fields() {
        let doc = document(None);

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["config_file"], "rbxshop.toml");
        assert_eq!(
            doc["experience"]["universe_id"], "100",
            "ids are strings in every document this tool writes"
        );
        assert_eq!(doc["passes"]["vip"]["name"], "VIP Pass");
        assert_eq!(doc["passes"]["vip"]["price"], 199);
        assert_eq!(doc["passes"]["vip"]["for_sale"], true);
        assert_eq!(doc["passes"]["vip"]["icon"], "icons/vip.png");
        assert_eq!(doc["passes"]["starter"]["for_sale"], false);
        assert_eq!(doc["badges"]["first_win"]["enabled"], true);
        assert_eq!(doc["products"]["coins_100"]["price"], 50);
        assert_eq!(doc["products"]["coins_100"]["store_page"], true);
    }

    /// The word `rbx check --json` owns. A declared-state document says
    /// nothing about whether anything is in sync, so a filter reaching for an
    /// outcome must find nothing rather than something plausible.
    #[test]
    fn a_declared_document_carries_no_drift_vocabulary() {
        let doc = document(Some("prod"));

        for word in ["outcome", "checks", "check", "tool", "summary", "details"] {
            assert!(doc.get(word).is_none(), "{word} must not appear: {doc}");
        }
        // And no `totals`, which in `rbx check --json` counts outcomes.
        assert!(doc.get("totals").is_none(), "{doc}");
    }

    /// The name is the override and never the resolved fallback, so a consumer
    /// can still tell "renamed" from "named by its key".
    #[test]
    fn an_absent_name_means_the_key_is_the_name() {
        let doc = document(None);

        assert!(doc["passes"]["starter"].get("name").is_none(), "{doc}");
        assert_eq!(doc["passes"]["vip"]["name"], "VIP Pass");
    }

    /// A pass with no price is free, and that is the human view's own word for
    /// it. Omitting the key is how the document says so.
    #[test]
    fn a_pass_without_a_price_omits_the_key_rather_than_emitting_null() {
        let doc = document(None);

        assert!(doc["passes"]["starter"].get("price").is_none(), "{doc}");
        assert!(
            doc["passes"]["starter"].get("description").is_none(),
            "{doc}"
        );
    }

    /// `--env` is the whole reason `show` resolves anything: the overlay has
    /// to be applied, and env-exclusive resources have to appear.
    #[test]
    fn the_env_overlay_is_applied_and_the_env_is_named() {
        let doc = document(Some("prod"));

        assert_eq!(doc["env"], "prod");
        assert_eq!(doc["passes"]["vip"]["price"], 299);
        assert_eq!(doc["passes"]["prod_only"]["price"], 10);
    }

    /// The base view has no single overlay, so it omits `env` rather than
    /// inventing a name for "none" — the same rule `rbx check --json` uses.
    #[test]
    fn the_base_view_omits_the_env_and_the_env_exclusive_resources() {
        let doc = document(None);

        assert!(doc.get("env").is_none(), "{doc}");
        assert!(doc["passes"].get("prod_only").is_none(), "{doc}");
        assert_eq!(doc["passes"]["vip"]["price"], 199);
    }

    /// An empty shop is a document with empty objects, not a missing document:
    /// `.passes | length == 0` has to be answerable.
    #[test]
    fn an_empty_shop_is_empty_objects_not_absent_ones() {
        let config: Config = toml::from_str("").expect("an empty file is a valid config");
        let resources = config.resolve_env(None).expect("nothing to resolve");
        let doc = parsed(&ShowDocument::new(
            std::path::Path::new("rbxshop.toml"),
            None,
            None,
            &resources,
        ));

        assert_eq!(doc["passes"].as_object().map(|m| m.len()), Some(0));
        assert_eq!(doc["badges"].as_object().map(|m| m.len()), Some(0));
        assert_eq!(doc["products"].as_object().map(|m| m.len()), Some(0));
        assert!(doc.get("experience").is_none(), "{doc}");
    }

    #[test]
    fn the_list_envelope_names_the_kind_in_its_cli_spelling() {
        let passes: Vec<GamePass> = serde_json::from_str(
            r#"[
                {"gamePassId":1,"name":"VIP","isForSale":true,
                 "priceInformation":{"defaultPriceInRobux":199}},
                {"gamePassId":2,"name":"Free"}
            ]"#,
        )
        .expect("fixture");
        let doc = parsed(&ListDocument::new(
            Some("prod"),
            100,
            ResourceKind::Pass,
            passes.iter().map(Resource::from).collect(),
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["env"], "prod");
        assert_eq!(
            doc["universe_id"], "100",
            "ids are strings in every document this tool writes"
        );
        assert_eq!(doc["resource"], "passes");
        // The id is a string and the price beside it is not: that pair is the
        // whole convention in two lines. Ids identify, prices count.
        assert_eq!(doc["resources"][0]["id"], "1");
        assert_eq!(doc["resources"][0]["price"], 199);
        assert_eq!(doc["resources"][0]["for_sale"], true);
        // Roblox said nothing about either, so the document says nothing.
        assert!(doc["resources"][1].get("price").is_none(), "{doc}");
        assert!(doc["resources"][1].get("for_sale").is_none(), "{doc}");
    }

    /// `list` is about the remote side and carries no `config_file`, because
    /// no local file was read. The absence is the signal.
    #[test]
    fn the_list_document_does_not_claim_to_have_read_a_config_file() {
        let doc = parsed(&ListDocument::new(
            None,
            100,
            ResourceKind::Badge,
            Vec::new(),
        ));

        assert!(doc.get("config_file").is_none(), "{doc}");
        assert!(doc.get("env").is_none(), "{doc}");
        assert_eq!(doc["resource"], "badges");
        assert_eq!(doc["resources"].as_array().map(Vec::len), Some(0));
    }

    /// Each kind fills only the fields that mean something for it, so one
    /// filter reads all three without tripping over a field that would have
    /// been `null`.
    #[test]
    fn a_badge_row_has_no_price_and_a_product_row_has_a_store_page() {
        let badge: RemoteBadge =
            serde_json::from_str(r#"{"id":7,"name":"First Win","enabled":true}"#).expect("fixture");
        let badge = parsed(&Resource::from(&badge));
        assert_eq!(badge["enabled"], true);
        assert!(badge.get("price").is_none(), "{badge}");
        assert!(badge.get("store_page").is_none(), "{badge}");

        let product: DeveloperProduct = serde_json::from_str(
            r#"{"productId":9,"name":"Coins","storePageEnabled":false,
                "priceInformation":{"defaultPriceInRobux":50}}"#,
        )
        .expect("fixture");
        let product = parsed(&Resource::from(&product));
        assert_eq!(product["price"], 50);
        assert_eq!(product["store_page"], false);
        assert!(product.get("enabled").is_none(), "{product}");
    }
}
