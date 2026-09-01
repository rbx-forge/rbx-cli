//! Guards on the extractor itself.
//!
//! Every one of these exists because the scanner in [`crate::extract`] once got
//! it wrong and the whole file went quietly green on almost nothing.

use std::fs;

use serde_json::Value;

use crate::{extract::*, spec::*};
/// The bug this whole file's coverage depended on for months: a
/// `#[cfg(test)]` helper near the top of a client used to hide every call
/// below it.
#[test]
fn a_cfg_test_item_hides_itself_and_nothing_after_it() {
    let src = r#"
impl Client {
    #[cfg(test)]
    fn with_base_url(mut self, url: String) -> Self {
        self.base = ApiBase::new("https://mock.example/v1/nope");
        self
    }

    fn upload(&self) -> String {
        self.base.join("/universes/v1/places")
    }
}
"#;
    let stripped = strip_tests(src);
    assert!(
        !stripped.contains("mock.example"),
        "the gated item must be blanked out"
    );
    assert!(
        stripped.contains("/universes/v1/places"),
        "code after the gated item must survive: {stripped}"
    );
    assert_eq!(
        stripped.lines().count(),
        src.lines().count(),
        "line numbers are reported to the reader, so they must not shift"
    );
}

/// The `mod tests { ... }` at the bottom of a file, which is what the old
/// truncation handled correctly and this must keep handling.
#[test]
fn a_trailing_test_module_is_still_removed_whole() {
    let src = r#"
fn call() -> String {
    base.join("/cloud/v2/universes/{}/places")
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixture() {
        assert_eq!(url(), "https://apis.roblox.com/cloud/v2/universes/1");
    }
}
"#;
    let stripped = strip_tests(src);
    assert!(stripped.contains("/cloud/v2/universes/{}/places"));
    assert!(
        !stripped.contains("universes/1"),
        "a unit test's fixed id is not an endpoint template: {stripped}"
    );
}

/// A brace inside a string or a comment is not a brace, and `'a` is not a
/// char literal. Getting either wrong swallows the rest of the file, which is
/// the failure this test exists to make loud.
#[test]
fn the_item_scanner_is_not_fooled_by_braces_in_strings_or_lifetimes() {
    let src = r#"
#[cfg(test)]
fn gated<'a>(name: &'a str) -> String {
    // } this brace is a comment
    format!("{{literal braces}} {name}")
}

fn real() -> String {
    base.join("/assets/v1/assets/{}/versions")
}
"#;
    let stripped = strip_tests(src);
    assert!(
        stripped.contains("/assets/v1/assets/{}/versions"),
        "the scanner stopped in the wrong place: {stripped}"
    );
    assert!(!stripped.contains("literal braces"));
}

/// Several files explain in prose why an item is `#[cfg(test)]`. Blanking
/// from a doc comment would delete the code it documents.
#[test]
fn a_mention_in_a_doc_comment_is_not_an_attribute() {
    let src = r#"
/// `#[cfg(test)]` rather than `#[doc(hidden)] pub`, because the module is
/// private.
fn call() -> String {
    base.join("/games/v1/games")
}
"#;
    assert!(strip_tests(src).contains("/games/v1/games"));
}

/// `http_crates` reads manifests, so it fails silently if the parse is wrong.
#[test]
fn the_manifest_scan_sees_dependencies_and_ignores_dev_dependencies() {
    let root = repo_root();
    let crates = http_crates(&root);

    assert!(
        crates.contains("rbx-place"),
        "rbx-place depends on reqwest: {crates:?}"
    );
    assert!(
        !crates.contains("rbx-spec-drift"),
        "this crate has no reqwest dependency at all and must not be counted: {crates:?}"
    );
}

/// The day this test goes red is the day `schemas/rbxavatar.schema.json` can
/// stop being maintained by hand.
///
/// That schema is the one file in `schemas/` with no freshness guarantee: it
/// describes the inside of `engineAvatarSettings`, and every other schema there
/// is regenerated from the serde model the CLI parses with, so CI fails when
/// one goes stale. This one has no model to regenerate from, because Roblox
/// types the field as an opaque string and publishes nothing about its
/// contents.
///
/// So this asserts the constraint still holds rather than the schema still
/// matches: the only checkable form of the question. If Roblox ever replaces
/// `"type": "string"` with a real object schema, this fails and says so, and
/// the hand-written file becomes a derived one like the rest.
///
/// It is deliberately not a test of *our* schema. Comparing our key names
/// against a document that describes none of them is not possible, and a test
/// that pretended otherwise would be worse than this one.
#[test]
fn engine_avatar_settings_is_still_an_opaque_string() {
    let spec: Value = serde_json::from_str(
        &fs::read_to_string(repo_root().join("spec/openapi.json")).expect("the vendored spec"),
    )
    .expect("the vendored spec is valid JSON");

    let field = &spec["components"]["schemas"]
        ["Roblox.Api.Develop.Models.UniverseSettingsRequestV2"]["properties"]
        ["engineAvatarSettings"];

    assert!(
        !field.is_null(),
        "engineAvatarSettings has left UniverseSettingsRequestV2. Either Roblox \
         removed the field it warned it might remove, or it moved. Check what \
         `game.engine_avatar_settings` should now send."
    );

    assert_eq!(
        field["type"], "string",
        "engineAvatarSettings is no longer an opaque JSON string in the Roblox \
         spec, which is the whole reason schemas/rbxavatar.schema.json is \
         hand-written and unverified.\n\nIf Roblox now documents its contents, \
         derive that schema from the spec instead and delete \
         crates/rbx-schema/src/engine_avatar.rs.\n\nField as vendored: {field}"
    );
}
