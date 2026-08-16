//! Writing `rbxshop.toml` back without destroying it.
//!
//! `pull` and `rename` used to reserialize the whole model through serde,
//! which dropped comments, reordered keys, and deleted every key the model
//! does not have a field for. These tests are the contract for the
//! replacement: the file that comes back is the file that went in, plus
//! exactly the edit that was asked for.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use rbx_shop::config::{unknown_root_keys, Config, KeyRename, ResourceKind};

fn write(dir: &Path, contents: &str) -> PathBuf {
    let path = dir.join("rbxshop.toml");
    std::fs::write(&path, contents).unwrap();
    path
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// A config with a comment on every structural element, so a writer that drops
/// any of them fails loudly rather than partially.
const COMMENTED: &str = r#"# Top-of-file note the user cares about.

[experience]
universe_id = 1234          # the live universe

# Everything the game sells.
[passes.VIP]
# The flagship pass. Do not lower this price without asking.
price = 499
description = "VIP access"

[passes.Starter]
price = 99

# Rewards, not purchases.
[badges.Welcome]
description = "Welcome!"

[products.Coins]
price = 50

# Prod charges more.
[envs.prod.passes.VIP]
price = 999
"#;

#[test]
fn a_write_back_that_changes_nothing_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), COMMENTED);

    let config = Config::load(&path).unwrap();
    config.save_in_place(&path).unwrap();

    assert_eq!(
        read(&path),
        COMMENTED,
        "an idempotent write must be byte-identical"
    );
}

#[test]
fn changing_one_price_leaves_every_comment_and_key_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), COMMENTED);

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().price = Some(799);
    config.save_in_place(&path).unwrap();

    let after = read(&path);
    assert!(after.contains("price = 799"));
    assert!(!after.contains("price = 499"));

    // Every comment survived, including the one attached to the edited key.
    for comment in [
        "# Top-of-file note the user cares about.",
        "# the live universe",
        "# Everything the game sells.",
        "# The flagship pass. Do not lower this price without asking.",
        "# Rewards, not purchases.",
        "# Prod charges more.",
    ] {
        assert!(after.contains(comment), "lost comment: {comment}\n{after}");
    }

    // And so did the order the user wrote things in.
    let order: Vec<usize> = ["[experience]", "[passes.VIP]", "[passes.Starter]"]
        .iter()
        .map(|h| after.find(h).unwrap_or_else(|| panic!("lost {h}")))
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "keys were reordered");
}

/// The behaviour `docs/env.md` calls out as the bug that had to go: a key the
/// model does not know about must not disappear because something else was
/// edited.
#[test]
fn an_unmodeled_root_key_survives_the_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"
# Settings for a tool that is not rbx.
[deploy]
channel = "beta"

[passes.VIP]
price = 499
"#,
    );

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().price = Some(999);
    config.save_in_place(&path).unwrap();

    let after = read(&path);
    assert!(after.contains("[deploy]"), "{after}");
    assert!(after.contains(r#"channel = "beta""#), "{after}");
    assert!(after.contains("# Settings for a tool that is not rbx."));
    assert!(after.contains("price = 999"));
}

/// Same rule one level down. `PassConfig`/`ProductConfig` accept unknown
/// fields silently at load, so a stray key inside a resource is exactly the
/// kind of thing a reserializing writer eats.
#[test]
fn an_unmodeled_key_inside_a_resource_survives_the_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"
[passes.VIP]
price = 499
internal_note = "renewal negotiated 2026-03"
"#,
    );

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().price = Some(999);
    config.save_in_place(&path).unwrap();

    let after = read(&path);
    assert!(
        after.contains(r#"internal_note = "renewal negotiated 2026-03""#),
        "{after}"
    );
    assert!(after.contains("price = 999"));
}

/// Absence already means the default. Writing them anyway would turn every
/// pull into a diff across the whole file.
#[test]
fn defaults_are_not_written_into_a_file_that_never_mentioned_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"
[passes.VIP]
price = 499
"#,
    );

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().price = Some(999);
    config.save_in_place(&path).unwrap();

    let after = read(&path);
    for noise in ["for_sale", "regional_pricing", "create_gift"] {
        assert!(!after.contains(noise), "wrote a default: {noise}\n{after}");
    }
}

/// But a value that diverges from the default has to be written, or the file
/// stops describing the shop.
#[test]
fn a_non_default_value_is_written_even_when_the_key_was_absent() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"
[passes.VIP]
price = 499
"#,
    );

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().for_sale = false;
    config.save_in_place(&path).unwrap();

    assert!(read(&path).contains("for_sale = false"));
}

/// A default the user wrote out explicitly is theirs to keep — updating it in
/// place is an edit, deleting it is a rewrite.
#[test]
fn an_explicitly_written_default_is_updated_rather_than_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"
[passes.VIP]
price = 499
for_sale = true
"#,
    );

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().for_sale = false;
    config.save_in_place(&path).unwrap();
    assert!(read(&path).contains("for_sale = false"));

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().for_sale = true;
    config.save_in_place(&path).unwrap();
    assert!(
        read(&path).contains("for_sale = true"),
        "a key the user wrote must not vanish when it returns to its default"
    );
}

/// A field the model cleared is genuinely gone — this is the one case where
/// removing a line is correct.
#[test]
fn a_field_cleared_on_the_model_is_removed_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"
[passes.VIP]
price = 499
description = "VIP access"
"#,
    );

    let mut config = Config::load(&path).unwrap();
    config.passes.get_mut("VIP").unwrap().description = None;
    config.save_in_place(&path).unwrap();

    let after = read(&path);
    assert!(!after.contains("description"), "{after}");
    assert!(after.contains("price = 499"));
}

#[test]
fn a_resource_dropped_from_the_model_is_removed_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), COMMENTED);

    let mut config = Config::load(&path).unwrap();
    config.passes.remove("Starter");
    config.save_in_place(&path).unwrap();

    let after = read(&path);
    assert!(!after.contains("[passes.Starter]"), "{after}");
    assert!(after.contains("[passes.VIP]"));
}

/// The rename path. Without the key-move hint the entry would be dropped and a
/// bare table appended at the end, losing the comment that explains it.
#[test]
fn a_rename_carries_the_entrys_comment_and_its_place_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), COMMENTED);

    let mut config = Config::load(&path).unwrap();
    let entry = config.passes.remove("VIP").unwrap();
    config.passes.insert("vip_pass".into(), entry);
    config
        .save_in_place_renaming(
            &path,
            &[KeyRename {
                kind: ResourceKind::Pass,
                from: "VIP".into(),
                to: "vip_pass".into(),
            }],
        )
        .unwrap();

    let after = read(&path);
    assert!(after.contains("[passes.vip_pass]"), "{after}");
    assert!(!after.contains("[passes.VIP]"));
    assert!(
        after.contains("# The flagship pass. Do not lower this price without asking."),
        "the renamed entry lost its comment\n{after}"
    );
    assert!(
        after.find("[passes.vip_pass]").unwrap() < after.find("[passes.Starter]").unwrap(),
        "the renamed entry was moved to the end\n{after}"
    );
}

/// Included files are written back the same way, so a comment in a split
/// config is no less safe than one in the main file.
#[test]
fn an_included_file_is_written_back_in_place_too() {
    let dir = tempfile::tempdir().unwrap();
    let main = write(
        dir.path(),
        "[experience]\nuniverse_id = 1\n\n[include]\nfiles = [\"passes.toml\"]\n",
    );
    let included = dir.path().join("passes.toml");
    std::fs::write(
        &included,
        "# Passes live here.\n[passes.VIP]\nprice = 499\n",
    )
    .unwrap();

    let files = Config::load_all(&main).unwrap();
    assert_eq!(files.len(), 2);
    let mut inc = files[1].config.clone();
    inc.passes.get_mut("VIP").unwrap().price = Some(999);
    inc.save_in_place(&files[1].path).unwrap();

    let after = read(&included);
    assert!(after.contains("# Passes live here."), "{after}");
    assert!(after.contains("price = 999"));
}

// ── unknown-key reporting ──

/// Preserved is not the same as honoured. The keys are kept on write, and
/// named on load, so a misspelled one does not read as a key that took effect.
#[test]
fn unrecognised_root_keys_are_named() {
    let found = unknown_root_keys(
        r#"
[experience]
universe_id = 1

[deploy]
channel = "beta"

[pases.VIP]
price = 1
"#,
    );
    assert_eq!(found, ["deploy", "pases"]);
}

#[test]
fn a_config_of_only_known_keys_reports_nothing() {
    assert!(unknown_root_keys(COMMENTED).is_empty());
}

/// A document that does not parse is the typed loader's error to report, with
/// a line number this cannot produce.
#[test]
fn an_unparseable_document_reports_no_unknown_keys() {
    assert!(unknown_root_keys("[[[not toml").is_empty());
}
