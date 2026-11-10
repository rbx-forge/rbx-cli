//! Request bodies: what we serialise against what the spec says the endpoint
//! accepts.

use std::fs;

use serde_json::Value;

use crate::configs::CONFIGS_CLIENT;
use crate::{extract::*, spec::*};
/// Request bodies this workspace sends, as `(source file, Rust struct, spec
/// schema, spec properties deliberately not sent)`.
///
/// Both directions are checked, and the second one is the one that was
/// missing. A key we send that the schema does not define is a rejected
/// request: these schemas carry `additionalProperties: false`, asserted below,
/// so a misspelled field is a 4xx rather than a field Roblox ignores. And a
/// property the schema defines that we neither send nor list here is the bug
/// that actually shipped: `UpdateDraftRequest.conditionalRules` was absent
/// from `OverwriteBody`, and on `PUT draft:overwrite` an absent
/// `conditionalRules` means "delete every published conditional rule".
///
/// So an entry in the fourth field is a deliberate statement that the property
/// is safe to omit, and the reason belongs in the doc comment of the struct
/// that omits it. An empty list means "we send all of it", which is the
/// strongest position and where both of these are.
const REQUEST_BODIES: &[(&str, &str, &str, &[&str])] = &[
    (CONFIGS_CLIENT, "OverwriteBody", "UpdateDraftRequest", &[]),
    (CONFIGS_CLIENT, "PublishBody", "PublishDraftRequest", &[]),
];

/// The JSON keys a `#[derive(Serialize)]` struct puts on the wire, read from
/// the source text.
///
/// Text rather than reflection because this crate compiles against nothing in
/// the workspace on purpose (see the module docs), so the alternative is no
/// check at all. It models what the structs here use: a bare field name,
/// `#[serde(rename = "...")]`, `#[serde(rename_all = "camelCase")]` and
/// `#[serde(skip...)]`. It refuses anything else it is shown, because another
/// `rename_all` or a `flatten` changes the whole key set, and a scanner that
/// shrugged at one would compare a key set nobody sends.
pub(crate) fn serialised_keys(src: &str, struct_name: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let declaration = format!("struct {struct_name}");
    let at = lines
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            (trimmed.starts_with("struct ") || trimmed.starts_with("pub struct "))
                && trimmed
                    .split([' ', '{', '<', '('])
                    .any(|word| word == struct_name)
                && trimmed.contains(&declaration)
        })
        .unwrap_or_else(|| panic!("no `{declaration}` in the source scanned for it"));

    let mut camel_case = false;
    for line in lines[..at].iter().rev() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("#[") {
            break;
        }
        if trimmed.contains("rename_all") {
            assert!(
                trimmed.contains(r#"rename_all = "camelCase""#),
                "{struct_name} carries {trimmed}: this scan knows `camelCase` and no other \
                 `rename_all`, so it would report a key set that is not the one on the wire."
            );
            camel_case = true;
        }
    }

    let mut keys = Vec::new();
    let mut rename: Option<String> = None;
    let mut skip = false;
    for line in &lines[at + 1..] {
        let trimmed = line.trim();
        if trimmed.starts_with('}') {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("#[") {
            assert!(
                !trimmed.contains("flatten"),
                "{struct_name} flattens a field, which lifts another struct's keys into \
                 this one: {trimmed}. Teach this scan to follow it before trusting the result."
            );
            if let Some((_, after)) = trimmed.split_once("rename = ") {
                rename = string_literals(after).into_iter().next();
            }
            // `skip`, `skip_serializing` and `skip_deserializing` take the field
            // off the wire. `skip_serializing_if` only omits an empty value, so
            // the options are read one by one rather than matched as text.
            skip |= trimmed
                .trim_start_matches("#[serde(")
                .trim_end_matches(")]")
                .split(',')
                .map(str::trim)
                .any(|option| matches!(option, "skip" | "skip_serializing" | "skip_deserializing"));
            continue;
        }
        let Some((name, _)) = trimmed.split_once(':') else {
            continue;
        };
        if std::mem::take(&mut skip) {
            rename = None;
            continue;
        }
        // Response models have public fields, and the request bodies do not.
        let field = name
            .trim()
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ");
        let key = match rename.take() {
            Some(explicit) => explicit,
            None if camel_case => to_camel_case(field),
            None => field.to_string(),
        };
        keys.push(key);
    }
    keys
}

/// What `rename_all = "camelCase"` makes of a snake_case field name.
pub(crate) fn to_camel_case(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut upper = false;
    for c in field.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// What the tool sends and what the endpoint accepts, compared against the
/// document rather than against a mock the tool configures itself.
///
/// The regression that shipped was pinned only by a wiremock body matcher, so
/// renaming the serde attribute and the test literal together left the suite
/// green. This is the half that cannot be renamed into agreement.
#[test]
fn every_request_body_matches_the_schema_the_spec_documents() {
    let root = repo_root();
    let spec = load_document(&root);

    for (file, struct_name, schema_name, omitted) in REQUEST_BODIES {
        // The configs client lives in rbx-core, wherever the manifest takes it from.
        let source = if *file == CONFIGS_CLIENT {
            crate::configs::rbx_core_root(&root).join(file)
        } else {
            root.join(file)
        };
        let src = fs::read_to_string(&source)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", source.display()));
        let keys = serialised_keys(&strip_tests(&src), struct_name);
        assert!(
            !keys.is_empty(),
            "read no serialised key from {struct_name} in {file}: the scan found the struct \
             and nothing in it, so it is comparing empty sets."
        );

        let schema = &spec["components"]["schemas"][schema_name];
        let properties = schema["properties"].as_object().unwrap_or_else(|| {
            panic!(
                "components.schemas.{schema_name}.properties is not an object in the vendored \
                 spec. If Roblox renamed the schema, rename it here: an unresolvable schema \
                 must not read as an empty one."
            )
        });
        assert!(
            !properties.is_empty(),
            "components.schemas.{schema_name} documents no property, so this check would pass \
             on anything."
        );
        assert_eq!(
            schema["additionalProperties"],
            Value::Bool(false),
            "components.schemas.{schema_name} no longer refuses unknown properties. The \
             comparison below is written on the premise that it does: a key this document \
             does not define is a rejected request, not a field Roblox ignores."
        );

        let unknown: Vec<&String> = keys
            .iter()
            .filter(|key| !properties.contains_key(key.as_str()))
            .collect();
        assert!(
            unknown.is_empty(),
            "{struct_name} ({file}) sends {unknown:?}, which \
             components.schemas.{schema_name} does not define. With \
             `additionalProperties: false` that is a 4xx on every call, not an ignored \
             field. Known properties: {:?}",
            properties.keys().collect::<Vec<_>>()
        );

        let unsent: Vec<&String> = properties
            .keys()
            .filter(|property| !keys.contains(property))
            .filter(|property| !omitted.contains(&property.as_str()))
            .collect();
        assert!(
            unsent.is_empty(),
            "components.schemas.{schema_name} documents {unsent:?}, which {struct_name} \
             ({file}) neither sends nor declares as deliberately omitted. Read what the \
             property does when absent before adding it to the omission list in \
             REQUEST_BODIES: `conditionalRules` on `draft:overwrite` reads as \
             \"delete every published conditional rule\", and that is how this became a \
             test."
        );
    }
}
