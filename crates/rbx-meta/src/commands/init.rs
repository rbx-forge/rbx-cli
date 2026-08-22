use anyhow::{bail, Result};
use colored::Colorize;

use crate::api::models::ApiSocialLink;
use crate::api::RbxClient;
use crate::config::{
    AnimationType, Avatar, AvatarType, CollisionType, Config, Devices, Experience, Game, Genre,
    JointPositioningType, MediaConfig, PaidAccess, PrivateServer, ServerFill, SocialLink,
    SocialLinks, Visibility,
};
use crate::ctx::MetaCtx;
use crate::diff::config_to_lock;
use crate::lockfile::{
    EnvLock, Lockfile, MediaLockfile, DEFAULT_ENV, LOCKFILE_NAME, LOCKFILE_VERSION,
};
use rbx_core::places;

pub async fn run(
    ctx: &MetaCtx<'_>,
    from_remote: bool,
    universe_id_arg: Option<u64>,
    place_id_arg: Option<u64>,
) -> Result<()> {
    if ctx.config.exists() {
        bail!("{} already exists", ctx.config.display());
    }

    if !from_remote {
        std::fs::write(&ctx.config, Config::default_template())?;
        println!(
            "{} {}",
            "Created".green().bold(),
            ctx.config.display().to_string().cyan()
        );
        println!(
            "Edit it, then run `rbx meta sync --env <name>` (resolves IDs via rbxplace.toml) or \
             add [experience] for standalone mode."
        );
        return Ok(());
    }

    // Resolve IDs: explicit --universe-id/--place-id beats --env, beats nothing.
    let (env_name, universe_id, place_id) =
        match (universe_id_arg, place_id_arg, &ctx.env().map(String::from)) {
            (Some(u), Some(p), _) => (DEFAULT_ENV.to_string(), u, p),
            (Some(_), None, _) => bail!("--universe-id requires --place-id"),
            (None, Some(_), _) => bail!("--place-id requires --universe-id"),
            (None, None, Some(env)) => {
                let (u, p) = places::resolve(ctx.places_path(), env, ctx.place())?;
                (env.clone(), u, p)
            }
            (None, None, None) => bail!(
                "--from-remote requires either --env <name> (resolved via rbxplace.toml) or \
             --universe-id + --place-id"
            ),
        };

    let media = MediaConfig::default();
    let cookie = ctx.resolve_cookie();
    let client = RbxClient::new(
        ctx.api_key(),
        cookie.clone(),
        universe_id,
        place_id,
        media.bleed,
        media.language_code.clone(),
    );

    println!("Fetching universe {}...", universe_id);
    let universe = client.get_universe().await?;
    println!("Fetching place {}...", place_id);
    let place = client.get_place().await?;

    let (server_fill, allow_copying) = if client.has_cookie() {
        println!("Fetching legacy place fields (via cookie)...");
        match client.get_place_legacy().await {
            Ok(legacy) => (
                ServerFill::from_legacy(
                    legacy.social_slot_type.as_deref(),
                    legacy.custom_social_slots_count,
                ),
                legacy.copying_allowed,
            ),
            Err(e) => {
                eprintln!(
                    "  warning: legacy place fetch failed ({}). Skipping cookie-only place fields.",
                    e
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // One fetch, several fields. What comes back is what `GET /v1/.../
    // configuration` carries; `permissions` and the avatar scales are not in
    // it and cannot be adopted here: see `api::legacy::UniverseConfigLegacy`.
    let universe_config = match client.get_universe_config_legacy().await {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "  warning: universe config fetch failed ({}). Skipping the cookie-only universe fields.",
                e
            );
            Default::default()
        }
    };
    let studio_access_to_apis_allowed = universe_config.studio_access_to_apis_allowed;
    let avatar = Avatar {
        kind: universe_config
            .universe_avatar_type
            .as_ref()
            .and_then(|v| v.resolve(AvatarType::from_legacy, AvatarType::from_api_name)),
        animation: universe_config
            .universe_animation_type
            .as_ref()
            .and_then(|v| v.resolve(AnimationType::from_legacy, AnimationType::from_api_name)),
        collision: universe_config
            .universe_collision_type
            .as_ref()
            .and_then(|v| v.resolve(CollisionType::from_legacy, CollisionType::from_api_name)),
        joint_positioning: universe_config
            .universe_joint_positioning_type
            .as_ref()
            .and_then(|v| {
                v.resolve(
                    JointPositioningType::from_legacy,
                    JointPositioningType::from_api_name,
                )
            }),
        // Not readable. Left unset rather than defaulted, so the config does
        // not claim a scale range or a slot override nobody chose.
        min_scale: None,
        max_scale: None,
        asset_overrides: None,
    };
    let genre = universe_config
        .genre
        .as_ref()
        .and_then(|v| v.resolve(Genre::from_legacy, Genre::from_api_name));
    let paid_access = match universe_config.is_for_sale {
        Some(true) => universe_config
            .price
            .map(|price| PaidAccess::Paid { price }),
        Some(false) => Some(PaidAccess::Free),
        None => None,
    };

    let beta_mode = if client.has_cookie() {
        match client.get_beta_mode().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "  warning: experience-releases fetch failed ({}). Skipping beta_mode.",
                    e
                );
                None
            }
        }
    } else {
        None
    };

    let visibility = universe
        .visibility
        .as_deref()
        .and_then(Visibility::from_open_cloud);

    let game = Game {
        name: place.display_name.clone().or(universe.display_name.clone()),
        description: place.description.clone().or(universe.description.clone()),
        server_size: place.server_size,
        voice_chat: universe.voice_chat_enabled,
        private_server: universe
            .private_server_price_robux
            .map(|price| PrivateServer { price }),
        devices: Devices {
            desktop: universe.desktop_enabled,
            mobile: universe.mobile_enabled,
            tablet: universe.tablet_enabled,
            console: universe.console_enabled,
            vr: universe.vr_enabled,
        },
        social_links: SocialLinks {
            facebook: universe.facebook_social_link.as_ref().map(to_social),
            twitter: universe.twitter_social_link.as_ref().map(to_social),
            youtube: universe.youtube_social_link.as_ref().map(to_social),
            twitch: universe.twitch_social_link.as_ref().map(to_social),
            discord: universe.discord_social_link.as_ref().map(to_social),
            roblox_group: universe.roblox_group_social_link.as_ref().map(to_social),
            guilded: universe.guilded_social_link.as_ref().map(to_social),
        },
        server_fill,
        allow_copying,
        visibility,
        studio_access_to_apis_allowed,
        beta_mode,
        // Write-only on Roblox's side, so there is nothing to adopt: an
        // `init` that invented a value here would be writing a claim about
        // the experience that it never checked.
        permissions: None,
        avatar,
        paid_access,
        genre,
        // Same reason, and one more: this one names a file, and inventing a
        // path to a file that does not exist would make the very next command
        // fail.
        engine_avatar_settings: None,
    };

    // If standalone mode (no --env), embed [experience] in the toml.
    let experience = if env_name == DEFAULT_ENV {
        Some(Experience {
            universe_id,
            place_id,
        })
    } else {
        None
    };

    let config = Config {
        experience,
        game: game.clone(),
        media,
        envs: Default::default(),
    };
    config.save(&ctx.config)?;

    let mut lockfile = Lockfile {
        version: LOCKFILE_VERSION,
        envs: Default::default(),
    };
    lockfile.envs.insert(
        env_name.clone(),
        EnvLock {
            universe_id,
            place_id,
            game: config_to_lock(&game),
            media: MediaLockfile::default(),
        },
    );
    let lockfile_path = ctx
        .config
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(LOCKFILE_NAME);
    lockfile.save(&lockfile_path)?;

    println!(
        "{} {} and {}",
        "Created".green().bold(),
        ctx.config.display().to_string().cyan(),
        lockfile_path.display().to_string().cyan()
    );
    if env_name == DEFAULT_ENV {
        println!(
            "Standalone mode (no --env). [experience] written to rbxmeta.toml. \
             Add `icon` / `thumbnails` under [media], then `rbx meta sync`."
        );
    } else {
        println!(
            "Env mode: '{}' targeted via rbxplace.toml. Add `icon` / `thumbnails` under [media] \
             (or [envs.{}.media]), then `rbx meta sync --env {}`.",
            env_name, env_name, env_name
        );
    }

    Ok(())
}

fn to_social(link: &ApiSocialLink) -> SocialLink {
    SocialLink {
        title: link.title.clone(),
        url: link.uri.clone(),
    }
}
