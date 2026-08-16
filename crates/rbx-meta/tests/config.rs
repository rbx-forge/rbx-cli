#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rbx_meta::config::{
    Config, Devices, EnvOverlay, Experience, Game, MediaConfig, MediaOverlay, PrivateServer,
    ServerFill, SocialLink, SocialLinks, Visibility,
};
use tempfile::tempdir;

fn write_config(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxmeta.toml");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

// ---------------------------------------------------------------------------
// Load / save
// ---------------------------------------------------------------------------

#[test]
fn loads_minimal_config() {
    let (_d, path) = write_config(
        r#"
[game]
name = "My Game"
description = "Fun"
"#,
    );
    let config = Config::load(&path).unwrap();
    assert!(config.experience.is_none());
    assert_eq!(config.game.name.as_deref(), Some("My Game"));
    assert_eq!(config.game.description.as_deref(), Some("Fun"));
    assert!(config.envs.is_empty());
}

#[test]
fn loads_standalone_with_experience() {
    let (_d, path) = write_config(
        r#"
[experience]
universe_id = 42
place_id = 100
"#,
    );
    let config = Config::load(&path).unwrap();
    let exp = config.experience.as_ref().unwrap();
    assert_eq!(exp.universe_id, 42);
    assert_eq!(exp.place_id, 100);
}

#[test]
fn default_template_parses() {
    let (_d, path) = write_config(&Config::default_template());
    Config::load(&path).unwrap();
}

#[test]
fn save_load_round_trip_preserves_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxmeta.toml");

    let mut config = Config {
        experience: Some(Experience {
            universe_id: 1,
            place_id: 2,
        }),
        game: Game {
            name: Some("X".into()),
            description: Some("Y".into()),
            server_size: Some(50),
            voice_chat: Some(true),
            private_server: Some(PrivateServer { price: 100 }),
            ..Default::default()
        },
        media: MediaConfig::default(),
        envs: BTreeMap::new(),
    };
    config.game.devices.desktop = Some(true);
    config.save(&path).unwrap();

    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.game.name.as_deref(), Some("X"));
    assert_eq!(loaded.game.server_size, Some(50));
    assert_eq!(loaded.game.voice_chat, Some(true));
    assert_eq!(
        loaded.game.private_server.as_ref().map(|p| p.price),
        Some(100)
    );
    assert_eq!(loaded.game.devices.desktop, Some(true));
}

// ---------------------------------------------------------------------------
// resolve_env: base + overlay merging
// ---------------------------------------------------------------------------

#[test]
fn resolve_env_no_overlay_returns_base_unchanged() {
    let config = Config {
        experience: None,
        game: Game {
            name: Some("base name".into()),
            visibility: Some(Visibility::Public),
            ..Default::default()
        },
        media: MediaConfig::default(),
        envs: BTreeMap::new(),
    };
    let (game, _media) = config.resolve_env(Some("dev"));
    assert_eq!(game.name.as_deref(), Some("base name"));
    assert!(matches!(game.visibility, Some(Visibility::Public)));
}

#[test]
fn resolve_env_overlay_overrides_specific_field() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".into(),
        EnvOverlay {
            visibility: Some(Visibility::Private),
            ..Default::default()
        },
    );
    let config = Config {
        experience: None,
        game: Game {
            name: Some("base".into()),
            visibility: Some(Visibility::Public),
            ..Default::default()
        },
        media: MediaConfig::default(),
        envs,
    };

    // dev overlay flips visibility, but keeps base name.
    let (dev, _) = config.resolve_env(Some("dev"));
    assert_eq!(dev.name.as_deref(), Some("base"));
    assert!(matches!(dev.visibility, Some(Visibility::Private)));

    // prod has no overlay, so base wins.
    let (prod, _) = config.resolve_env(Some("prod"));
    assert!(matches!(prod.visibility, Some(Visibility::Public)));
}

#[test]
fn resolve_env_devices_merge_field_by_field() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".into(),
        EnvOverlay {
            devices: Devices {
                console: Some(false),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let config = Config {
        experience: None,
        game: Game {
            devices: Devices {
                desktop: Some(true),
                mobile: Some(true),
                console: Some(true),
                ..Default::default()
            },
            ..Default::default()
        },
        media: MediaConfig::default(),
        envs,
    };

    let (dev, _) = config.resolve_env(Some("dev"));
    // Overlay only touched console.
    assert_eq!(dev.devices.desktop, Some(true));
    assert_eq!(dev.devices.mobile, Some(true));
    assert_eq!(dev.devices.console, Some(false));
}

#[test]
fn resolve_env_social_links_merge_per_platform() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".into(),
        EnvOverlay {
            social_links: SocialLinks {
                discord: Some(SocialLink {
                    title: "dev discord".into(),
                    url: "https://discord.gg/dev".parse().unwrap(),
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let config = Config {
        experience: None,
        game: Game {
            social_links: SocialLinks {
                discord: Some(SocialLink {
                    title: "base discord".into(),
                    url: "https://discord.gg/base".parse().unwrap(),
                }),
                twitter: Some(SocialLink {
                    title: "x".into(),
                    url: "https://x.com/me".parse().unwrap(),
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        media: MediaConfig::default(),
        envs,
    };

    let (dev, _) = config.resolve_env(Some("dev"));
    // Overlay replaced discord, twitter kept from base.
    assert_eq!(
        dev.social_links.discord.as_ref().map(|s| s.title.as_str()),
        Some("dev discord")
    );
    assert_eq!(
        dev.social_links.twitter.as_ref().map(|s| s.title.as_str()),
        Some("x")
    );
}

#[test]
fn resolve_env_media_overlay_overrides_icon_path() {
    let mut envs = BTreeMap::new();
    envs.insert(
        "dev".into(),
        EnvOverlay {
            media: MediaOverlay {
                icon: Some("dev-icon.png".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let config = Config {
        experience: None,
        game: Game::default(),
        media: MediaConfig {
            icon: Some("base-icon.png".into()),
            ..Default::default()
        },
        envs,
    };

    let (_, media) = config.resolve_env(Some("dev"));
    assert_eq!(
        media.icon.as_deref(),
        Some(std::path::Path::new("dev-icon.png"))
    );
}

#[test]
fn resolve_env_none_env_returns_base() {
    let config = Config {
        experience: None,
        game: Game {
            name: Some("base".into()),
            ..Default::default()
        },
        media: MediaConfig::default(),
        envs: BTreeMap::new(),
    };
    let (game, _) = config.resolve_env(None);
    assert_eq!(game.name.as_deref(), Some("base"));
}

// ---------------------------------------------------------------------------
// validate_invariants
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_private_with_paid_private_server() {
    let game = Game {
        visibility: Some(Visibility::Private),
        private_server: Some(PrivateServer { price: 100 }),
        ..Default::default()
    };
    let err = Config::validate_invariants(&game).unwrap_err().to_string();
    assert!(
        err.contains("PUBLIC"),
        "error should explain the rule: {err}"
    );
}

#[test]
fn validate_accepts_private_with_free_private_server() {
    let game = Game {
        visibility: Some(Visibility::Private),
        private_server: Some(PrivateServer { price: 0 }),
        ..Default::default()
    };
    Config::validate_invariants(&game).unwrap();
}

#[test]
fn validate_accepts_public_with_paid_private_server() {
    let game = Game {
        visibility: Some(Visibility::Public),
        private_server: Some(PrivateServer { price: 100 }),
        ..Default::default()
    };
    Config::validate_invariants(&game).unwrap();
}

// ---------------------------------------------------------------------------
// ServerFill API mapping
// ---------------------------------------------------------------------------

#[test]
fn server_fill_api_strings() {
    assert_eq!(ServerFill::Automatic.social_slot_type(), "Automatic");
    assert_eq!(ServerFill::Empty.social_slot_type(), "Empty");
    assert_eq!(
        ServerFill::Custom { reserved_slots: 5 }.social_slot_type(),
        "Custom"
    );
}

#[test]
fn server_fill_from_legacy_round_trip() {
    let auto = ServerFill::from_legacy(Some("Automatic"), None).unwrap();
    assert!(matches!(auto, ServerFill::Automatic));

    let empty = ServerFill::from_legacy(Some("Empty"), None).unwrap();
    assert!(matches!(empty, ServerFill::Empty));

    let custom = ServerFill::from_legacy(Some("Custom"), Some(7)).unwrap();
    assert!(matches!(custom, ServerFill::Custom { reserved_slots: 7 }));

    assert!(ServerFill::from_legacy(None, None).is_none());
    assert!(ServerFill::from_legacy(Some("Garbage"), None).is_none());
}

#[test]
fn server_fill_custom_count() {
    assert_eq!(ServerFill::Automatic.custom_count(), None);
    assert_eq!(ServerFill::Empty.custom_count(), None);
    assert_eq!(
        ServerFill::Custom { reserved_slots: 3 }.custom_count(),
        Some(3)
    );
}

// ---------------------------------------------------------------------------
// Visibility parsing
// ---------------------------------------------------------------------------

#[test]
fn visibility_parses_open_cloud_enum_values() {
    assert!(matches!(
        Visibility::from_open_cloud("PUBLIC"),
        Some(Visibility::Public)
    ));
    assert!(matches!(
        Visibility::from_open_cloud("PRIVATE"),
        Some(Visibility::Private)
    ));
    assert!(Visibility::from_open_cloud("DRAFT").is_none());
}

#[test]
fn visibility_is_public_helper() {
    assert!(Visibility::Public.is_public());
    assert!(!Visibility::Private.is_public());
}
