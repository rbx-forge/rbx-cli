//! One env's read: everything Roblox is asked for, and the shape it comes back in.

use anyhow::Result;
use colored::Colorize;

use crate::api::models::ApiSocialLink;
use crate::api::RbxClient;
use crate::config::{
    AnimationType, AvatarType, CollisionType, Config, Genre, JointPositioningType, PaidAccess,
    PrivateServer, ServerFill, SocialLink, Visibility,
};
use crate::ctx::MetaCtx;
use crate::lockfile::Lockfile;

use super::{differential::*, write::*};

pub(super) async fn fetch_env(
    ctx: &MetaCtx<'_>,
    config: &mut Config,
    lockfile: &Lockfile,
    env_name: &str,
    universe_id: u64,
    place_id: u64,
) -> Result<(RbxClient, ConfirmedReads)> {
    // The place half of the guard `sync` applies. The differential pull
    // compares remote against this section, so a section describing a
    // different place makes it write overlays for what are really two
    // different places. `Repoint::Universe` because re-pointing an env at a
    // new universe and pulling is how you adopt it.
    crate::commands::ensure_not_repointed(
        env_name,
        &lockfile.env_view(env_name),
        universe_id,
        place_id,
        crate::commands::Repoint::Universe,
    )?;

    let cookie = ctx.resolve_cookie();
    let client = RbxClient::new(
        ctx.api_key(),
        cookie,
        universe_id,
        place_id,
        config.media.bleed,
        config.media.language_code.clone(),
    );

    // -----------------------------------------------------------------------
    // Fetch remote
    // -----------------------------------------------------------------------
    println!(
        "Pulling env '{}' (universe {} / place {})...",
        env_name.cyan(),
        universe_id,
        place_id
    );
    let universe = client.get_universe().await?;
    let place = client.get_place().await?;

    let (remote_server_fill, remote_allow_copying) = if client.has_cookie() {
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
                    "warning: legacy place fetch failed ({}). Skipping cookie-only place fields.",
                    e
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // One read, several fields. `permissions` and the avatar scale tables are
    // deliberately absent from it: Roblox exposes no GET that returns them,
    // so they are write-only and `pull` leaves whatever the config says.
    // A failure warns and yields an empty configuration, which is exactly the
    // right shape: every field then resolves to `None`, meaning "not
    // confirmed", and `reconcile_lock` keeps the previous lock entry rather
    // than recording what the config asked for. No separate flag is needed:
    // the absence *is* the signal, and it covers a value Roblox sent that this
    // build does not recognise just as well as a call that never happened.
    let universe_config = match client.get_universe_config_legacy().await {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "warning: universe config fetch failed ({}). Skipping the cookie-only universe fields.",
                e
            );
            Default::default()
        }
    };
    let remote_studio_access = universe_config.studio_access_to_apis_allowed;

    // `None` on failure, not the lockfile's value.
    //
    // Feeding the lock back in here looked like a graceful fallback and was
    // not: `diff_apply_opt` writes whatever it is given into the *config* and
    // lists it under "Config updates", so a user who edited `beta_mode` and
    // pulled while this endpoint was down had their edit silently replaced by
    // the old value and reported as something Roblox had said. `None` means
    // "not confirmed", `diff_apply_opt` then leaves the config alone, and
    // `reconcile_lock` carries the previous value into the lock where it
    // belongs.
    let remote_beta_mode = if client.has_cookie() {
        match client.get_beta_mode().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "warning: experience-releases fetch failed ({}). Skipping beta_mode.",
                    e
                );
                None
            }
        }
    } else {
        None
    };

    let remote_visibility = universe
        .visibility
        .as_deref()
        .and_then(Visibility::from_open_cloud);

    // Everything that came from an endpoint allowed to fail, gathered before
    // the differential apply mutates the config. `reconcile_lock` needs the
    // *confirmed* values, and after `diff_apply_opt` has run the config no
    // longer distinguishes what was read from what was already written.
    let confirmed = ConfirmedReads {
        allow_copying: remote_allow_copying,
        server_fill: remote_server_fill.clone(),
        studio_access_to_apis_allowed: remote_studio_access,
        beta_mode: remote_beta_mode,
        genre: universe_config
            .genre
            .as_ref()
            .and_then(|v| v.resolve(Genre::from_legacy, Genre::from_api_name)),
        paid_access: match universe_config.is_for_sale {
            Some(true) => universe_config
                .price
                .map(|price| PaidAccess::Paid { price }),
            Some(false) => Some(PaidAccess::Free),
            None => None,
        },
        avatar_kind: universe_config
            .universe_avatar_type
            .as_ref()
            .and_then(|v| v.resolve(AvatarType::from_legacy, AvatarType::from_api_name)),
        avatar_animation: universe_config
            .universe_animation_type
            .as_ref()
            .and_then(|v| v.resolve(AnimationType::from_legacy, AnimationType::from_api_name)),
        avatar_collision: universe_config
            .universe_collision_type
            .as_ref()
            .and_then(|v| v.resolve(CollisionType::from_legacy, CollisionType::from_api_name)),
        avatar_joint_positioning: universe_config
            .universe_joint_positioning_type
            .as_ref()
            .and_then(|v| {
                v.resolve(
                    JointPositioningType::from_legacy,
                    JointPositioningType::from_api_name,
                )
            }),
    };

    // -----------------------------------------------------------------------
    // Differential apply on the in-memory config
    // -----------------------------------------------------------------------
    let mut changes: Vec<String> = Vec::new();
    let overlay = config.envs.entry(env_name.to_string()).or_default();

    // Scalars
    diff_apply_opt(
        &mut config.game.name,
        &mut overlay.name,
        place.display_name.clone().or(universe.display_name.clone()),
        "name",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.description,
        &mut overlay.description,
        place.description.clone().or(universe.description.clone()),
        "description",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.server_size,
        &mut overlay.server_size,
        place.server_size,
        "server_size",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.voice_chat,
        &mut overlay.voice_chat,
        universe.voice_chat_enabled,
        "voice_chat",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.allow_copying,
        &mut overlay.allow_copying,
        remote_allow_copying,
        "allow_copying",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.visibility,
        &mut overlay.visibility,
        remote_visibility,
        "visibility",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.studio_access_to_apis_allowed,
        &mut overlay.studio_access_to_apis_allowed,
        remote_studio_access,
        "studio_access_to_apis_allowed",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.beta_mode,
        &mut overlay.beta_mode,
        remote_beta_mode,
        "beta_mode",
        &mut changes,
    );

    // Avatar, genre and paid access come from the same v1 configuration read.
    //
    // Not `permissions`, and not the avatar scales: Roblox returns neither
    // from any GET, so there is nothing to adopt and inventing a value would
    // put a claim in the config that was never checked against the
    // experience. See `api::legacy::UniverseConfigLegacy`.
    diff_apply_opt(
        &mut config.game.avatar.kind,
        &mut overlay.avatar.kind,
        universe_config
            .universe_avatar_type
            .as_ref()
            .and_then(|v| v.resolve(AvatarType::from_legacy, AvatarType::from_api_name)),
        "avatar.type",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.avatar.animation,
        &mut overlay.avatar.animation,
        universe_config
            .universe_animation_type
            .as_ref()
            .and_then(|v| v.resolve(AnimationType::from_legacy, AnimationType::from_api_name)),
        "avatar.animation",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.avatar.collision,
        &mut overlay.avatar.collision,
        universe_config
            .universe_collision_type
            .as_ref()
            .and_then(|v| v.resolve(CollisionType::from_legacy, CollisionType::from_api_name)),
        "avatar.collision",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.avatar.joint_positioning,
        &mut overlay.avatar.joint_positioning,
        universe_config
            .universe_joint_positioning_type
            .as_ref()
            .and_then(|v| {
                v.resolve(
                    JointPositioningType::from_legacy,
                    JointPositioningType::from_api_name,
                )
            }),
        "avatar.joint_positioning",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.genre,
        &mut overlay.genre,
        universe_config
            .genre
            .as_ref()
            .and_then(|v| v.resolve(Genre::from_legacy, Genre::from_api_name)),
        "genre",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.paid_access,
        &mut overlay.paid_access,
        match universe_config.is_for_sale {
            Some(true) => universe_config
                .price
                .map(|price| PaidAccess::Paid { price }),
            Some(false) => Some(PaidAccess::Free),
            None => None,
        },
        "paid_access",
        &mut changes,
    );

    // private_server (atomic)
    let remote_private_server = universe
        .private_server_price_robux
        .map(|price| PrivateServer { price });
    diff_apply_opt(
        &mut config.game.private_server,
        &mut overlay.private_server,
        remote_private_server,
        "private_server",
        &mut changes,
    );

    // server_fill (atomic)
    diff_apply_opt(
        &mut config.game.server_fill,
        &mut overlay.server_fill,
        remote_server_fill,
        "server_fill",
        &mut changes,
    );

    // devices (per-field)
    diff_apply_opt(
        &mut config.game.devices.desktop,
        &mut overlay.devices.desktop,
        universe.desktop_enabled,
        "devices.desktop",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.mobile,
        &mut overlay.devices.mobile,
        universe.mobile_enabled,
        "devices.mobile",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.tablet,
        &mut overlay.devices.tablet,
        universe.tablet_enabled,
        "devices.tablet",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.console,
        &mut overlay.devices.console,
        universe.console_enabled,
        "devices.console",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.vr,
        &mut overlay.devices.vr,
        universe.vr_enabled,
        "devices.vr",
        &mut changes,
    );

    // social links (per platform, atomic)
    diff_apply_social(
        &mut config.game.social_links.facebook,
        &mut overlay.social_links.facebook,
        universe.facebook_social_link.as_ref().map(to_social),
        "facebook",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.twitter,
        &mut overlay.social_links.twitter,
        universe.twitter_social_link.as_ref().map(to_social),
        "twitter",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.youtube,
        &mut overlay.social_links.youtube,
        universe.youtube_social_link.as_ref().map(to_social),
        "youtube",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.twitch,
        &mut overlay.social_links.twitch,
        universe.twitch_social_link.as_ref().map(to_social),
        "twitch",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.discord,
        &mut overlay.social_links.discord,
        universe.discord_social_link.as_ref().map(to_social),
        "discord",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.roblox_group,
        &mut overlay.social_links.roblox_group,
        universe.roblox_group_social_link.as_ref().map(to_social),
        "roblox_group",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.guilded,
        &mut overlay.social_links.guilded,
        universe.guilded_social_link.as_ref().map(to_social),
        "guilded",
        &mut changes,
    );

    // Print scalar diff
    if changes.is_empty() {
        println!("\n{}", "✓ Scalar fields already match remote.".green());
    } else {
        println!("\n{}", "Config updates:".bold());
        for c in &changes {
            println!("  • {}", c);
        }
    }

    Ok((client, confirmed))
}

pub(super) fn to_social(link: &ApiSocialLink) -> SocialLink {
    SocialLink {
        title: link.title.clone(),
        url: link.uri.clone(),
    }
}
