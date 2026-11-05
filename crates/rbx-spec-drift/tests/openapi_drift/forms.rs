//! The fields `rbx shop` writes, against what the spec says each route accepts.
//!
//! These are the writes that create things Roblox will not let anyone delete,
//! and a path check says nothing about them. They go out as multipart forms
//! built by hand, so there is no struct for `bodies` to read: a renamed field
//! compiles, passes every mock, and fails on a real account.
//!
//! Absent properties are not reported, unlike in `bodies`. Every form here is a
//! create with optional fields or a PATCH, where absent means "leave it alone".

use std::fs;

use serde_json::Value;

use crate::{extract::*, spec::*};

struct Form {
    file: &'static str,
    function: &'static str,
    route: &'static str,
    method: &'static str,
    /// Fields sent knowing the spec marks them deprecated. An entry is a
    /// statement that nothing replaces them yet, and it has to stay true: one
    /// that is no longer sent or no longer deprecated fails too.
    deprecated_on_purpose: &'static [&'static str],
}

/// `isRegionalPricingEnabled` is only sent for a config that asks for
/// `regional_pricing` by itself: see `Pricing` in `rbx-shop`.
const REGIONAL: &str = "isRegionalPricingEnabled";

/// What `Pricing::apply` can add to a form. Read from the source, so renaming
/// one there cannot leave this check comparing a name nobody sends.
const PRICING_FIELDS: &[&str] = &[REGIONAL, "isManagedPricingEnabled"];
const PRICING_SOURCE: &str = "crates/rbx-shop/src/api/mod.rs";

/// `POST /legacy-publish/v1/badges/{badgeId}/icon` is not listed: the spec
/// documents no request body for it, so there is nothing to compare against.
const FORMS: &[Form] = &[
    Form {
        file: "crates/rbx-shop/src/api/passes.rs",
        function: "create_game_pass",
        route: "/game-passes/v1/universes/{universeId}/game-passes",
        method: "post",
        deprecated_on_purpose: &[REGIONAL],
    },
    Form {
        file: "crates/rbx-shop/src/api/passes.rs",
        function: "update_game_pass",
        route: "/game-passes/v1/universes/{universeId}/game-passes/{gamePassId}",
        method: "patch",
        deprecated_on_purpose: &[REGIONAL],
    },
    Form {
        file: "crates/rbx-shop/src/api/products.rs",
        function: "create_developer_product",
        route: "/developer-products/v2/universes/{universeId}/developer-products",
        method: "post",
        deprecated_on_purpose: &[REGIONAL],
    },
    Form {
        file: "crates/rbx-shop/src/api/products.rs",
        function: "update_developer_product",
        route: "/developer-products/v2/universes/{universeId}/developer-products/{productId}",
        method: "patch",
        // The only way to set `store_page`, and the first step of taking a
        // product off sale, which Roblox refuses while it is on the store page.
        deprecated_on_purpose: &[REGIONAL, "storePageEnabled"],
    },
    Form {
        file: "crates/rbx-shop/src/api/badges.rs",
        function: "create_badge",
        route: "/legacy-badges/v1/universes/{universeId}/badges",
        method: "post",
        deprecated_on_purpose: &[],
    },
    Form {
        file: "crates/rbx-shop/src/api/badges.rs",
        function: "update_badge",
        route: "/legacy-badges/v1/badges/{badgeId}",
        method: "patch",
        deprecated_on_purpose: &[],
    },
];

/// The body of one `pub async fn`, up to the next.
fn function_body<'a>(src: &'a str, function: &str) -> &'a str {
    let start = src
        .find(&format!("pub async fn {function}("))
        .unwrap_or_else(|| panic!("no `pub async fn {function}` in the source scanned for it"));
    let rest = &src[start + 1..];
    let end = rest.find("pub async fn ").unwrap_or(rest.len());
    &rest[..end]
}

/// Every field name a function body puts on the wire.
///
/// Text rather than reflection for the reason `bodies` gives. It reads
/// `.text("x"`, `.part("x"` and the keys of a `json!({`, and refuses a call whose
/// name is not a literal on the same line, because a field it cannot read is a
/// field it would silently stop checking.
fn sent_fields(body: &str) -> Vec<String> {
    let mut fields = Vec::new();
    for method in [".text(", ".part("] {
        for (at, _) in body.match_indices(method) {
            let after = &body[at + method.len()..];
            // `response.text()` reads the reply: it is no field.
            if after.starts_with(')') {
                continue;
            }
            let Some(literal) = after.strip_prefix('"') else {
                panic!(
                    "`{method}` with a name that is not a literal on the same line: {}. Teach \
                     this scan to read it before trusting the result.",
                    after.lines().next().unwrap_or_default()
                );
            };
            let name = literal.split('"').next().unwrap_or_default();
            fields.push(name.to_string());
        }
    }
    if let Some(at) = body.find("json!({") {
        let object = &body[at..];
        let end = object.find("})").unwrap_or(object.len());
        for line in object[..end].lines().skip(1) {
            if let Some((key, _)) = line.trim().split_once("\":") {
                fields.push(key.trim_start_matches('"').to_string());
            }
        }
    }
    if body.contains("pricing.apply(") {
        fields.extend(PRICING_FIELDS.iter().map(|f| f.to_string()));
    }
    fields.sort();
    fields.dedup();
    fields
}

fn resolve<'a>(spec: &'a Value, schema: &'a Value) -> &'a Value {
    match schema["$ref"].as_str() {
        Some(reference) => {
            let name = reference.rsplit('/').next().unwrap_or_default();
            resolve(spec, &spec["components"]["schemas"][name])
        }
        None => schema,
    }
}

/// What is wrong with sending `fields` to `operation`, as readable sentences.
fn problems(
    spec: &Value,
    operation: &Value,
    fields: &[String],
    deprecated_on_purpose: &[&str],
) -> Vec<String> {
    let Some(content) = operation["requestBody"]["content"].as_object() else {
        return vec!["the spec documents no request body for this route any more".to_string()];
    };
    let Some(media) = content.values().next() else {
        return vec!["the request body documents no content type".to_string()];
    };
    let schema = resolve(spec, &media["schema"]);
    let Some(properties) = schema["properties"].as_object() else {
        return vec!["the request body documents no properties".to_string()];
    };

    let is_deprecated = |name: &str| {
        let property = &properties[name];
        property["deprecated"] == Value::Bool(true)
            || resolve(spec, property)["deprecated"] == Value::Bool(true)
    };

    let mut found = Vec::new();
    for field in fields {
        if !properties.contains_key(field) {
            found.push(format!(
                "sends `{field}`, which the route does not define. Known: {:?}",
                properties.keys().collect::<Vec<_>>()
            ));
        } else if is_deprecated(field) && !deprecated_on_purpose.contains(&field.as_str()) {
            found.push(format!(
                "sends `{field}`, which the spec now marks deprecated. Read what replaces it, \
                 or declare it in `deprecated_on_purpose` with the reason."
            ));
        }
    }
    for declared in deprecated_on_purpose {
        if !fields.iter().any(|f| f == declared) {
            found.push(format!(
                "declares `{declared}` as deprecated on purpose, and no longer sends it"
            ));
        } else if properties.contains_key(*declared) && !is_deprecated(declared) {
            found.push(format!(
                "declares `{declared}` as deprecated on purpose, and the spec no longer marks \
                 it deprecated. Remove the entry."
            ));
        }
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let required = required.as_str().unwrap_or_default();
        if !fields.iter().any(|f| f == required) {
            found.push(format!(
                "does not send `{required}`, which the route requires"
            ));
        }
    }
    found
}

#[test]
fn every_shop_write_sends_only_what_the_spec_documents() {
    let root = repo_root();
    let path = root.join("spec/openapi.json");
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let spec: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));

    let pricing = fs::read_to_string(root.join(PRICING_SOURCE))
        .unwrap_or_else(|e| panic!("cannot read {PRICING_SOURCE}: {e}"));
    for field in PRICING_FIELDS {
        assert!(
            pricing.contains(&format!("\"{field}\"")),
            "{PRICING_SOURCE} no longer sends `{field}`, so PRICING_FIELDS is checking a name \
             nobody uses."
        );
    }

    let mut report = Vec::new();
    for form in FORMS {
        let source = root.join(form.file);
        let src = fs::read_to_string(&source)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", source.display()));
        let src = strip_tests(&src);
        let fields = sent_fields(function_body(&src, form.function));
        assert!(
            !fields.is_empty(),
            "read no field from {} in {}: the scan found the function and nothing in it, so it \
             would be comparing empty sets.",
            form.function,
            form.file
        );

        let operation = &spec["paths"][form.route][form.method];
        assert!(
            operation.is_object(),
            "{} {} is not in the spec. `every_endpoint_we_call_still_exists_in_the_roblox_spec` \
             says whether it moved; update the route here to match.",
            form.method,
            form.route
        );
        for problem in problems(&spec, operation, &fields, form.deprecated_on_purpose) {
            report.push(format!(
                "{} ({}, {} {}) {problem}",
                form.function, form.file, form.method, form.route
            ));
        }
    }

    assert!(
        report.is_empty(),
        "\n\nThe shop writes disagree with the vendored spec:\n\n  {}\n\n",
        report.join("\n  ")
    );
}

mod guards {
    use super::*;
    use serde_json::json;

    fn operation() -> Value {
        json!({ "requestBody": { "content": { "multipart/form-data": { "schema": {
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "old": { "type": "boolean", "deprecated": true },
            }
        }}}}})
    }

    fn fields(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn check(names: &[&str], deprecated_on_purpose: &[&str]) -> Vec<String> {
        problems(
            &json!({}),
            &operation(),
            &fields(names),
            deprecated_on_purpose,
        )
    }

    #[test]
    fn a_form_the_route_documents_has_no_problem() {
        assert!(check(&["name"], &[]).is_empty());
        assert!(check(&["name", "old"], &["old"]).is_empty());
    }

    #[test]
    fn a_renamed_field_is_reported() {
        let found = check(&["name", "nmae"], &[]);
        assert!(found[0].contains("`nmae`"), "{found:?}");
    }

    #[test]
    fn a_field_going_deprecated_is_reported_unless_declared() {
        let found = check(&["name", "old"], &[]);
        assert!(found[0].contains("deprecated"), "{found:?}");
    }

    #[test]
    fn a_declaration_that_stopped_being_true_is_reported() {
        let unsent = check(&["name"], &["old"]);
        assert!(unsent[0].contains("no longer sends"), "{unsent:?}");
        let revived = check(&["name"], &["name"]);
        assert!(revived[0].contains("no longer marks"), "{revived:?}");
    }

    #[test]
    fn a_missing_required_field_is_reported() {
        let found = check(&[], &[]);
        assert!(found[0].contains("requires"), "{found:?}");
    }

    #[test]
    fn a_route_losing_its_body_is_reported_rather_than_passing() {
        let found = problems(&json!({}), &json!({}), &fields(&["name"]), &[]);
        assert!(found[0].contains("no request body"), "{found:?}");
    }

    #[test]
    fn the_scan_reads_all_three_shapes() {
        let body = r#"
            let form = multipart::Form::new().text("name", x).part("files", y);
            let body = serde_json::json!({
                "enabled": on,
            });
        "#;
        assert_eq!(sent_fields(body), ["enabled", "files", "name"]);
    }

    #[test]
    #[should_panic(expected = "not a literal")]
    fn the_scan_refuses_a_name_it_cannot_read() {
        sent_fields("form.text(field_name, value)");
    }
}
