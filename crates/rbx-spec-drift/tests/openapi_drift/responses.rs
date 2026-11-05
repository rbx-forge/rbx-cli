//! The fields `rbx shop` reads from a response, against the schema the spec
//! gives that response.
//!
//! Every field on these models is optional, so a field Roblox renames does not
//! fail to parse. It reads as `None`, and on a create that used to mean an id
//! recorded as 0. So this runs against the document on every refresh, which is
//! before anyone hits it.
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

const MODELS: &str = "crates/rbx-shop/src/api/models.rs";

/// Where a schema comes from: the response of a route, or a property of another
/// schema, for a nested type that has no route of its own.
enum Source {
    Route {
        path: &'static str,
        method: &'static str,
        status: &'static str,
    },
    Property {
        schema_of: &'static str,
        property: &'static str,
    },
}

/// `(model, where its schema is)`. `BadgeIconResponse` is not here because the
/// spec documents no response for that route, and `AssetDeliveryResponse` is the
/// public asset delivery, which this document does not describe.
const RESPONSES: &[(&str, Source)] = &[
    (
        "GamePass",
        Source::Route {
            path: "/game-passes/v1/universes/{universeId}/game-passes",
            method: "post",
            status: "200",
        },
    ),
    (
        "ListGamePassesResponse",
        Source::Route {
            path: "/game-passes/v1/universes/{universeId}/game-passes/creator",
            method: "get",
            status: "200",
        },
    ),
    (
        "DeveloperProduct",
        Source::Route {
            path: "/developer-products/v2/universes/{universeId}/developer-products",
            method: "post",
            status: "200",
        },
    ),
    (
        "ListDeveloperProductsResponse",
        Source::Route {
            path: "/developer-products/v2/universes/{universeId}/developer-products/creator",
            method: "get",
            status: "200",
        },
    ),
    (
        "Badge",
        Source::Route {
            path: "/legacy-badges/v1/universes/{universeId}/badges",
            method: "post",
            status: "200",
        },
    ),
    (
        "Badge",
        Source::Route {
            path: "/v1/badges/{badgeId}",
            method: "get",
            status: "200",
        },
    ),
    (
        "ListBadgesResponse",
        Source::Route {
            path: "/v1/universes/{universeId}/badges",
            method: "get",
            status: "200",
        },
    ),
    (
        "PriceInformation",
        Source::Property {
            schema_of: "/game-passes/v1/universes/{universeId}/game-passes",
            property: "priceInformation",
        },
    ),
    (
        "PriceInformation",
        Source::Property {
            schema_of: "/developer-products/v2/universes/{universeId}/developer-products",
            property: "priceInformation",
        },
    ),
];

fn resolve<'a>(spec: &'a Value, schema: &'a Value) -> &'a Value {
    match schema["$ref"].as_str() {
        Some(reference) => {
            let name = reference.rsplit('/').next().unwrap_or_default();
            resolve(spec, &spec["components"]["schemas"][name])
        }
        None => schema,
    }
}

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

/// The JSON schema of a route's response, or `Null` when it documents none.
fn response_schema<'a>(spec: &'a Value, path: &str, method: &str, status: &str) -> &'a Value {
    &spec["paths"][path][method]["responses"][status]["content"]["application/json"]["schema"]
}

fn schema_for<'a>(spec: &'a Value, source: &Source) -> &'a Value {
    match source {
        Source::Route {
            path,
            method,
            status,
        } => response_schema(spec, path, method, status),
        Source::Property {
            schema_of,
            property,
        } => {
            let parent = resolve(spec, response_schema(spec, schema_of, "post", "200"));
            &parent["properties"][*property]
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
fn every_shop_model_reads_only_what_the_spec_documents() {
    let root = repo_root();
    let path = root.join("spec/openapi.json");
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let spec: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
    let src = fs::read_to_string(root.join(MODELS))
        .unwrap_or_else(|e| panic!("cannot read {MODELS}: {e}"));
    let src = strip_tests(&src);

    let mut report = Vec::new();
    for (model, source) in RESPONSES {
        let keys = serialised_keys(&src, model);
        assert!(
            !keys.is_empty(),
            "read no key from {model} in {MODELS}: the scan found the struct and nothing in it, \
             so it would be comparing empty sets."
        );
        let documented = property_names(&spec, schema_for(&spec, source));
        assert!(
            !documented.is_empty(),
            "the spec documents no property where {model} expects its schema. An unresolvable \
             schema must not read as an empty one: if Roblox moved the route, update it here."
        );
        for key in unknown_keys(&keys, &documented) {
            report.push(format!(
                "{model} reads `{key}`, which the spec does not document here. Known: {documented:?}"
            ));
        }
    }

    assert!(
        report.is_empty(),
        "\n\nThe shop models disagree with the vendored spec. These fields read as None, not as \
         an error:\n\n  {}\n\n",
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
}
