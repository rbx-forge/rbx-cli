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
/// check at all. It models the two things the request bodies here use, a bare
/// field name and `#[serde(rename = "...")]`, and refuses anything else it is
/// shown: a `rename_all` or a `flatten` changes the whole key set, and a
/// scanner that shrugged at one would compare a key set nobody sends.
fn serialised_keys(src: &str, struct_name: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let declaration = format!("struct {struct_name}");
    let at = lines
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            (trimmed.starts_with("struct ") || trimmed.starts_with("pub struct "))
                && trimmed.contains(&declaration)
        })
        .unwrap_or_else(|| panic!("no `{declaration}` in the source scanned for it"));

    for line in lines[..at].iter().rev() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("#[") {
            break;
        }
        assert!(
            !trimmed.contains("rename_all"),
            "{struct_name} carries {trimmed}, which renames every field: this scan reads \
             field names and `rename` attributes only, so it would report a key set that \
             is not the one being sent."
        );
    }

    let mut keys = Vec::new();
    let mut rename: Option<String> = None;
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
            continue;
        }
        let Some((name, _)) = trimmed.split_once(':') else {
            continue;
        };
        keys.push(rename.take().unwrap_or_else(|| name.trim().to_string()));
    }
    keys
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
    let path = root.join("spec/openapi.json");
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let spec: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));

    for (file, struct_name, schema_name, omitted) in REQUEST_BODIES {
        let source = root.join(file);
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
