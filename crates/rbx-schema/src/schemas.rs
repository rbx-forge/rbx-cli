//! The registry: one entry per config file the CLI reads.
//!
//! Adding a config file means adding a line to [`all`] and a `JsonSchema`
//! derive on its root model. Nothing else here is per-file.

use schemars::gen::SchemaSettings;
use schemars::schema::{RootSchema, Schema as SchemaNode};
use schemars::JsonSchema;

/// One generated schema, and the config file it describes.
#[derive(Debug)]
pub struct Schema {
    /// The TOML file this validates, e.g. `rbxplace.toml`. Used in the
    /// schema's own `title`/`description` and in error messages.
    pub config_file: &'static str,
    /// Output file name under `schemas/`.
    pub file_name: &'static str,
    pub body: RootSchema,
}

/// Draft 7, not the 2020-12 default.
///
/// taplo and VS Code's Even Better TOML (the two consumers this exists for)
/// are built on draft 7 validators. Emitting 2020-12 makes them fall back to a
/// best-effort reading of a document they do not fully understand, which is a
/// silent partial validation rather than an error, and the worst of both.
fn settings() -> SchemaSettings {
    SchemaSettings::draft07()
}

fn build<T: JsonSchema>(
    config_file: &'static str,
    file_name: &'static str,
    description: &'static str,
) -> Schema {
    finish::<T>(config_file, file_name, description, None)
}

/// Like [`build`], but for a file whose top level is `#[serde(flatten)]`ed map
/// of `V`: every table the named fields do not claim is a `V`.
///
/// This needs saying explicitly because **schemars emits nothing at all for a
/// flattened map**. `rbxplace.toml` and `rbxconfig.toml` are almost entirely
/// that shape, so without this their schemas validated the two or three named
/// keys and silently accepted anything else: no completion inside `[dev]`, no
/// error on `universe_id = "123"`, which is the validation people install this
/// for. The generated schema looked plausible, which is the bad kind of wrong.
///
/// One line per file, next to the file name, and
/// `the_env_tables_are_validated` fails if either loses it.
fn build_map<T: JsonSchema, V: JsonSchema>(
    config_file: &'static str,
    file_name: &'static str,
    description: &'static str,
) -> Schema {
    let mut generator = settings().into_generator();
    // Registers `V` (and everything it references) in the definitions that
    // `into_root_schema_for` then collects.
    let value = generator.subschema_for::<V>();
    finish::<T>(
        config_file,
        file_name,
        description,
        Some((generator, value)),
    )
}

fn finish<T: JsonSchema>(
    config_file: &'static str,
    file_name: &'static str,
    description: &'static str,
    map: Option<(schemars::gen::SchemaGenerator, SchemaNode)>,
) -> Schema {
    let (generator, additional) = match map {
        Some((generator, value)) => (generator, Some(value)),
        None => (settings().into_generator(), None),
    };
    let mut body = generator.into_root_schema_for::<T>();

    if let Some(value) = additional {
        body.schema.object().additional_properties = Some(Box::new(value));
    }

    // The root model's Rust name (`PlacesFile`, `RawFile`) is an implementation
    // detail and reads as noise in an editor's hover. The file name is what the
    // user is looking at.
    let meta = body.schema.metadata();
    meta.title = Some(config_file.to_string());
    meta.description = Some(description.to_string());

    strip_rust_doc_links(&mut body);

    Schema {
        config_file,
        file_name,
        body,
    }
}

/// Rewrite rustdoc intra-doc links into plain text, everywhere in the schema.
///
/// The doc comments are written for someone reading the source, so they use
/// `` [`PlacesFile::resolve_owner`] `` to link a Rust item. That renders as a
/// link in rustdoc and as literal brackets in an editor tooltip, pointing at a
/// name the reader of a TOML file has no way to look up. Stripping the
/// brackets keeps the sentence and drops the false affordance.
///
/// Deliberately only cosmetic. Anything more (trimming to the first
/// paragraph, say) would throw away the parts worth hovering for, like what
/// `codegen = false` trades away.
fn strip_rust_doc_links(root: &mut RootSchema) {
    fn clean(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("[`") {
            let Some(end) = rest[start..].find("`]") else {
                break;
            };
            out.push_str(&rest[..start]);
            out.push('`');
            out.push_str(&rest[start + 2..start + end]);
            out.push('`');
            rest = &rest[start + end + 2..];
        }
        out.push_str(rest);
        out
    }

    fn walk(node: &mut SchemaNode) {
        let SchemaNode::Object(object) = node else {
            return;
        };
        if let Some(meta) = object.metadata.as_mut() {
            if let Some(description) = meta.description.as_mut() {
                *description = clean(description);
            }
        }
        if let Some(validation) = object.object.as_mut() {
            for schema in validation.properties.values_mut() {
                walk(schema);
            }
            if let Some(additional) = validation.additional_properties.as_mut() {
                walk(additional);
            }
        }
        if let Some(array) = object.array.as_mut() {
            if let Some(schemars::schema::SingleOrVec::Single(items)) = array.items.as_mut() {
                walk(items);
            }
        }
        for schema in object.subschemas.as_mut().into_iter().flat_map(|s| {
            s.any_of
                .iter_mut()
                .chain(s.one_of.iter_mut())
                .chain(s.all_of.iter_mut())
                .flatten()
        }) {
            walk(schema);
        }
    }

    for schema in root.definitions.values_mut() {
        walk(schema);
    }
    let mut root_node = SchemaNode::Object(root.schema.clone());
    walk(&mut root_node);
    if let SchemaNode::Object(object) = root_node {
        root.schema = object;
    }
}

/// Every schema, in the order they are written.
pub fn all() -> Vec<Schema> {
    vec![
        build_map::<rbx_core::places::PlacesFile, rbx_core::places::Environment>(
            "rbxplace.toml",
            "rbxplace.schema.json",
            "Environment map shared by every rbx subcommand: env name to universe id, \
             the places under it, and the project owner. See docs/env.md.",
        ),
        build::<rbx_apikey::config::RawFile>(
            "rbxapikey.toml",
            "rbxapikey.schema.json",
            "Declarative Open Cloud API keys: scopes, expiry, allowed CIDRs, and where \
             each key's secret is written. See docs/apikey.md.",
        ),
        build::<rbx_meta::config::Config>(
            "rbxmeta.toml",
            "rbxmeta.schema.json",
            "Universe and place metadata: name, description, devices, social links, \
             server fill, media, and per-env overlays. See docs/meta.md.",
        ),
        build_map::<rbx_config::config::ConfigsFile, rbx_config::config::EnvConfig>(
            "rbxconfig.toml",
            "rbxconfig.schema.json",
            "In-experience config entries per environment, published through the Open \
             Cloud Configs API. See docs/config.md.",
        ),
        build::<rbx_shop::config::Config>(
            "rbxshop.toml",
            "rbxshop.schema.json",
            "Game passes, badges, and developer products: prices, icons, gift twins, \
             codegen output, and per-env overlays. See docs/shop.md.",
        ),
        // The one entry whose model is not what the CLI parses with, because
        // the CLI deliberately parses this file with nothing at all. See
        // `crate::engine_avatar` for why that is a considered exception rather
        // than a lapse, and why the schema is worth shipping anyway.
        build::<crate::engine_avatar::EngineAvatarSettings>(
            "rbxavatar.toml",
            "rbxavatar.schema.json",
            "The modern avatar rules, sent verbatim as `engineAvatarSettings`: rig, \
             animations, clothing, accessories, collisions and body scaling. Pointed \
             at by `game.engine_avatar_settings` in rbxmeta.toml. See docs/meta.md. \
             \n\n\
             GUIDANCE, NOT VALIDATION. Roblox describes this field as an opaque JSON \
             string and publishes no schema for what is inside it, so nothing detects \
             drift here the way CI detects it for the other schemas in this folder: \
             they are regenerated from the models the CLI parses with and a stale one \
             fails the build, while this one is maintained by hand. Key names and \
             mode meanings were read from the Phoenix-CLI worked example on \
             2026-08-17. `additionalProperties` is open throughout, so a key Roblox \
             adds after that date is one your editor stays quiet about and `rbx meta` \
             sends anyway.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk every subschema, including definitions, and hand each object
    /// schema to `check`.
    fn for_each_object(root: &RootSchema, mut check: impl FnMut(&str, &schemars::schema::Schema)) {
        for (name, schema) in &root.definitions {
            check(name, schema);
        }
        check("<root>", &SchemaNode::Object(root.schema.clone()));
    }

    /// The models that close on purpose, because the tool closes with them.
    ///
    /// `[badges.*]` and its per-env overlay carry
    /// `#[serde(deny_unknown_fields)]`: `create_gift` is documented right next
    /// to badges but only applies to passes and products, and swallowing it
    /// there would read as a no-op bug rather than an unsupported field. The
    /// rule below is "never stricter than the tool", not "never strict", so
    /// these are named here rather than dissolved by loosening the check,
    /// and `the_badge_tables_really_do_reject_an_unknown_key` holds the
    /// justification in place.
    const CLOSED_ON_PURPOSE: &[&str] = &["BadgeConfig", "BadgeOverlay"];

    /// The rule from the module docs of `main.rs`, as a test.
    ///
    /// Unknown keys are warned about and ignored, never rejected: a key from a
    /// newer release has to stay loadable, or adopting a field would mean
    /// upgrading every machine in the same instant (`docs/env.md`). A schema
    /// that set `additionalProperties: false` anywhere would paint those keys
    /// red in the editor while the tool accepted them: stricter than the
    /// tool, which is worse than no schema, because it trains people to stop
    /// reading the squiggles.
    #[test]
    fn no_schema_rejects_an_unknown_key() {
        for schema in all() {
            for_each_object(&schema.body, |name, node| {
                let SchemaNode::Object(object) = node else {
                    return;
                };
                let Some(validation) = &object.object else {
                    return;
                };
                let closed = matches!(
                    validation.additional_properties.as_deref(),
                    Some(SchemaNode::Bool(false))
                );
                assert!(
                    !closed || CLOSED_ON_PURPOSE.contains(&name),
                    "{} / {name} closes additionalProperties; the tools warn on \
                     unknown keys rather than rejecting them",
                    schema.config_file
                );
            });
        }
    }

    /// The exemption above is only defensible while the tool is really that
    /// strict. If `deny_unknown_fields` ever comes off the badge models, this
    /// fails, and the schema has to open up with them.
    #[test]
    fn the_badge_tables_really_do_reject_an_unknown_key() {
        let closed = "[badges.Welcome]\nnot_a_badge_field = true\n";
        assert!(
            toml::from_str::<rbx_shop::config::Config>(closed).is_err(),
            "rbx shop rejects an unknown key under [badges.*], so its schema may too"
        );
        let open = "[passes.VIP]\nnot_a_pass_field = true\n";
        assert!(
            toml::from_str::<rbx_shop::config::Config>(open).is_ok(),
            "the neighbouring tables stay open, and so must their schemas"
        );
    }

    /// Every schema is valid JSON and carries the draft its consumers read.
    #[test]
    fn every_schema_declares_draft_07() {
        for schema in all() {
            let json = serde_json::to_value(&schema.body).expect("serialises");
            assert_eq!(
                json.get("$schema").and_then(|v| v.as_str()),
                Some("http://json-schema.org/draft-07/schema#"),
                "{} is not draft 7; taplo and Even Better TOML read draft 7",
                schema.config_file
            );
        }
    }

    /// The title is what an editor shows; it should name the file, not the
    /// Rust type that happens to model it.
    #[test]
    fn every_schema_is_titled_after_its_config_file() {
        for schema in all() {
            assert_eq!(
                schema
                    .body
                    .schema
                    .metadata
                    .as_ref()
                    .and_then(|m| m.title.as_deref()),
                Some(schema.config_file),
            );
        }
    }

    /// The payoff of deriving rather than hand-writing: the doc comments on
    /// the models become hover text. If descriptions stop coming through, the
    /// schemas still validate but the feature has quietly lost half its point.
    #[test]
    fn descriptions_survive_the_derive() {
        let places = all()
            .into_iter()
            .find(|s| s.config_file == "rbxplace.toml")
            .expect("rbxplace.toml is registered");
        let json = serde_json::to_string(&places.body).expect("serialises");
        assert!(
            json.contains("Per-place id map"),
            "the doc comment on Environment::places should reach the schema"
        );
    }

    /// The two files whose content is almost entirely unnamed tables must
    /// actually describe those tables.
    ///
    /// schemars emits nothing for a `#[serde(flatten)]`ed map, so the first
    /// version of these two schemas validated `[owner]` and `[codegen]` and
    /// silently accepted anything else: every `[dev]`, `[prod]` and their
    /// contents. It looked like a working schema and gave no completion and no
    /// type errors where users spend all their time. `build_map` is the fix
    /// and this is what keeps it.
    #[test]
    fn the_env_tables_are_validated() {
        for (file, definition) in [
            ("rbxplace.toml", "Environment"),
            ("rbxconfig.toml", "EnvConfig"),
        ] {
            let schema = all()
                .into_iter()
                .find(|s| s.config_file == file)
                .expect("registered");
            let json = serde_json::to_value(&schema.body).expect("serialises");
            assert_eq!(
                json.pointer("/additionalProperties/$ref")
                    .and_then(|v| v.as_str()),
                Some(format!("#/definitions/{definition}").as_str()),
                "{file}: an unnamed top-level table must validate as {definition}",
            );
        }
    }

    /// A rustdoc link is a dead end in an editor tooltip: it points at a Rust
    /// item the reader of a TOML file cannot look up.
    #[test]
    fn no_description_carries_rustdoc_link_syntax() {
        for schema in all() {
            let json = serde_json::to_string(&schema.body).expect("serialises");
            assert!(
                !json.contains("[`"),
                "{} still has rustdoc intra-doc links in its descriptions",
                schema.config_file
            );
        }
    }

    /// `[envs.<name>]` is a differential overlay, not another copy of the base
    /// table: every field optional, missing ones inherited. It has its own
    /// type, so it must reach the schema as its own definition rather than
    /// being collapsed into the base one.
    #[test]
    fn the_meta_env_overlay_is_its_own_shape() {
        let meta = all()
            .into_iter()
            .find(|s| s.config_file == "rbxmeta.toml")
            .expect("rbxmeta.toml is registered");
        assert!(
            meta.body.definitions.contains_key("EnvOverlay"),
            "definitions: {:?}",
            meta.body.definitions.keys().collect::<Vec<_>>()
        );

        // The base table requires nothing either, but the overlay is the one
        // whose whole contract is "everything optional".
        let SchemaNode::Object(overlay) = &meta.body.definitions["EnvOverlay"] else {
            panic!("EnvOverlay should be an object schema");
        };
        let required = overlay
            .object
            .as_ref()
            .map(|o| o.required.clone())
            .unwrap_or_default();
        assert!(
            required.is_empty(),
            "an overlay field cannot be required: {required:?}"
        );
    }
}
