//! TOML write-back. Every edit goes through toml_edit so a user's comments
//! and formatting survive a pull.

use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::{value, DocumentMut, Item, Table};

use crate::config::{
    AnimationType, AssetOverride, AssetOverrides, Avatar, AvatarType, CollisionType, Config,
    Devices, EnvOverlay, Genre, JointPositioningType, MediaConfig, MediaOverlay, PaidAccess,
    Permissions, PrivateServer, ServerFill, SocialLinks,
};
use crate::lockfile::GameLock;

use super::toml_edit_helpers::*;

pub(super) fn write_config_toml(path: &Path, config: &Config) -> Result<()> {
    let original = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut doc: DocumentMut = original
        .parse()
        .with_context(|| format!("Failed to parse {} as TOML", path.display()))?;

    // [experience] (if present in config)
    if let Some(exp) = &config.experience {
        let t = ensure_table(&mut doc, "experience");
        set_value(t, "universe_id", value(exp.universe_id as i64));
        set_value(t, "place_id", value(exp.place_id as i64));
    } else if doc.contains_key("experience") {
        doc.remove("experience");
    }

    // [game] base
    write_game_block(&mut doc, "game", config);

    // [media] base
    write_media_block(&mut doc, "media", &config.media);

    // [envs.<name>] overlays
    sync_envs_tables(&mut doc, config);

    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Everything `pull` managed to read from an endpoint that is allowed to fail.
///
/// `None` means "not confirmed", for any of four reasons that are the same
/// reason underneath: the endpoint was not called (no cookie), the call failed,
/// Roblox did not send the field, or it sent a value this build does not
/// recognise. All four leave the lockfile with nothing new to say.
#[derive(Default)]
pub(crate) struct ConfirmedReads {
    pub allow_copying: Option<bool>,
    pub server_fill: Option<ServerFill>,
    pub studio_access_to_apis_allowed: Option<bool>,
    pub beta_mode: Option<bool>,
    pub genre: Option<Genre>,
    pub paid_access: Option<PaidAccess>,
    pub avatar_kind: Option<AvatarType>,
    pub avatar_animation: Option<AnimationType>,
    pub avatar_collision: Option<CollisionType>,
    pub avatar_joint_positioning: Option<JointPositioningType>,
}

/// Rebuild the fields of the lock entry that `config_to_lock` must not derive
/// from the config.
///
/// # The rule, stated once
///
/// **The lockfile records what Roblox was confirmed to have.** `config_to_lock`
/// derives its whole argument from the *config*, which is the same thing only
/// for a field that was just read back. For every other field it is a claim
/// nobody checked, and recording it is how `sync` comes to send nothing while
/// `check` reports agreement.
///
/// That failure appeared four separate times in one day, through four different
/// doors: fields Roblox never returns, a cleared hash, a universe-configuration
/// read that failed, and a place-configuration read that was never attempted.
/// Rather than block them one at a time, every field here is now
/// `confirmed.or(previous)`: never the config, ever, by construction. A fifth
/// door would have to be a field added to `GameLock` and not to this function,
/// which is a much smaller hole than the shape that produced the first four.
///
/// The fields absent from this function are the ones read through Open Cloud on
/// a call that fails the whole command rather than warning: name, description,
/// server size, voice chat, visibility, private servers, devices and social
/// links. If those did not arrive, there is no lockfile to write.
pub(super) fn reconcile_lock(
    fresh: &mut GameLock,
    previous: &GameLock,
    confirmed: &ConfirmedReads,
) {
    // Never returned by any endpoint. Only the previous entry can speak.
    fresh.permissions = previous.permissions;
    fresh.avatar.min_scale = previous.avatar.min_scale;
    fresh.avatar.max_scale = previous.avatar.max_scale;
    fresh.avatar.asset_overrides = previous.avatar.asset_overrides;
    // `config_to_lock` clears this: it has no path to hash. Erasing it would
    // make the next `sync` re-send an unchanged avatar document over the
    // cookie-authenticated legacy PATCH.
    fresh.engine_avatar_settings_hash = previous.engine_avatar_settings_hash.clone();

    // Read from an endpoint that may warn instead of failing.
    fresh.allow_copying = confirmed.allow_copying.or(previous.allow_copying);
    fresh.server_fill = confirmed
        .server_fill
        .clone()
        .or_else(|| previous.server_fill.clone());
    fresh.studio_access_to_apis_allowed = confirmed
        .studio_access_to_apis_allowed
        .or(previous.studio_access_to_apis_allowed);
    fresh.beta_mode = confirmed.beta_mode.or(previous.beta_mode);
    fresh.genre = confirmed.genre.or(previous.genre);
    fresh.paid_access = confirmed
        .paid_access
        .clone()
        .or_else(|| previous.paid_access.clone());
    fresh.avatar.kind = confirmed.avatar_kind.or(previous.avatar.kind);
    fresh.avatar.animation = confirmed.avatar_animation.or(previous.avatar.animation);
    fresh.avatar.collision = confirmed.avatar_collision.or(previous.avatar.collision);
    fresh.avatar.joint_positioning = confirmed
        .avatar_joint_positioning
        .or(previous.avatar.joint_positioning);
}

pub(super) fn write_game_block(doc: &mut DocumentMut, key: &str, config: &Config) {
    let game = &config.game;
    let t = ensure_table(doc, key);
    set_opt_str(t, "name", game.name.as_deref());
    set_opt_str(t, "description", game.description.as_deref());
    set_opt_int(t, "server_size", game.server_size.map(|v| v as i64));
    set_opt_bool(t, "voice_chat", game.voice_chat);
    set_opt_bool(t, "allow_copying", game.allow_copying);
    set_opt_str(t, "visibility", game.visibility.map(visibility_str));
    set_opt_bool(
        t,
        "studio_access_to_apis_allowed",
        game.studio_access_to_apis_allowed,
    );
    set_opt_bool(t, "beta_mode", game.beta_mode);

    // sub-tables
    write_private_server_sub(doc, key, game.private_server.as_ref());
    write_devices_sub(doc, key, &game.devices);
    write_server_fill_sub(doc, key, game.server_fill.as_ref());
    write_social_links_sub(doc, key, &game.social_links);
    write_avatar_sub(doc, key, &game.avatar);
    write_asset_overrides_sub(doc, key, game.avatar.asset_overrides.as_ref());
    write_paid_access_sub(doc, key, game.paid_access.as_ref());
    write_permissions_sub(doc, key, game.permissions.as_ref());
    let t = ensure_table(doc, key);
    set_opt_str(t, "genre", game.genre.map(genre_str));
    set_opt_path(
        t,
        "engine_avatar_settings",
        game.engine_avatar_settings.as_deref(),
    );
}
/// Avatar rules, as `[<parent>.avatar]`.
///
/// The scale tables are written from whatever the config already holds rather
/// than from a pull: they are write-only on Roblox's side, so a pull neither
/// learns them nor is entitled to drop them.
pub(super) fn write_avatar_sub(doc: &mut DocumentMut, parent: &str, avatar: &Avatar) {
    if avatar.is_empty() {
        remove_subtable(doc, parent, "avatar");
        return;
    }
    let t = ensure_subtable(doc, parent, "avatar");
    set_opt_str(t, "type", avatar.kind.map(avatar_type_str));
    set_opt_str(t, "animation", avatar.animation.map(animation_type_str));
    set_opt_str(t, "collision", avatar.collision.map(collision_type_str));
    set_opt_str(
        t,
        "joint_positioning",
        avatar.joint_positioning.map(joint_positioning_str),
    );
}

/// Paid access, as `[<parent>.paid_access]`.
pub(super) fn write_paid_access_sub(
    doc: &mut DocumentMut,
    parent: &str,
    paid: Option<&PaidAccess>,
) {
    match paid {
        Some(paid) => {
            let t = ensure_subtable(doc, parent, "paid_access");
            match paid {
                PaidAccess::Free => {
                    set_value(t, "mode", value("free"));
                    t.remove("price");
                }
                PaidAccess::Paid { price } => {
                    set_value(t, "mode", value("paid"));
                    set_value(t, "price", value(*price as i64));
                }
            }
        }
        None => remove_subtable(doc, parent, "paid_access"),
    }
}

/// Third-party permissions, as `[<parent>.permissions]`. All four keys or
/// none, matching `config::Permissions`.
pub(super) fn write_permissions_sub(
    doc: &mut DocumentMut,
    parent: &str,
    perms: Option<&Permissions>,
) {
    match perms {
        Some(p) => {
            let t = ensure_subtable(doc, parent, "permissions");
            set_value(t, "third_party_teleport", value(p.third_party_teleport));
            set_value(t, "third_party_asset", value(p.third_party_asset));
            set_value(t, "third_party_purchase", value(p.third_party_purchase));
            set_value(t, "client_teleport", value(p.client_teleport));
        }
        None => remove_subtable(doc, parent, "permissions"),
    }
}

/// Avatar slot overrides, as `[<parent>.avatar.asset_overrides]`.
///
/// Write-only on Roblox's side, so this only ever mirrors back what the config
/// already held. It exists so the table survives a `pull` that rewrites the
/// file, which it would not if the write-back simply did not know about it.
pub(super) fn write_asset_overrides_sub(
    doc: &mut DocumentMut,
    parent: &str,
    slots: Option<&AssetOverrides>,
) {
    let dotted = format!("{parent}.avatar");
    match slots {
        Some(slots) => {
            let t = ensure_subtable_dotted(doc, &dotted, "asset_overrides");
            for (key, slot) in [
                ("face", slots.face),
                ("head", slots.head),
                ("torso", slots.torso),
                ("left_arm", slots.left_arm),
                ("right_arm", slots.right_arm),
                ("left_leg", slots.left_leg),
                ("right_leg", slots.right_leg),
                ("t_shirt", slots.t_shirt),
                ("shirt", slots.shirt),
                ("pants", slots.pants),
            ] {
                match slot {
                    AssetOverride::Asset(id) => set_value(t, key, value(id as i64)),
                    AssetOverride::PlayerChoice(_) => set_value(t, key, value("player_choice")),
                }
            }
        }
        None => remove_subtable_dotted(doc, &dotted, "asset_overrides"),
    }
}

pub(super) fn avatar_type_str(v: AvatarType) -> &'static str {
    match v {
        AvatarType::R6 => "r6",
        AvatarType::PlayerChoice => "player_choice",
        AvatarType::R15 => "r15",
    }
}

pub(super) fn animation_type_str(v: AnimationType) -> &'static str {
    match v {
        AnimationType::Standard => "standard",
        AnimationType::PlayerChoice => "player_choice",
    }
}

pub(super) fn collision_type_str(v: CollisionType) -> &'static str {
    match v {
        CollisionType::InnerBox => "inner_box",
        CollisionType::OuterBox => "outer_box",
    }
}

pub(super) fn joint_positioning_str(v: JointPositioningType) -> &'static str {
    match v {
        JointPositioningType::Standard => "standard",
        JointPositioningType::ArtistIntent => "artist_intent",
    }
}

pub(super) fn write_private_server_sub(
    doc: &mut DocumentMut,
    parent: &str,
    ps: Option<&PrivateServer>,
) {
    match ps {
        Some(ps) => {
            let t = ensure_subtable(doc, parent, "private_server");
            set_value(t, "price", value(ps.price as i64));
        }
        None => remove_subtable(doc, parent, "private_server"),
    }
}

pub(super) fn write_devices_sub(doc: &mut DocumentMut, parent: &str, devices: &Devices) {
    if devices.is_empty() {
        remove_subtable(doc, parent, "devices");
        return;
    }
    let t = ensure_subtable(doc, parent, "devices");
    set_opt_bool(t, "desktop", devices.desktop);
    set_opt_bool(t, "mobile", devices.mobile);
    set_opt_bool(t, "tablet", devices.tablet);
    set_opt_bool(t, "console", devices.console);
    set_opt_bool(t, "vr", devices.vr);
}

pub(super) fn write_server_fill_sub(doc: &mut DocumentMut, parent: &str, sf: Option<&ServerFill>) {
    match sf {
        Some(sf) => {
            let t = ensure_subtable(doc, parent, "server_fill");
            set_value(t, "mode", value(server_fill_mode_str(sf)));
            match sf {
                ServerFill::Custom { reserved_slots } => {
                    set_value(t, "reserved_slots", value(*reserved_slots as i64));
                }
                _ => {
                    t.remove("reserved_slots");
                }
            }
        }
        None => remove_subtable(doc, parent, "server_fill"),
    }
}

pub(super) fn write_social_links_sub(doc: &mut DocumentMut, parent: &str, links: &SocialLinks) {
    set_social(doc, parent, "facebook", &links.facebook);
    set_social(doc, parent, "twitter", &links.twitter);
    set_social(doc, parent, "youtube", &links.youtube);
    set_social(doc, parent, "twitch", &links.twitch);
    set_social(doc, parent, "discord", &links.discord);
    set_social(doc, parent, "roblox_group", &links.roblox_group);
    set_social(doc, parent, "guilded", &links.guilded);
}

pub(super) fn write_media_block(doc: &mut DocumentMut, key: &str, media: &MediaConfig) {
    let t = ensure_table(doc, key);
    set_opt_path(t, "icon", media.icon.as_deref());
    set_path_array(t, "thumbnails", &media.thumbnails);
    set_opt_path(t, "dir", media.dir.as_deref());
    if media.bleed {
        t.remove("bleed");
    } else {
        set_value(t, "bleed", value(false));
    }
    if media.language_code == "en_us" {
        t.remove("language_code");
    } else {
        set_value(t, "language_code", value(media.language_code.clone()));
    }
}

/// Write or update `[envs.<name>]` overlays in the doc. Removes envs that
/// no longer exist in the config (or are entirely empty).
pub(super) fn sync_envs_tables(doc: &mut DocumentMut, config: &Config) {
    // Add/update overlays
    for (name, overlay) in &config.envs {
        if overlay.is_empty() {
            remove_env_section(doc, name);
            continue;
        }
        write_env_overlay(doc, name, overlay);
    }

    // Remove envs from the doc that are not in the config anymore.
    if let Some(envs) = doc.get_mut("envs").and_then(|i| i.as_table_mut()) {
        let to_remove: Vec<String> = envs
            .iter()
            .filter_map(|(k, _)| {
                if !config.envs.contains_key(k)
                    || config.envs.get(k).map(|o| o.is_empty()).unwrap_or(true)
                {
                    Some(k.to_string())
                } else {
                    None
                }
            })
            .collect();
        for k in to_remove {
            envs.remove(&k);
        }
        if envs.is_empty() {
            doc.remove("envs");
        }
    }
}

pub(super) fn write_env_overlay(doc: &mut DocumentMut, env: &str, overlay: &EnvOverlay) {
    let envs_table = ensure_table(doc, "envs");
    if !envs_table.contains_key(env) {
        envs_table.insert(env, Item::Table(Table::new()));
    }
    let t = envs_table[env]
        .as_table_mut()
        .expect("env entry is not a table");

    set_opt_str(t, "name", overlay.name.as_deref());
    set_opt_str(t, "description", overlay.description.as_deref());
    set_opt_int(t, "server_size", overlay.server_size.map(|v| v as i64));
    set_opt_bool(t, "voice_chat", overlay.voice_chat);
    set_opt_bool(t, "allow_copying", overlay.allow_copying);
    set_opt_str(t, "visibility", overlay.visibility.map(visibility_str));
    set_opt_bool(
        t,
        "studio_access_to_apis_allowed",
        overlay.studio_access_to_apis_allowed,
    );
    set_opt_bool(t, "beta_mode", overlay.beta_mode);

    // sub-tables under envs.<env>.<sub>
    let env_parent = format!("envs.{}", env);
    set_opt_str(t, "genre", overlay.genre.map(genre_str));
    set_opt_path(
        t,
        "engine_avatar_settings",
        overlay.engine_avatar_settings.as_deref(),
    );

    write_private_server_sub(doc, &env_parent, overlay.private_server.as_ref());
    write_devices_sub(doc, &env_parent, &overlay.devices);
    write_server_fill_sub(doc, &env_parent, overlay.server_fill.as_ref());
    write_social_links_sub(doc, &env_parent, &overlay.social_links);
    write_avatar_sub(doc, &env_parent, &overlay.avatar);
    write_asset_overrides_sub(doc, &env_parent, overlay.avatar.asset_overrides.as_ref());
    write_paid_access_sub(doc, &env_parent, overlay.paid_access.as_ref());
    write_permissions_sub(doc, &env_parent, overlay.permissions.as_ref());
    write_media_overlay(doc, &env_parent, &overlay.media);
}

pub(super) fn write_media_overlay(doc: &mut DocumentMut, parent: &str, media: &MediaOverlay) {
    if media.is_empty() {
        remove_subtable_dotted(doc, parent, "media");
        return;
    }
    let t = ensure_subtable_dotted(doc, parent, "media");
    set_opt_path(t, "icon", media.icon.as_deref());
    match &media.thumbnails {
        Some(thumbs) => set_path_array(t, "thumbnails", thumbs),
        None => {
            t.remove("thumbnails");
        }
    }
    set_opt_path(t, "dir", media.dir.as_deref());
    set_opt_bool(t, "bleed", media.bleed);
    set_opt_str(t, "language_code", media.language_code.as_deref());
}

pub(super) fn remove_env_section(doc: &mut DocumentMut, env: &str) {
    if let Some(envs) = doc.get_mut("envs").and_then(|i| i.as_table_mut()) {
        envs.remove(env);
        if envs.is_empty() {
            doc.remove("envs");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{Experience, Game, SocialLink, Visibility};
    use std::collections::BTreeMap;

    /// Seed a config file and return `(tempdir, path)`. The tempdir must stay
    /// alive for the path to remain valid.
    fn seed(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rbxmeta.toml");
        std::fs::write(&path, content).expect("seed write");
        (dir, path)
    }

    fn social(title: &str) -> SocialLink {
        SocialLink {
            title: title.to_string(),
            url: format!("https://example.test/{}", title.to_ascii_lowercase()),
        }
    }

    /// Every device toggle set, with mixed values so a field written to the
    /// wrong key is not masked by an identical neighbour.
    fn all_devices() -> Devices {
        Devices {
            desktop: Some(true),
            mobile: Some(false),
            tablet: Some(true),
            console: Some(false),
            vr: Some(true),
        }
    }

    fn all_social_links() -> SocialLinks {
        SocialLinks {
            facebook: Some(social("facebook")),
            twitter: Some(social("twitter")),
            youtube: Some(social("youtube")),
            twitch: Some(social("twitch")),
            discord: Some(social("discord")),
            roblox_group: Some(social("robloxgroup")),
            guilded: Some(social("guilded")),
        }
    }

    /// Ten slots left to the player. The fixture that then overrides one of
    /// them proves the write-back distinguishes the two forms.
    fn all_player_choice() -> AssetOverrides {
        let choice = AssetOverride::PlayerChoice(crate::config::PlayerChoiceMarker::PlayerChoice);
        AssetOverrides {
            face: choice,
            head: choice,
            torso: choice,
            left_arm: choice,
            right_arm: choice,
            left_leg: choice,
            right_leg: choice,
            t_shirt: choice,
            shirt: choice,
            pants: choice,
        }
    }

    /// A `Config` with every optional field populated.
    ///
    /// The struct literals here are deliberately exhaustive: no
    /// `..Default::default()`. A field added to `config.rs` breaks this
    /// constructor at compile time, which forces whoever adds it to decide what
    /// the `toml_edit` mirror should do with it. That is the whole point: the
    /// mirror is a second serializer, and nothing else makes drift visible.
    fn fully_populated_config() -> Config {
        let mut envs = BTreeMap::new();
        envs.insert(
            "prod".to_string(),
            EnvOverlay {
                name: Some("overlay name".to_string()),
                description: Some("overlay description".to_string()),
                server_size: Some(70),
                voice_chat: Some(false),
                allow_copying: Some(false),
                visibility: Some(Visibility::Private),
                studio_access_to_apis_allowed: Some(false),
                beta_mode: Some(false),
                private_server: Some(PrivateServer { price: 0 }),
                devices: Devices {
                    desktop: Some(false),
                    mobile: Some(true),
                    tablet: Some(false),
                    console: Some(true),
                    vr: Some(false),
                },
                social_links: SocialLinks {
                    facebook: Some(social("prod-facebook")),
                    twitter: Some(social("prod-twitter")),
                    youtube: Some(social("prod-youtube")),
                    twitch: Some(social("prod-twitch")),
                    discord: Some(social("prod-discord")),
                    roblox_group: Some(social("prod-robloxgroup")),
                    guilded: Some(social("prod-guilded")),
                },
                server_fill: Some(ServerFill::Empty),
                permissions: Some(Permissions {
                    third_party_teleport: false,
                    third_party_asset: true,
                    third_party_purchase: false,
                    client_teleport: true,
                }),
                avatar: Avatar {
                    kind: Some(AvatarType::R6),
                    animation: Some(AnimationType::Standard),
                    collision: Some(CollisionType::InnerBox),
                    joint_positioning: Some(JointPositioningType::Standard),
                    min_scale: None,
                    max_scale: None,
                    asset_overrides: Some(all_player_choice()),
                },
                paid_access: Some(PaidAccess::Free),
                genre: Some(Genre::Adventure),
                engine_avatar_settings: Some(PathBuf::from("prod-avatar.json")),
                media: MediaOverlay {
                    icon: Some(PathBuf::from("assets/prod/icon.png")),
                    thumbnails: Some(vec![
                        PathBuf::from("assets/prod/one.png"),
                        PathBuf::from("assets/prod/two.png"),
                    ]),
                    dir: Some(PathBuf::from("assets/prod")),
                    bleed: Some(false),
                    language_code: Some("de_de".to_string()),
                },
            },
        );

        Config {
            experience: Some(Experience {
                universe_id: 1234,
                place_id: 5678,
            }),
            game: Game {
                name: Some("Parity Game".to_string()),
                description: Some("A description".to_string()),
                server_size: Some(42),
                voice_chat: Some(true),
                private_server: Some(PrivateServer { price: 25 }),
                devices: all_devices(),
                social_links: all_social_links(),
                server_fill: Some(ServerFill::Custom { reserved_slots: 7 }),
                allow_copying: Some(true),
                visibility: Some(Visibility::Public),
                studio_access_to_apis_allowed: Some(true),
                beta_mode: Some(true),
                permissions: Some(Permissions {
                    third_party_teleport: true,
                    third_party_asset: false,
                    third_party_purchase: true,
                    client_teleport: false,
                }),
                avatar: Avatar {
                    kind: Some(AvatarType::R15),
                    animation: Some(AnimationType::PlayerChoice),
                    collision: Some(CollisionType::OuterBox),
                    joint_positioning: Some(JointPositioningType::ArtistIntent),
                    // Scale tables stay out of the parity fixture on purpose:
                    // the mirror never writes them, because Roblox never
                    // returns them and a pull that emitted a scale range
                    // would be inventing one.
                    min_scale: None,
                    max_scale: None,
                    asset_overrides: Some(AssetOverrides {
                        pants: AssetOverride::Asset(12345),
                        ..all_player_choice()
                    }),
                },
                paid_access: Some(PaidAccess::Paid { price: 25 }),
                genre: Some(Genre::TownAndCity),
                engine_avatar_settings: Some(PathBuf::from("avatar.json")),
            },
            media: MediaConfig {
                icon: Some(PathBuf::from("assets/icon.png")),
                thumbnails: vec![
                    PathBuf::from("assets/one.png"),
                    PathBuf::from("assets/two.png"),
                ],
                dir: Some(PathBuf::from("assets")),
                // Both deliberately non-default: the mirror omits these keys at
                // their default value, so a default-valued test would pass even
                // if the mirror never wrote them at all.
                bleed: false,
                language_code: "fr_fr".to_string(),
            },
            envs,
        }
    }

    /// The parity test. Anything the mirror fails to write is data the user
    /// silently loses on the next `rbx meta pull`.
    #[test]
    fn toml_edit_mirror_writes_every_field_the_serde_model_reads() {
        let config = fully_populated_config();
        let (_dir, path) = seed("");

        write_config_toml(&path, &config).expect("write through the toml_edit mirror");

        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(
            reparsed,
            config,
            "toml_edit mirror and serde model disagree.\n--- file written ---\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
    }

    /// Default-valued `media.bleed` / `media.language_code` are omitted by the
    /// mirror on purpose. That is only correct as long as serde's defaults agree.
    #[test]
    fn omitted_media_defaults_reparse_to_the_same_values() {
        let mut config = fully_populated_config();
        config.media.bleed = true;
        config.media.language_code = "en_us".to_string();
        // Clear the env media overlay too, so the assertions below can look at
        // the whole file rather than having to isolate the base `[media]` table.
        config
            .envs
            .get_mut("prod")
            .expect("prod overlay")
            .media
            .bleed = None;
        config
            .envs
            .get_mut("prod")
            .expect("prod overlay")
            .media
            .language_code = None;
        let (_dir, path) = seed("");

        write_config_toml(&path, &config).expect("write through the toml_edit mirror");

        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(
            !written.contains("bleed"),
            "default bleed should not be written: {written}"
        );
        assert!(
            !written.contains("language_code"),
            "default language_code should not be written: {written}"
        );
        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(reparsed, config);
    }

    /// A cleared field must be removed from the file, not left stale.
    #[test]
    fn mirror_removes_fields_that_are_no_longer_set() {
        let (_dir, path) = seed(
            r#"[experience]
universe_id = 1
place_id = 2

[game]
name = "old"
beta_mode = true

[game.private_server]
price = 100

[game.devices]
desktop = true

[game.server_fill]
mode = "custom"
reserved_slots = 3

[game.social_links.discord]
title = "Discord"
url = "https://example.test/discord"

[envs.dev]
name = "dev name"
"#,
        );

        let empty = Config {
            experience: None,
            game: Game::default(),
            media: MediaConfig::default(),
            envs: BTreeMap::new(),
        };
        write_config_toml(&path, &empty).expect("write through the toml_edit mirror");

        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(reparsed, empty);
    }

    /// Round-trip with user comments and an env overlay: the mirror exists only
    /// to preserve the former, so a test that ignores them tests nothing.
    #[test]
    fn round_trip_preserves_user_comments_and_overlays() {
        let (_dir, path) = seed(
            r#"# rbx meta configuration
# Managed by hand, please keep these notes.

[experience]
universe_id = 1 # the live universe
place_id = 2

[game]
# The public-facing name. Marketing owns this string.
name = "old name"
server_size = 10

# Private servers are a revenue line; do not disable without asking.
[game.private_server]
price = 50

[media]
# Icons live next to the source art.
icon = "assets/old-icon.png"

[envs.dev]
# Dev is deliberately hidden.
visibility = "private"
"#,
        );

        let mut config = Config::load(&path).expect("load seed");
        config.game.name = Some("new name".to_string());
        config.game.server_size = Some(25);
        config.media.icon = Some(PathBuf::from("assets/new-icon.png"));
        config.envs.get_mut("dev").expect("dev overlay").description = Some("dev only".to_string());

        write_config_toml(&path, &config).expect("write through the toml_edit mirror");
        let written = std::fs::read_to_string(&path).expect("read back");

        for comment in [
            "# rbx meta configuration",
            "# Managed by hand, please keep these notes.",
            "# The public-facing name. Marketing owns this string.",
            "# Private servers are a revenue line; do not disable without asking.",
            "# Icons live next to the source art.",
            "# Dev is deliberately hidden.",
            "# the live universe",
        ] {
            assert!(
                written.contains(comment),
                "comment lost on write-back: {comment}\n--- file written ---\n{written}"
            );
        }

        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(reparsed, config);
    }
}

#[cfg(test)]
mod reconcile_lock_tests {
    use super::{reconcile_lock, ConfirmedReads};
    use crate::config::{Avatar, AvatarType, Genre, PaidAccess, Permissions, ServerFill};
    use crate::lockfile::GameLock;

    fn perms(on: bool) -> Option<Permissions> {
        Some(Permissions {
            third_party_teleport: on,
            third_party_asset: on,
            third_party_purchase: on,
            client_teleport: on,
        })
    }

    /// A lock recording what Roblox was actually sent, last time.
    fn previous() -> GameLock {
        GameLock {
            permissions: perms(false),
            genre: Some(Genre::All),
            paid_access: Some(PaidAccess::Free),
            studio_access_to_apis_allowed: Some(false),
            beta_mode: Some(false),
            allow_copying: Some(false),
            server_fill: Some(ServerFill::Automatic),
            engine_avatar_settings_hash: Some("deadbeef".into()),
            avatar: Avatar {
                kind: Some(AvatarType::R6),
                ..Avatar::default()
            },
            ..GameLock::default()
        }
    }

    /// What `config_to_lock` hands over: every field taken from the config,
    /// confirmed or not.
    fn from_config() -> GameLock {
        GameLock {
            permissions: perms(true),
            genre: Some(Genre::WildWest),
            paid_access: Some(PaidAccess::Paid { price: 25 }),
            studio_access_to_apis_allowed: Some(true),
            beta_mode: Some(true),
            allow_copying: Some(true),
            server_fill: Some(ServerFill::Empty),
            engine_avatar_settings_hash: None,
            avatar: Avatar {
                kind: Some(AvatarType::R15),
                ..Avatar::default()
            },
            ..GameLock::default()
        }
    }

    /// The rule in one test: when nothing was confirmed, **no field** may come
    /// from the config.
    ///
    /// This is the shape that produced the same failure four times: fields
    /// Roblox never returns, a cleared hash, a failed universe read, and a
    /// place read that was never attempted. One assertion per door.
    #[test]
    fn nothing_confirmed_means_nothing_from_the_config() {
        let mut fresh = from_config();
        reconcile_lock(&mut fresh, &previous(), &ConfirmedReads::default());

        assert_eq!(fresh.permissions.map(|p| p.client_teleport), Some(false));
        assert_eq!(
            fresh.engine_avatar_settings_hash.as_deref(),
            Some("deadbeef")
        );
        assert_eq!(fresh.genre, Some(Genre::All));
        assert_eq!(fresh.paid_access, Some(PaidAccess::Free));
        assert_eq!(fresh.studio_access_to_apis_allowed, Some(false));
        assert_eq!(fresh.beta_mode, Some(false));
        assert_eq!(fresh.allow_copying, Some(false));
        assert_eq!(fresh.server_fill, Some(ServerFill::Automatic));
        assert_eq!(fresh.avatar.kind, Some(AvatarType::R6));
    }

    /// A confirmed value wins over the previous entry. Without this the lock
    /// would freeze on its first value and never notice a real change.
    #[test]
    fn a_confirmed_read_replaces_the_previous_value() {
        let mut fresh = from_config();
        reconcile_lock(
            &mut fresh,
            &previous(),
            &ConfirmedReads {
                genre: Some(Genre::Pirate),
                allow_copying: Some(true),
                avatar_kind: Some(AvatarType::PlayerChoice),
                ..ConfirmedReads::default()
            },
        );

        assert_eq!(fresh.genre, Some(Genre::Pirate));
        assert_eq!(fresh.allow_copying, Some(true));
        assert_eq!(fresh.avatar.kind, Some(AvatarType::PlayerChoice));
        // Untouched by that read, so still the previous entry.
        assert_eq!(fresh.beta_mode, Some(false));
    }

    /// The write-only four ignore `ConfirmedReads` entirely: no endpoint can
    /// confirm them, so there is no field on that struct to try.
    #[test]
    fn the_write_only_fields_never_come_from_a_read() {
        let mut fresh = from_config();
        reconcile_lock(
            &mut fresh,
            &previous(),
            &ConfirmedReads {
                genre: Some(Genre::Pirate),
                ..ConfirmedReads::default()
            },
        );

        assert_eq!(fresh.permissions.map(|p| p.client_teleport), Some(false));
        assert_eq!(fresh.avatar.min_scale, None);
    }

    /// A first pull has no previous entry and nothing confirmed. Clearing is
    /// right: the lock then says "not confirmed" rather than inventing.
    #[test]
    fn an_empty_previous_lock_clears_rather_than_invents() {
        let mut fresh = from_config();
        reconcile_lock(&mut fresh, &GameLock::default(), &ConfirmedReads::default());

        assert_eq!(fresh.permissions, None);
        assert_eq!(fresh.genre, None);
        assert_eq!(fresh.beta_mode, None);
        assert_eq!(fresh.engine_avatar_settings_hash, None);
    }
}
