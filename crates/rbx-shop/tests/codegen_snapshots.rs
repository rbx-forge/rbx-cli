//! The generated module folder, snapshotted whole.
//!
//! The Luau and TypeScript this crate emits is a contract with the user's game
//! code: a `require` that returns a table of the wrong shape breaks a running
//! experience, not a build. `tests/codegen.rs` asserted that contract with
//! `contains()` on fragments, which passes happily while the surrounding
//! output degrades — a stray blank line, a lost `local`, a type that stopped
//! being exported.
//!
//! So the whole folder goes into one snapshot per style, files and separators
//! included. Intentional output changes are `cargo insta review`; unintentional
//! ones are a diff of the exact lines that moved.
//!
//! The behavioural tests next door stay: env dispatch, stub semantics, the
//! `--check` exit codes and the error cases say *why* the output looks like
//! this, which a snapshot never does.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rbx_shop::codegen;
use rbx_shop::config::{
    BadgeConfig, CodegenConfig, CodegenPaths, CodegenStyle, Config, PassConfig, PassOverlay,
    ProductConfig,
};
use rbx_shop::lockfile::{BadgeLock, EnvLock, Lockfile, PassLock, ProductLock, LOCKFILE_VERSION};

// ── fixture ──

fn pass_cfg(path: Option<&str>, create_gift: bool) -> PassConfig {
    PassConfig {
        name: None,
        price: Some(499),
        description: None,
        icon: None,
        for_sale: true,
        regional_pricing: false,
        create_gift,
        path: path.map(str::to_string),
    }
}

fn product_cfg(path: Option<&str>) -> ProductConfig {
    ProductConfig {
        name: None,
        price: 99,
        description: None,
        icon: None,
        for_sale: true,
        regional_pricing: false,
        store_page: false,
        create_gift: false,
        path: path.map(str::to_string),
    }
}

fn badge_cfg() -> BadgeConfig {
    BadgeConfig {
        name: None,
        description: None,
        icon: None,
        enabled: true,
        path: Some("rewards".into()),
    }
}

fn pass_lock(id: u64, name: &str) -> PassLock {
    PassLock {
        id,
        name: name.into(),
        price: Some(499),
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
    }
}

fn badge_lock(id: u64, name: &str) -> BadgeLock {
    BadgeLock {
        id,
        name: name.into(),
        description: None,
        enabled: true,
        icon_asset_id: None,
        icon_hash: None,
    }
}

fn product_lock(id: u64, name: &str) -> ProductLock {
    ProductLock {
        id,
        name: name.into(),
        price: 99,
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
        store_page: false,
    }
}

/// One config exercising every knob that reaches the emitters at once: custom
/// `[codegen.paths]`, a per-resource `path` override, `[codegen.extra]`, a
/// gift-enabled pass, and an env overlay that makes a pass exclusive to `dev`.
fn config(style: CodegenStyle, typescript: bool) -> Config {
    Config {
        experience: None,
        owner: None,
        codegen: CodegenConfig {
            output: Some("src/shared/GameIds".into()),
            typescript,
            style,
            paths: CodegenPaths {
                passes: None,
                badges: None,
                products: Some("shop.items".into()),
            },
            extra: BTreeMap::from([("passes.legacy_vip".to_string(), 1234567_u64)]),
        },
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::from([
            // `path` override: this one lands outside the default `passes` table.
            ("VIP".to_string(), pass_cfg(Some("shop.specials"), true)),
            ("Starter".to_string(), pass_cfg(None, false)),
        ]),
        badges: BTreeMap::from([("Welcome".to_string(), badge_cfg())]),
        products: BTreeMap::from([("Coins100".to_string(), product_cfg(None))]),
        envs: BTreeMap::from([(
            "dev".to_string(),
            rbx_shop::config::EnvOverlay {
                passes: BTreeMap::from([(
                    "BetaPass".to_string(),
                    PassOverlay {
                        price: Some(0),
                        ..Default::default()
                    },
                )]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    }
}

/// The lockfile a `sync` of that config would leave behind: `dev` also carries
/// the env-exclusive `BetaPass`, so `prod` picks up a 0 stub for it. `GiftVIP`
/// is the derived twin of the gift-enabled `VIP`.
fn lockfile() -> Lockfile {
    Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([
            (
                "dev".to_string(),
                EnvLock {
                    universe_id: 9876543210,
                    passes: BTreeMap::from([
                        ("VIP".into(), pass_lock(11, "VIP")),
                        ("Starter".into(), pass_lock(12, "Starter")),
                        ("BetaPass".into(), pass_lock(13, "BetaPass")),
                    ]),
                    badges: BTreeMap::from([("Welcome".into(), badge_lock(21, "Welcome"))]),
                    products: BTreeMap::from([
                        ("Coins100".into(), product_lock(31, "Coins100")),
                        ("GiftVIP".into(), product_lock(32, "[GIFT] VIP")),
                    ]),
                },
            ),
            (
                "prod".to_string(),
                EnvLock {
                    universe_id: 9876543211,
                    passes: BTreeMap::from([
                        ("VIP".into(), pass_lock(111, "VIP")),
                        ("Starter".into(), pass_lock(112, "Starter")),
                    ]),
                    badges: BTreeMap::from([("Welcome".into(), badge_lock(121, "Welcome"))]),
                    products: BTreeMap::from([
                        ("Coins100".into(), product_lock(131, "Coins100")),
                        ("GiftVIP".into(), product_lock(132, "[GIFT] VIP")),
                    ]),
                },
            ),
        ]),
    }
}

/// Every file the plan would write, concatenated under its own name. Taken
/// from `plan` rather than from disk so the snapshot covers exactly what
/// `generate` and `codegen --check` both compare against.
fn generated_folder(style: CodegenStyle, typescript: bool) -> String {
    let plan = codegen::plan(
        &config(style, typescript),
        &lockfile(),
        std::path::Path::new("."),
    )
    .unwrap()
    .expect("the fixture sets [codegen].output and a non-empty lockfile");

    let mut out = String::new();
    for file in &plan.files {
        let name = file
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .expect("generated files are named");
        out.push_str(&format!("======== {name} ========\n{}\n", file.content));
    }
    out
}

// ── snapshots ──

/// Nested style with TypeScript on — the fullest output the crate produces.
#[test]
fn nested_style_folder() {
    insta::assert_snapshot!("nested", generated_folder(CodegenStyle::Nested, true));
}

/// Flat style, where every path becomes a bracketed string key rather than a
/// table of tables.
#[test]
fn flat_style_folder() {
    insta::assert_snapshot!("flat", generated_folder(CodegenStyle::Flat, true));
}

/// With `typescript = false` the `.d.ts` is simply absent — asserted here
/// rather than by reading a directory, so the snapshot is the file list too.
#[test]
fn nested_style_folder_without_typescript() {
    insta::assert_snapshot!(
        "nested_no_typescript",
        generated_folder(CodegenStyle::Nested, false)
    );
}
