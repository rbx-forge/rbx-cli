//! The schemas must accept every config file this repository ships.
//!
//! This is the test that guards the one failure mode worth fearing. A schema
//! *stricter* than the tool is worse than no schema: an editor painting a
//! valid file red teaches people to stop reading the squiggles, and then the
//! real errors go unread too. Unit tests in `schemas.rs` check the shape of
//! the schema; this one runs a real validator over real documents.
//!
//! The corpus is the repository's own `.example` configs plus the samples in
//! `docs/`, which are the exact files a newcomer copies.

use std::path::{Path, PathBuf};

use boon::{Compiler, Schemas};

/// Repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels up from this crate")
}

/// Validate `document` against the schema for `config_file`, panicking with
/// the validator's own explanation on failure.
fn assert_valid(config_file: &str, source: &str, document: &str) {
    let schema_file = repo_root().join("schemas").join(format!(
        "{}.schema.json",
        config_file.trim_end_matches(".toml")
    ));
    let schema_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&schema_file)
            .unwrap_or_else(|e| panic!("reading {}: {e}", schema_file.display())),
    )
    .expect("the committed schema is valid JSON");

    // TOML in, JSON out: a JSON Schema validates the JSON projection of the
    // document, which is what taplo feeds it too.
    let toml_value: toml::Value =
        toml::from_str(document).unwrap_or_else(|e| panic!("{source} is not valid TOML: {e}"));
    let instance: serde_json::Value =
        serde_json::to_value(&toml_value).expect("TOML maps onto JSON");

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    let url = format!("mem://{config_file}");
    compiler
        .add_resource(&url, schema_json)
        .expect("the schema is a usable resource");
    let key = compiler
        .compile(&url, &mut schemas)
        .unwrap_or_else(|e| panic!("compiling {config_file}'s schema: {e}"));

    if let Err(error) = schemas.validate(&instance, key) {
        panic!(
            "{source} is a valid config the tools accept, but {config_file}'s \
             schema rejects it.\nA schema stricter than its tool is worse than \
             no schema.\n\n{error:#}"
        );
    }
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

#[test]
fn the_committed_place_configs_validate() {
    for file in [
        "testenv/rbxplace.example.toml",
        "prodread/rbxplace.example.toml",
    ] {
        assert_valid("rbxplace.toml", file, &read(file));
    }
}

#[test]
fn the_committed_apikey_configs_validate() {
    for file in [
        "testenv/rbxapikey.example.toml",
        "prodread/rbxapikey.example.toml",
    ] {
        assert_valid("rbxapikey.toml", file, &read(file));
    }
}

/// Every documented env key at once, including the ones no committed file
/// happens to use. The `.example` files are a thin corpus: between them they
/// exercise `universe_id` and `places` and nothing else.
#[test]
fn a_place_config_using_every_documented_key_validates() {
    assert_valid(
        "rbxplace.toml",
        "the full rbxplace.toml surface",
        r#"
[owner]
type = "group"
id = 1234567

[codegen]
output = "src/shared/Envs.luau"

[dev]
universe_id = 100
confirm = false
env = "Development"
codegen = true
[dev.places]
main = 1001
lobby = 1002

[prod]
universe_id = 200
confirm = true
owner = { type = "user", id = 42 }
[prod.places]
main = 2001
"#,
    );
}

/// The documented leniency, as a document rather than as a shape assertion:
/// an unrecognised key is warned about on stderr and ignored, never rejected
/// (`docs/env.md`). The schema has to agree, or the editor contradicts the
/// tool on a file that loads fine.
#[test]
fn an_unknown_key_is_accepted_the_way_the_tools_accept_it() {
    assert_valid(
        "rbxplace.toml",
        "an rbxplace.toml with a key from a newer release",
        r#"
[dev]
universe_id = 100
some_key_from_a_newer_release = "x"
"#,
    );

    assert_valid(
        "rbxapikey.toml",
        "an rbxapikey.toml with an unknown settings key",
        r#"
[settings]
default_envs = ["prod"]
setting_from_the_future = true

[keys.viewer]
scopes = ["universe:read"]
"#,
    );
}

#[test]
fn a_meta_config_with_a_per_env_overlay_validates() {
    assert_valid(
        "rbxmeta.toml",
        "an rbxmeta.toml with overlays",
        r#"
[experience]
universe_id = 100
place_id = 1001

[game]
name = "My Game"
description = "A game."
server_size = 50
voice_chat = false
allow_copying = false
visibility = "public"
server_fill = { mode = "custom", reserved_slots = 5 }

[game.devices]
computer = true
phone = true

[game.social_links.discord]
title = "Join our Discord"
url = "https://discord.gg/example"

[envs.dev]
name = "My Game (dev)"
visibility = "private"

[envs.dev.media]
icon = "art/dev-icon.png"
"#,
    );
}

#[test]
fn a_config_file_with_entries_validates() {
    assert_valid(
        "rbxconfig.toml",
        "an rbxconfig.toml with scalar and table values",
        r#"
[dev.entries."features.new_xp_popup"]
value = true
description = "Testing new popup"

[dev.entries."balance.speed_multipliers"]
value = { tier_1 = 1.5, tier_2 = 2.0 }

[prod.entries."features.new_xp_popup"]
value = false
"#,
    );
}

/// One shop config exercising all three resource kinds, the tables around
/// them, and a per-env overlay: the shape `docs/shop.md` documents.
#[test]
fn a_shop_config_with_all_three_resource_kinds_validates() {
    assert_valid(
        "rbxshop.toml",
        "an rbxshop.toml using passes, badges, products and an env overlay",
        r#"
[experience]
universe_id = 123456789

[creator]
type = "group"
id = 987

[codegen]
output = "src/shared/Shop"
typescript = true
style = "nested"

[codegen.extra]
"legacy.old_pass" = 111

[icons]
bleed = true
dir = "art/icons"

[gifts]
label = "[GIFT] "
key_prefix = "Gift"

[passes.VIP]
price = 499
description = "Everything, forever."
icon = "art/icons/vip.png"
create_gift = true

[badges.Welcome]
name = "Welcome!"
enabled = true

[products.Coins100]
price = 100
store_page = true

[envs.dev.passes.VIP]
price = 1

[envs.dev.products.Coins100]
for_sale = false
"#,
    );
}

/// The other half of the contract: a schema that accepts everything is not
/// validating. A wrong type on a documented key has to be caught, or the
/// feature buys nothing.
#[test]
fn a_wrong_type_on_a_known_key_is_rejected() {
    let schema_json: serde_json::Value =
        serde_json::from_str(&read("schemas/rbxplace.schema.json")).expect("valid JSON");
    let toml_value: toml::Value = toml::from_str("[dev]\nuniverse_id = \"not a number\"\n")
        .expect("valid TOML, invalid config");
    let instance: serde_json::Value = serde_json::to_value(&toml_value).expect("maps onto JSON");

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler
        .add_resource("mem://rbxplace.toml", schema_json)
        .expect("usable resource");
    let key = compiler
        .compile("mem://rbxplace.toml", &mut schemas)
        .expect("compiles");

    assert!(
        schemas.validate(&instance, key).is_err(),
        "a string universe_id should be flagged: the tools reject it too, and \
         catching it in the editor is the point of the whole feature"
    );
}

// ── rbxavatar.toml ──

/// The document `game.engine_avatar_settings` points at, in the TOML form.
///
/// This is the shape a developer actually writes: a few groups, a few keys
/// each, nowhere near the full surface. A schema that only accepted the
/// complete document would be useless for the file people have.
#[test]
fn a_partial_avatar_document_validates() {
    assert_valid(
        "rbxavatar.toml",
        "a hand-written avatar settings file",
        r#"
version = 1

[AvatarRules]
AvatarType = 1

[AvatarCollisionRules]
CollisionMode = 1
HitAndTouchDetectionMode = 0
SingleColliderSize = [2, 3, 1]

[AvatarBodyRules]
ScaleMode = 1
CustomHeight = [5.5, 5.5]
KeepPlayerHead = true

[AvatarAnimationRules]
AnimationClipsMode = 0
CustomWalkAnimationEnabled = true
CustomWalkAnimationId = 123456789
"#,
    );
}

/// The rule that matters most for this file. `rbx meta` sends the document
/// through without inspecting it, so a key Roblox adds tomorrow already works
/// today, and a schema that painted it red would be lying about the tool.
#[test]
fn an_avatar_key_the_schema_does_not_know_is_still_accepted() {
    assert_valid(
        "rbxavatar.toml",
        "an avatar file using a key this schema has never heard of",
        r#"
[AvatarRules]
AvatarType = 1
SomethingRobloxAddedLastTuesday = 7

[RulesInventedAfterThisSchemaWasWritten]
Whatever = true
"#,
    );
}

/// The other half: a schema that accepts everything is not validating. A wrong
/// type on a key it does know has to be caught.
#[test]
fn a_wrong_type_in_the_avatar_document_is_rejected() {
    let schema_json: serde_json::Value =
        serde_json::from_str(&read("schemas/rbxavatar.schema.json")).expect("valid JSON");
    let toml_value: toml::Value =
        toml::from_str("[AvatarRules]\nAvatarType = \"R15\"\n").expect("valid TOML");
    let instance: serde_json::Value = serde_json::to_value(&toml_value).expect("maps onto JSON");

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler
        .add_resource("mem://rbxavatar.toml", schema_json)
        .expect("usable resource");
    let key = compiler
        .compile("mem://rbxavatar.toml", &mut schemas)
        .expect("compiles");

    assert!(
        schemas.validate(&instance, key).is_err(),
        "AvatarType is an integer; a string should be flagged in the editor"
    );
}

/// The three-number vectors are fixed length, and getting one wrong is the
/// kind of mistake that reaches Roblox silently otherwise.
#[test]
fn a_collider_size_of_the_wrong_length_is_rejected() {
    let schema_json: serde_json::Value =
        serde_json::from_str(&read("schemas/rbxavatar.schema.json")).expect("valid JSON");
    let toml_value: toml::Value =
        toml::from_str("[AvatarCollisionRules]\nSingleColliderSize = [2, 3]\n")
            .expect("valid TOML");
    let instance: serde_json::Value = serde_json::to_value(&toml_value).expect("maps onto JSON");

    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler
        .add_resource("mem://rbxavatar.toml", schema_json)
        .expect("usable resource");
    let key = compiler
        .compile("mem://rbxavatar.toml", &mut schemas)
        .expect("compiles");

    assert!(
        schemas.validate(&instance, key).is_err(),
        "SingleColliderSize is three numbers; two should be flagged"
    );
}
