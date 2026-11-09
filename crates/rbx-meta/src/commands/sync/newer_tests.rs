//! `sync` against a Roblox that is not where the lockfile left it.

#![allow(clippy::unwrap_used)]

use serde_json::json;
use tempfile::TempDir;

use rbx_core::GlobalFlags;

use super::canned::{place_named, read, Canned};
use super::*;
use crate::lockfile::EnvLock;

const PLACES: &str = r#"
[dev]
universe_id = 100
[dev.places]
main = 200

[prod]
universe_id = 150
confirm = true
[prod.places]
main = 250
"#;

/// Two envs declaring `name = "Declared"`, each recorded as holding `locked`.
fn repo(locked: Option<&str>) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rbxplace.toml"), PLACES).unwrap();
    std::fs::write(
        dir.path().join("rbxmeta.toml"),
        "[game]\nname = \"Declared\"\n",
    )
    .unwrap();

    if let Some(locked) = locked {
        let mut lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            ..Default::default()
        };
        for (env, universe_id, place_id) in [("dev", 100, 200), ("prod", 150, 250)] {
            lockfile.envs.insert(
                env.to_string(),
                EnvLock {
                    universe_id,
                    place_id,
                    game: GameLock {
                        name: Some(locked.to_string()),
                        ..Default::default()
                    },
                    media: Default::default(),
                },
            );
        }
        lockfile.save(&dir.path().join(LOCKFILE_NAME)).unwrap();
    }
    dir
}

fn flags(dir: &TempDir, env: Option<&str>) -> GlobalFlags {
    GlobalFlags {
        api_key: None,
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: env.map(String::from),
        place: None,
        places: dir.path().join("rbxplace.toml"),
        universe_id: None,
        place_id: Vec::new(),
    }
}

async fn sync(dir: &TempDir, env: &str, options: Options, roblox: Canned) -> Result<()> {
    let global = flags(dir, Some(env));
    let ctx = MetaCtx {
        config: dir.path().join("rbxmeta.toml"),
        global: &global,
    };
    run_with(&ctx, options, None, &roblox).await
}

const APPLY: Options = Options {
    dry_run: false,
    yes: true,
    overwrite_newer: false,
};

const DRY_RUN: Options = Options {
    dry_run: true,
    yes: true,
    overwrite_newer: false,
};

fn lockfile_bytes(dir: &TempDir) -> Option<Vec<u8>> {
    std::fs::read(dir.path().join(LOCKFILE_NAME)).ok()
}

fn recorded_name(dir: &TempDir, env: &str) -> Option<String> {
    Lockfile::load(&dir.path().join(LOCKFILE_NAME))
        .unwrap()
        .env_view(env)
        .game
        .name
}

/// The report that started this: a value edited on the Creator Hub, the two
/// local files in agreement, and `sync` answering "everything is in sync".
/// The run is refused before anything is sent or recorded, and names both
/// ways out.
#[tokio::test]
async fn a_value_changed_on_roblox_stops_the_run_and_names_both_ways_out() {
    let dir = repo(Some("Declared"));
    let before = lockfile_bytes(&dir);

    let err = sync(
        &dir,
        "prod",
        APPLY,
        Canned::always(|| place_named("Renamed on the site")),
    )
    .await
    .expect_err("Roblox was changed since the lockfile recorded it");

    let text = format!("{err:#}");
    assert!(
        text.contains(r#"name: lockfile "Declared", Roblox "Renamed on the site""#),
        "names the value: {text}"
    );
    assert!(text.contains("[prod]"), "names the env: {text}");
    assert!(
        text.contains("`rbx meta pull --env prod`"),
        "names the way to keep it: {text}"
    );
    assert!(
        text.contains("--overwrite-newer"),
        "names the way to replace it: {text}"
    );
    assert_eq!(
        lockfile_bytes(&dir),
        before,
        "a refused run records nothing"
    );
}

/// A preview must not show a plan the real run refuses.
#[tokio::test]
async fn a_dry_run_is_refused_the_same_way() {
    let dir = repo(Some("Declared"));

    let err = sync(
        &dir,
        "prod",
        DRY_RUN,
        Canned::always(|| place_named("Renamed on the site")),
    )
    .await
    .expect_err("the real run would refuse");

    assert!(format!("{err:#}").contains("--overwrite-newer"));
}

/// Asked for, the replacement is planned. A dry run stops there, having
/// written nothing.
#[tokio::test]
async fn overwrite_newer_lets_the_plan_through() {
    let dir = repo(Some("Declared"));
    let before = lockfile_bytes(&dir);

    sync(
        &dir,
        "prod",
        Options {
            overwrite_newer: true,
            ..DRY_RUN
        },
        Canned::always(|| place_named("Renamed on the site")),
    )
    .await
    .expect("the replacement was asked for");

    assert_eq!(lockfile_bytes(&dir), before, "a dry run writes nothing");
}

/// One env changed on Roblox refuses the whole run, before the env that is
/// fine gets written: a run that applied `dev` and then refused `prod` is the
/// half-applied run the planning pass exists to prevent.
#[tokio::test]
async fn one_env_changed_on_roblox_refuses_the_whole_run() {
    let dir = repo(Some("Declared"));
    let before = lockfile_bytes(&dir);
    // `dev` holds what was recorded and has nothing to do; `prod` was renamed.
    let roblox = Canned(Box::new(|universe_id| match universe_id {
        100 => place_named("Declared"),
        _ => place_named("Renamed on the site"),
    }));

    let err = sync(&dir, "all", APPLY, roblox)
        .await
        .expect_err("prod was changed on Roblox");

    let text = format!("{err:#}");
    assert!(text.contains("[prod]"), "{text}");
    assert!(!text.contains("[dev]"), "only the env that changed: {text}");
    assert_eq!(lockfile_bytes(&dir), before);
}

/// Roblox already holds what the config asks for, and the lockfile is behind.
/// Nothing is sent, and the lockfile catches up, or `check` would go on
/// reporting a change no sync could ever apply.
#[tokio::test]
async fn roblox_already_matching_the_config_brings_the_lockfile_up_to_date() {
    let dir = repo(Some("An older name"));

    sync(
        &dir,
        "dev",
        APPLY,
        Canned::always(|| place_named("Declared")),
    )
    .await
    .expect("nothing to write");

    assert_eq!(recorded_name(&dir, "dev").as_deref(), Some("Declared"));
    assert_eq!(
        recorded_name(&dir, "prod").as_deref(),
        Some("An older name"),
        "an env the run did not name is left alone"
    );
}

/// An env that was never synced, on an experience that already matches: the
/// section is created from what was read, ids included.
#[tokio::test]
async fn a_first_sync_that_finds_nothing_to_write_still_records_the_env() {
    let dir = repo(None);

    sync(
        &dir,
        "dev",
        APPLY,
        Canned::always(|| place_named("Declared")),
    )
    .await
    .expect("nothing to write");

    let lockfile = Lockfile::load(&dir.path().join(LOCKFILE_NAME)).unwrap();
    let section = lockfile.env_view("dev");
    assert_eq!(
        (section.universe_id, section.place_id),
        (100, 200),
        "the section says which experience it describes"
    );
    assert_eq!(section.game.name.as_deref(), Some("Declared"));
}

/// The same finding on a dry run is reported and not recorded.
#[tokio::test]
async fn a_dry_run_never_touches_the_lockfile() {
    let dir = repo(Some("An older name"));
    let before = lockfile_bytes(&dir);

    sync(
        &dir,
        "dev",
        DRY_RUN,
        Canned::always(|| place_named("Declared")),
    )
    .await
    .expect("nothing to write");

    assert_eq!(lockfile_bytes(&dir), before);
}

/// A link somebody added on the Creator Hub, declared nowhere in the
/// repository: nothing is planned for it and nothing is recorded, so the run
/// is the no-op it was before `sync` read Roblox.
#[tokio::test]
async fn a_link_added_on_roblox_and_declared_nowhere_is_left_alone() {
    let dir = repo(Some("Declared"));
    let before = lockfile_bytes(&dir);

    sync(
        &dir,
        "dev",
        APPLY,
        Canned::always(|| {
            read(
                json!({ "discordSocialLink": { "title": "Join us", "uri": "https://discord.gg/example" } }),
                json!({ "displayName": "Declared" }),
            )
        }),
    )
    .await
    .expect("nothing to write");

    assert_eq!(lockfile_bytes(&dir), before);
}

/// The hint is a command that can be pasted, for the target the run had.
#[test]
fn the_pull_hint_is_spelled_for_the_target() {
    let planned = |env: &str| PlannedEnv {
        env: env.to_string(),
        universe_id: 1234567890,
        place_id: 109876543210987,
        game: Game::default(),
        media: MediaConfig::default(),
        plan: SyncPlan::default(),
        seen: GameLock::default(),
        newer: vec!["name: lockfile \"a\", Roblox \"b\"".to_string()],
    };
    let dir = tempfile::tempdir().unwrap();
    let (prod, dev, standalone) = (planned("prod"), planned("dev"), planned(DEFAULT_ENV));

    let named = flags(&dir, Some("all"));
    let ctx = MetaCtx {
        config: dir.path().join("rbxmeta.toml"),
        global: &named,
    };
    assert_eq!(pull_hint(&ctx, &[&prod]), "`rbx meta pull --env prod`");
    assert_eq!(
        pull_hint(&ctx, &[&dev, &prod]),
        "`rbx meta pull --env <name>` for each env listed"
    );

    let bare = flags(&dir, None);
    let ctx = MetaCtx {
        config: dir.path().join("rbxmeta.toml"),
        global: &bare,
    };
    assert_eq!(pull_hint(&ctx, &[&standalone]), "`rbx meta pull`");

    let mut by_id = flags(&dir, None);
    by_id.universe_id = Some(1234567890);
    by_id.place_id = vec![109876543210987];
    let ctx = MetaCtx {
        config: dir.path().join("rbxmeta.toml"),
        global: &by_id,
    };
    assert_eq!(
        pull_hint(&ctx, &[&standalone]),
        "`rbx meta pull --universe-id 1234567890 --place-id 109876543210987`"
    );
}
