#![allow(clippy::unwrap_used)]
//! Integration tests for the codegen folder + dispatcher output.

use std::collections::BTreeMap;

use rbx_shop::codegen;
use rbx_shop::config::{CodegenConfig, CodegenStyle, Config};
use rbx_shop::lockfile::{BadgeLock, EnvLock, Lockfile, PassLock, ProductLock, LOCKFILE_VERSION};
use tempfile::tempdir;

fn pass(id: u64, name: &str) -> PassLock {
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

fn badge(id: u64, name: &str) -> BadgeLock {
    BadgeLock {
        id,
        name: name.into(),
        description: None,
        enabled: true,
        icon_asset_id: None,
        icon_hash: None,
    }
}

fn product(id: u64, name: &str, price: u64) -> ProductLock {
    ProductLock {
        id,
        name: name.into(),
        price,
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
        store_page: false,
    }
}

fn build_config(output: &str, style: CodegenStyle, typescript: bool) -> Config {
    Config {
        experience: None,
        owner: None,
        codegen: CodegenConfig {
            output: Some(output.into()),
            typescript,
            style,
            paths: Default::default(),
            extra: BTreeMap::new(),
        },
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::new(),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs: BTreeMap::new(),
    }
}

#[test]
fn emits_type_module_init_and_per_env_files() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([
            (
                "dev".to_string(),
                EnvLock {
                    universe_id: 100,
                    passes: BTreeMap::from([("VIP".into(), pass(11, "VIP"))]),
                    badges: BTreeMap::from([("Welcome".into(), badge(21, "Welcome"))]),
                    products: BTreeMap::from([("Coins".into(), product(31, "Coins", 99))]),
                },
            ),
            (
                "prod".to_string(),
                EnvLock {
                    universe_id: 200,
                    passes: BTreeMap::from([("VIP".into(), pass(12, "VIP"))]),
                    badges: BTreeMap::from([("Welcome".into(), badge(22, "Welcome"))]),
                    products: BTreeMap::from([("Coins".into(), product(32, "Coins", 99))]),
                },
            ),
        ]),
    };

    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let out = dir.path().join("out/GameIds");
    assert!(out.is_dir());
    assert!(out.join("init.luau").exists());
    assert!(out.join("GameIdsType.luau").exists());
    assert!(out.join("dev.luau").exists());
    assert!(out.join("prod.luau").exists());
}

/// The snapshots cover the type module's shape. This keeps the one claim they
/// cannot make: the absence of a directive, and why it is absent.
#[test]
fn the_type_module_sets_no_language_mode() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([(
            "dev".to_string(),
            EnvLock {
                universe_id: 100,
                passes: BTreeMap::from([("VIP".into(), pass(11, "VIP"))]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    };
    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let types = std::fs::read_to_string(dir.path().join("out/GameIds/GameIdsType.luau")).unwrap();
    // No `--!strict`: the consuming project sets the language mode in its
    // .luaurc, and a directive per generated file would only duplicate it.
    assert!(!types.contains("--!strict"));
}

#[test]
fn init_uses_exhaustive_dispatch() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([
            (
                "dev".to_string(),
                EnvLock {
                    universe_id: 9876543210,
                    passes: BTreeMap::from([("VIP".into(), pass(11, "VIP"))]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::new(),
                },
            ),
            (
                "prod".to_string(),
                EnvLock {
                    universe_id: 9876543211,
                    passes: BTreeMap::from([("VIP".into(), pass(12, "VIP"))]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::new(),
                },
            ),
        ]),
    };
    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let init = std::fs::read_to_string(dir.path().join("out/GameIds/init.luau")).unwrap();
    assert!(!init.contains("--!strict"));
    assert!(init.contains("require(script.GameIdsType)"));
    assert!(init.contains("export type GameIds = Types.GameIds"));
    assert!(init.contains(r#"export type EnvName = "dev" | "prod""#));
    assert!(init.contains("UNIVERSE_TO_ENV: { [number]: EnvName }"));
    assert!(init.contains("[9876543210] = \"dev\""));
    assert!(init.contains("[9876543211] = \"prod\""));
    assert!(init.contains("local function exhaustiveMatch(value: never): never"));
    assert!(init.contains("if env == \"dev\" then"));
    assert!(init.contains("elseif env == \"prod\" then"));
    assert!(init.contains("return require(script.dev)"));
    assert!(init.contains("return require(script.prod)"));
    assert!(init.contains("\texhaustiveMatch(env)\n"));
    assert!(init.contains("error(\"luau\")"));
    // The old IDS lookup must be gone.
    assert!(!init.contains("local IDS:"));
}

#[test]
fn missing_resource_is_stubbed_to_zero() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([
            (
                "dev".to_string(),
                EnvLock {
                    universe_id: 100,
                    passes: BTreeMap::from([
                        ("VIP".into(), pass(11, "VIP")),
                        ("BetaPass".into(), pass(99, "BetaPass")),
                    ]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::new(),
                },
            ),
            (
                "prod".to_string(),
                EnvLock {
                    universe_id: 200,
                    passes: BTreeMap::from([("VIP".into(), pass(22, "VIP"))]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::new(),
                },
            ),
        ]),
    };

    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let prod = std::fs::read_to_string(dir.path().join("out/GameIds/prod.luau")).unwrap();
    assert!(
        prod.contains("BetaPass = 0"),
        "missing entries should be 0-stubbed:\n{prod}"
    );
    assert!(prod.contains("0 are stubs"), "stub note should appear");

    let dev = std::fs::read_to_string(dir.path().join("out/GameIds/dev.luau")).unwrap();
    assert!(dev.contains("BetaPass = 99"));
    assert!(!dev.contains("0 are stubs"), "dev has no stubs so no note");

    let types = std::fs::read_to_string(dir.path().join("out/GameIds/GameIdsType.luau")).unwrap();
    assert!(types.contains("BetaPass: number"));
}

/// `typescript = true` puts a fifth file in the folder. Its contents are
/// snapshotted; what matters here is that the flag is what produces it.
#[test]
fn typescript_emits_init_d_ts() {
    let dir = tempdir().unwrap();
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([(
            "default".to_string(),
            EnvLock {
                universe_id: 1,
                passes: BTreeMap::from([("VIP".into(), pass(10, "VIP"))]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    };

    codegen::generate(
        &build_config("out/On", CodegenStyle::Nested, true),
        &lockfile,
        dir.path(),
    )
    .unwrap();
    assert!(dir.path().join("out/On/init.d.ts").exists());

    codegen::generate(
        &build_config("out/Off", CodegenStyle::Nested, false),
        &lockfile,
        dir.path(),
    )
    .unwrap();
    assert!(!dir.path().join("out/Off/init.d.ts").exists());
}

#[test]
fn standalone_default_env_emits_single_env_module() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([(
            "default".to_string(),
            EnvLock {
                universe_id: 42,
                passes: BTreeMap::from([("VIP".into(), pass(7, "VIP"))]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    };
    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let out = dir.path().join("out/GameIds");
    assert!(out.join("init.luau").exists());
    assert!(out.join("GameIdsType.luau").exists());
    assert!(out.join("default.luau").exists());

    let default_module = std::fs::read_to_string(out.join("default.luau")).unwrap();
    assert!(default_module.contains("VIP = 7"));
    assert!(default_module.contains("Types.gameIds("));

    let init = std::fs::read_to_string(out.join("init.luau")).unwrap();
    assert!(init.contains("[42] = \"default\""));
    assert!(init.contains(r#"export type EnvName = "default""#));
    assert!(init.contains("if env == \"default\" then"));
    assert!(init.contains("return require(script.default)"));
}

#[test]
fn extra_entries_appear_in_every_env() {
    let dir = tempdir().unwrap();
    let mut config = build_config("out/GameIds", CodegenStyle::Nested, false);
    config
        .codegen
        .extra
        .insert("passes.legacy".into(), 12345678);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([
            (
                "dev".to_string(),
                EnvLock {
                    universe_id: 1,
                    passes: BTreeMap::from([("VIP".into(), pass(10, "VIP"))]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::new(),
                },
            ),
            (
                "prod".to_string(),
                EnvLock {
                    universe_id: 2,
                    passes: BTreeMap::from([("VIP".into(), pass(20, "VIP"))]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::new(),
                },
            ),
        ]),
    };
    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    for env in ["dev", "prod"] {
        let content =
            std::fs::read_to_string(dir.path().join(format!("out/GameIds/{}.luau", env))).unwrap();
        assert!(
            content.contains("legacy = 12345678"),
            "extra entry missing in {env}.luau:\n{content}"
        );
    }
}

#[test]
fn env_name_colliding_with_generated_module_is_rejected() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    for bad in ["init", "GameIdsType"] {
        let lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            envs: BTreeMap::from([(
                bad.to_string(),
                EnvLock {
                    universe_id: 1,
                    passes: BTreeMap::from([("VIP".into(), pass(10, "VIP"))]),
                    badges: BTreeMap::new(),
                    products: BTreeMap::new(),
                },
            )]),
        };
        let err = codegen::generate(&config, &lockfile, dir.path()).unwrap_err();
        assert!(
            err.to_string().contains(bad),
            "error should name the colliding env: {err}"
        );
    }
}

#[test]
fn env_name_with_path_separator_is_rejected() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([(
            "../escape".to_string(),
            EnvLock {
                universe_id: 1,
                passes: BTreeMap::from([("VIP".into(), pass(10, "VIP"))]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    };
    assert!(codegen::generate(&config, &lockfile, dir.path()).is_err());
}

#[test]
fn output_ending_in_luau_extension_is_rejected() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds.luau", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([(
            "default".to_string(),
            EnvLock {
                universe_id: 1,
                passes: BTreeMap::from([("VIP".into(), pass(10, "VIP"))]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    };
    let err = codegen::generate(&config, &lockfile, dir.path()).unwrap_err();
    assert!(err.to_string().contains("GameIds"));
    assert!(!dir.path().join("out/GameIds.luau").exists());
}

#[test]
fn folder_name_drives_var_and_wrapper_names() {
    let dir = tempdir().unwrap();
    // Different folder name → different var/wrapper names + type module
    let config = build_config("out/Assets", CodegenStyle::Nested, false);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([(
            "default".to_string(),
            EnvLock {
                universe_id: 1,
                passes: BTreeMap::from([("VIP".into(), pass(10, "VIP"))]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    };
    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let out = dir.path().join("out/Assets");
    assert!(out.join("AssetsType.luau").exists());

    let types = std::fs::read_to_string(out.join("AssetsType.luau")).unwrap();
    assert!(types.contains("export type Assets ="));
    assert!(types.contains("local function assets(x: Assets): Assets"));

    let default_module = std::fs::read_to_string(out.join("default.luau")).unwrap();
    assert!(default_module.contains("Types.assets("));

    let init = std::fs::read_to_string(out.join("init.luau")).unwrap();
    assert!(init.contains("require(script.AssetsType)"));
    assert!(init.contains("export type Assets = Types.Assets"));
}

#[test]
fn generated_modules_carry_the_generated_marker() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, true);
    let lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: BTreeMap::from([(
            "dev".to_string(),
            EnvLock {
                universe_id: 100,
                passes: BTreeMap::from([("VIP".into(), pass(11, "VIP"))]),
                badges: BTreeMap::new(),
                products: BTreeMap::new(),
            },
        )]),
    };

    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let out = dir.path().join("out/GameIds");
    for name in ["GameIdsType.luau", "init.luau", "dev.luau"] {
        let content = std::fs::read_to_string(out.join(name)).unwrap();
        assert!(
            content.contains("-- This file is automatically @generated by rbx shop"),
            "{name} is missing the generated marker"
        );
        assert!(content.contains("-- It is not intended for manual editing."));
    }

    let dts = std::fs::read_to_string(out.join("init.d.ts")).unwrap();
    assert!(dts.contains("// This file is automatically @generated by rbx shop."));
}

// ---------------------------------------------------------------------------
// Write-or-check: the generated folder must stay a byte-for-byte function of
// rbxshop.toml + rbxshop.lock, or `rbx shop codegen --check` is worthless.
// ---------------------------------------------------------------------------

/// A lockfile with one env per `(name, universe_id, pass_id)` triple.
fn lockfile_with(envs: &[(&str, u64, u64)]) -> Lockfile {
    Lockfile {
        version: LOCKFILE_VERSION,
        envs: envs
            .iter()
            .map(|(name, universe_id, pass_id)| {
                (
                    (*name).to_string(),
                    EnvLock {
                        universe_id: *universe_id,
                        passes: BTreeMap::from([("VIP".into(), pass(*pass_id, "VIP"))]),
                        badges: BTreeMap::new(),
                        products: BTreeMap::new(),
                    },
                )
            })
            .collect(),
    }
}

#[test]
fn regenerating_prunes_the_module_of_an_env_that_disappeared() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let out = dir.path().join("out/GameIds");

    codegen::generate(
        &config,
        &lockfile_with(&[("dev", 100, 11), ("qa", 150, 12)]),
        dir.path(),
    )
    .unwrap();
    assert!(out.join("qa.luau").exists());

    // qa is gone from the lockfile. Left behind, qa.luau would still look
    // generated and still be requirable from game code.
    codegen::generate(&config, &lockfile_with(&[("dev", 100, 11)]), dir.path()).unwrap();
    assert!(!out.join("qa.luau").exists());
    assert!(out.join("dev.luau").exists());
}

#[test]
fn pruning_never_touches_a_file_we_did_not_generate() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let out = dir.path().join("out/GameIds");

    codegen::generate(&config, &lockfile_with(&[("dev", 100, 11)]), dir.path()).unwrap();
    // Same extension, no generated header — someone's own module.
    let handwritten = out.join("Helpers.luau");
    std::fs::write(&handwritten, "return {}\n").unwrap();

    codegen::generate(&config, &lockfile_with(&[("dev", 100, 11)]), dir.path()).unwrap();
    assert!(
        handwritten.exists(),
        "pruning must only remove files carrying our header"
    );
}

#[test]
fn a_freshly_generated_folder_passes_its_own_check() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, true);
    let lockfile = lockfile_with(&[("dev", 100, 11), ("prod", 200, 12)]);
    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    let plan = codegen::plan(&config, &lockfile, dir.path())
        .unwrap()
        .expect("codegen is configured");
    assert!(plan.stale.is_empty());

    let mut report = rbx_core::generated::CheckReport::new();
    for file in &plan.files {
        report.check(file).unwrap();
    }
    assert!(!report.has_drift());
}

#[test]
fn a_hand_edited_module_fails_the_check() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    let lockfile = lockfile_with(&[("dev", 100, 11)]);
    codegen::generate(&config, &lockfile, dir.path()).unwrap();

    // The scenario the whole feature exists for: someone edits the id in the
    // generated module instead of syncing.
    let module = dir.path().join("out/GameIds/dev.luau");
    let tampered = std::fs::read_to_string(&module)
        .unwrap()
        .replace("11", "99");
    std::fs::write(&module, tampered).unwrap();

    let plan = codegen::plan(&config, &lockfile, dir.path())
        .unwrap()
        .unwrap();
    let mut report = rbx_core::generated::CheckReport::new();
    for file in &plan.files {
        report.check(file).unwrap();
    }
    assert!(report.has_drift());
    assert!(report
        .finish("rbxshop.lock", "rbx shop codegen")
        .unwrap_err()
        .to_string()
        .contains("rbx shop codegen"));
}

#[test]
fn a_leftover_module_is_reported_as_stale_by_the_plan() {
    let dir = tempdir().unwrap();
    let config = build_config("out/GameIds", CodegenStyle::Nested, false);
    codegen::generate(
        &config,
        &lockfile_with(&[("dev", 100, 11), ("qa", 150, 12)]),
        dir.path(),
    )
    .unwrap();

    // Plan against a lockfile without qa, without writing: the check path must
    // see qa.luau as drift rather than silently ignoring it.
    let plan = codegen::plan(&config, &lockfile_with(&[("dev", 100, 11)]), dir.path())
        .unwrap()
        .unwrap();
    assert_eq!(plan.stale.len(), 1);
    assert!(plan.stale[0].ends_with("qa.luau"));
}
