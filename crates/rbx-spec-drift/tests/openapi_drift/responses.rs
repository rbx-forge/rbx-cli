//! The fields the tool reads from a response, against the schema the spec gives
//! that response.
//!
//! These models make most fields optional or defaulted, so a field Roblox
//! renames does not fail to parse. It reads as `None` or as empty: on a shop
//! create that once meant an id recorded as 0, and on a list it means a page
//! that looks empty. So this runs against the document on every refresh, which
//! is before anyone hits it.
//!
//! One direction only: a property the spec adds and these models ignore is not a
//! problem, because reading less than is sent is what `Deserialize` is for.
//!
//! The limit is the document itself. This proves the models agree with what
//! Roblox says it sends, not with what it does send.

use std::collections::BTreeSet;
use std::fs;

use serde_json::Value;

use crate::bodies::serialised_keys;
use crate::{extract::*, spec::*};

const SHOP: &str = "crates/rbx-shop/src/api/models.rs";
const ADS: &str = "crates/rbx-ads/src/model.rs";
const BAN: &str = "crates/rbx-ban/src/model.rs";
const DATA: &str = "crates/rbx-data/src/model.rs";
const GROUP: &str = "crates/rbx-group/src/model.rs";

/// Where a model's schema is: the 200 response of a route, or a property of it
/// for a nested type with no route of its own, or the items of that property
/// when it is a list.
enum Source {
    Route(&'static str, &'static str),
    Property(&'static str, &'static str, &'static str),
    Items(&'static str, &'static str, &'static str),
}

use Source::{Items, Property, Route};

/// `(file, model, source)`. Left out: `BadgeIconResponse`, whose route documents
/// no response, `AssetDeliveryResponse`, the public asset delivery this document
/// does not describe, the analytics models, whose shapes are named per query, and
/// `GroupRole`, which flattens another struct into itself and which the scan
/// refuses rather than reading half of.
const RESPONSES: &[(&str, &str, Source)] = &[
    (
        SHOP,
        "GamePass",
        Route("/game-passes/v1/universes/{universeId}/game-passes", "post"),
    ),
    (
        SHOP,
        "ListGamePassesResponse",
        Route(
            "/game-passes/v1/universes/{universeId}/game-passes/creator",
            "get",
        ),
    ),
    (
        SHOP,
        "DeveloperProduct",
        Route(
            "/developer-products/v2/universes/{universeId}/developer-products",
            "post",
        ),
    ),
    (
        SHOP,
        "ListDeveloperProductsResponse",
        Route(
            "/developer-products/v2/universes/{universeId}/developer-products/creator",
            "get",
        ),
    ),
    (
        SHOP,
        "Badge",
        Route("/legacy-badges/v1/universes/{universeId}/badges", "post"),
    ),
    (SHOP, "Badge", Route("/v1/badges/{badgeId}", "get")),
    (
        SHOP,
        "ListBadgesResponse",
        Route("/v1/universes/{universeId}/badges", "get"),
    ),
    (
        SHOP,
        "PriceInformation",
        Property(
            "/game-passes/v1/universes/{universeId}/game-passes",
            "post",
            "priceInformation",
        ),
    ),
    (
        SHOP,
        "PriceInformation",
        Property(
            "/developer-products/v2/universes/{universeId}/developer-products",
            "post",
            "priceInformation",
        ),
    ),
    (
        ADS,
        "Campaign",
        Route("/ads-management/v1/campaigns/{id}", "get"),
    ),
    (
        ADS,
        "ListCampaigns",
        Route("/ads-management/v1/campaigns", "get"),
    ),
    (
        BAN,
        "UserRestriction",
        Route(
            "/cloud/v2/universes/{universe_id}/user-restrictions/{user_restriction_id}",
            "get",
        ),
    ),
    (
        BAN,
        "GameJoinRestriction",
        Property(
            "/cloud/v2/universes/{universe_id}/user-restrictions/{user_restriction_id}",
            "get",
            "gameJoinRestriction",
        ),
    ),
    (
        BAN,
        "RestrictionPage",
        Route("/cloud/v2/universes/{universe_id}/user-restrictions", "get"),
    ),
    (
        BAN,
        "LogPage",
        Route(
            "/cloud/v2/universes/{universe_id}/user-restrictions:listLogs",
            "get",
        ),
    ),
    (
        BAN,
        "RestrictionLog",
        Items(
            "/cloud/v2/universes/{universe_id}/user-restrictions:listLogs",
            "get",
            "logs",
        ),
    ),
    (
        GROUP,
        "ListGroupRolesResponse",
        Route("/cloud/v2/groups/{group_id}/roles", "get"),
    ),
    (
        GROUP,
        "ListGroupMembershipsResponse",
        Route("/cloud/v2/groups/{group_id}/memberships", "get"),
    ),
    (
        GROUP,
        "GroupMembership",
        Items(
            "/cloud/v2/groups/{group_id}/memberships",
            "get",
            "groupMemberships",
        ),
    ),
    (
        DATA,
        "StoreList",
        Route("/cloud/v2/universes/{universe_id}/data-stores", "get"),
    ),
    (
        DATA,
        "DataStore",
        Items(
            "/cloud/v2/universes/{universe_id}/data-stores",
            "get",
            "dataStores",
        ),
    ),
    (
        DATA,
        "DataStoreEntry",
        Route(
            "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}/entries/{entry_id}",
            "get",
        ),
    ),
    (
        DATA,
        "EntryList",
        Route(
            "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}/entries",
            "get",
        ),
    ),
    (
        DATA,
        "SnapshotResult",
        Route(
            "/cloud/v2/universes/{universe_id}/data-stores:snapshot",
            "post",
        ),
    ),
];

/// The properties a schema documents, following `$ref` and `allOf`.
fn property_names(spec: &Value, schema: &Value) -> BTreeSet<String> {
    let schema = resolve(spec, schema);
    let mut names: BTreeSet<String> = schema["properties"]
        .as_object()
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();
    for part in schema["allOf"].as_array().into_iter().flatten() {
        names.extend(property_names(spec, part));
    }
    names
}

/// The JSON schema of a route's 200 response, or `Null` when it documents none.
fn response_schema<'a>(spec: &'a Value, path: &str, method: &str) -> &'a Value {
    &spec["paths"][path][method]["responses"]["200"]["content"]["application/json"]["schema"]
}

fn schema_for<'a>(spec: &'a Value, source: &Source) -> &'a Value {
    match source {
        Route(path, method) => response_schema(spec, path, method),
        Property(path, method, property) => {
            &resolve(spec, response_schema(spec, path, method))["properties"][*property]
        }
        Items(path, method, property) => {
            let list = &resolve(spec, response_schema(spec, path, method))["properties"][*property];
            &resolve(spec, list)["items"]
        }
    }
}

/// The keys a model reads that the schema does not document.
fn unknown_keys(keys: &[String], documented: &BTreeSet<String>) -> Vec<String> {
    keys.iter()
        .filter(|key| !documented.contains(*key))
        .cloned()
        .collect()
}

#[test]
fn every_response_model_reads_only_what_the_spec_documents() {
    let root = repo_root();
    let spec = load_document(&root);

    let mut report = Vec::new();
    for (file, model, source) in RESPONSES {
        let src = fs::read_to_string(root.join(file))
            .unwrap_or_else(|e| panic!("cannot read {file}: {e}"));
        let keys = serialised_keys(&strip_tests(&src), model);
        assert!(
            !keys.is_empty(),
            "read no key from {model} in {file}: the scan found the struct and nothing in it, \
             so it would be comparing empty sets."
        );
        let documented = property_names(&spec, schema_for(&spec, source));
        assert!(
            !documented.is_empty(),
            "the spec documents no property where {model} ({file}) expects its schema. An \
             unresolvable schema must not read as an empty one: if Roblox moved the route, \
             update it here."
        );
        for key in unknown_keys(&keys, &documented) {
            report.push(format!(
                "{model} ({file}) reads `{key}`, which the spec does not document here. \
                 Known: {documented:?}"
            ));
        }
    }

    assert!(
        report.is_empty(),
        "\n\nThe response models disagree with the vendored spec. These fields read as empty, \
         not as an error:\n\n  {}\n\n",
        report.join("\n  ")
    );
}

mod guards {
    use super::*;
    use serde_json::json;

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_renamed_field_is_reported_and_an_added_one_is_not() {
        let spec = json!({});
        let documented = property_names(
            &spec,
            &json!({ "properties": { "gamePassId": {}, "somethingNew": {} } }),
        );
        assert!(unknown_keys(&keys(&["gamePassId"]), &documented).is_empty());
        assert_eq!(unknown_keys(&keys(&["id"]), &documented), ["id"]);
    }

    #[test]
    fn properties_are_found_through_a_ref_and_an_all_of() {
        let spec = json!({ "components": { "schemas": {
            "Base": { "properties": { "id": {} } },
            "Whole": { "allOf": [ { "$ref": "#/components/schemas/Base" }, { "properties": { "name": {} } } ] },
        }}});
        let found = property_names(&spec, &json!({ "$ref": "#/components/schemas/Whole" }));
        assert!(found.contains("id") && found.contains("name"), "{found:?}");
    }

    #[test]
    fn a_schema_that_does_not_resolve_is_empty_and_the_test_refuses_that() {
        let found = property_names(&json!({}), &Value::Null);
        assert!(found.is_empty());
    }

    /// The models outside shop rename every field this way, so the scan has to
    /// produce the names on the wire rather than the Rust ones.
    #[test]
    fn the_scan_reads_camel_case_skip_and_an_explicit_rename() {
        let src = r#"
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub display_name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip)]
    pub local_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
    pub id: u64,
}
"#;
        assert_eq!(
            serialised_keys(src, "Sample"),
            ["displayName", "type", "nextPageToken", "id"]
        );
    }

    #[test]
    #[should_panic(expected = "knows `camelCase`")]
    fn the_scan_refuses_a_rename_all_it_does_not_know() {
        let src = "#[serde(rename_all = \"SCREAMING_SNAKE_CASE\")]\npub struct Shout {\n    pub a: u8,\n}\n";
        serialised_keys(src, "Shout");
    }
}
