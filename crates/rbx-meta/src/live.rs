//! `meta/live`: what Roblox has, against what the lockfile last recorded.
//!
//! Against the lockfile, not the config. A change written in `rbxmeta.toml`
//! and not synced yet is `meta/lockfile`'s to report; comparing the config here
//! would report it a second time, as if Roblox had drifted. The lockfile holds
//! what the last `sync` or `pull` confirmed Roblox had, so a difference here is
//! a change made on Roblox's side since then.
//!
//! The reads are `pull`'s own, through `read_remote`, so this cannot disagree
//! with `pull` about what Roblox said.

use std::fmt::Debug;

use anyhow::Result;
use serde::Serialize;

use crate::api::RbxClient;
use crate::commands::pull::{read_remote, toml_spelling, RemoteRead};
use crate::config::{Avatar, Devices, SocialLink, SocialLinks, Visibility};
use crate::lockfile::{EnvLock, GameLock};

/// What one env's comparison found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LiveReport {
    /// One line per field where Roblox and the lockfile disagree.
    pub differences: Vec<String>,
    /// What this run could not compare, one line per reason. A clean run is
    /// clean only for the fields not listed here.
    pub not_compared: Vec<String>,
}

/// Read only with the session cookie.
const COOKIE_FIELDS: &[&str] = &[
    "allow_copying",
    "server_fill",
    "studio_access_to_apis_allowed",
    "beta_mode",
    "genre",
    "paid_access",
    "avatar.type",
    "avatar.animation",
    "avatar.collision",
    "avatar.joint_positioning",
];

/// Returned by no endpoint, so there is nothing to compare them with.
const WRITE_ONLY: &[&str] = &[
    "permissions",
    "avatar.min_scale",
    "avatar.max_scale",
    "avatar.asset_overrides",
    "engine_avatar_settings",
];

/// Compare one env's lock section with what Roblox has now.
///
/// The ids come from the lock section, which is what it describes.
pub async fn compare(
    api_key: String,
    cookie: Option<String>,
    env_lock: &EnvLock,
) -> Result<LiveReport> {
    let client = RbxClient::new(
        Some(api_key),
        cookie,
        env_lock.universe_id,
        env_lock.place_id,
        true,
        None,
    );
    let remote = read_remote(&client).await?;
    Ok(compare_read(&env_lock.game, &remote, client.has_cookie()))
}

/// The comparison itself, apart from the network.
pub(crate) fn compare_read(lock: &GameLock, remote: &RemoteRead, has_cookie: bool) -> LiveReport {
    // Destructured without `..`: a field added to `GameLock`, or to a table
    // inside it, does not compile here until it is compared below or set
    // aside with its reason.
    let GameLock {
        name,
        description,
        server_size,
        voice_chat,
        allow_copying,
        visibility,
        audience,
        studio_access_to_apis_allowed,
        beta_mode,
        private_server,
        devices:
            Devices {
                desktop,
                mobile,
                tablet,
                console,
                vr,
            },
        social_links:
            SocialLinks {
                facebook,
                twitter,
                youtube,
                twitch,
                discord,
                roblox_group,
                guilded,
            },
        server_fill,
        // Write-only: returned by no endpoint, listed in `WRITE_ONLY`.
        permissions: _,
        avatar:
            Avatar {
                kind,
                animation,
                collision,
                joint_positioning,
                min_scale: _,
                max_scale: _,
                asset_overrides: _,
            },
        paid_access,
        genre,
        engine_avatar_settings_hash: _,
    } = lock;
    let mut c = Comparison::default();
    let universe = &remote.universe;

    c.recorded("name", name, remote.name());
    c.recorded("description", description, remote.description());
    c.recorded("server_size", server_size, remote.place.server_size);
    c.recorded("voice_chat", voice_chat, universe.voice_chat_enabled);
    c.recorded("devices.desktop", desktop, universe.desktop_enabled);
    c.recorded("devices.mobile", mobile, universe.mobile_enabled);
    c.recorded("devices.tablet", tablet, universe.tablet_enabled);
    c.recorded("devices.console", console, universe.console_enabled);
    c.recorded("devices.vr", vr, universe.vr_enabled);

    // Absence is a value for these two: no table means no private servers,
    // and a platform with no link has none on Roblox either.
    c.exact("private_server", private_server, &remote.private_server());
    for (platform, locked, live) in [
        ("facebook", facebook, &universe.facebook_social_link),
        ("twitter", twitter, &universe.twitter_social_link),
        ("youtube", youtube, &universe.youtube_social_link),
        ("twitch", twitch, &universe.twitch_social_link),
        ("discord", discord, &universe.discord_social_link),
        (
            "roblox_group",
            roblox_group,
            &universe.roblox_group_social_link,
        ),
        ("guilded", guilded, &universe.guilded_social_link),
    ] {
        let live = live.as_ref().map(SocialLink::from);
        if locked != &live {
            c.differences.push(format!(
                "social_links.{platform}: lockfile {}, Roblox {}",
                spell_link(locked),
                spell_link(&live)
            ));
        }
    }

    // Read anonymously, so a key and no cookie still compares it.
    let (live_visibility, live_audience) = remote.visibility_and_audience();
    match live_visibility {
        Some(live) => {
            c.recorded("visibility", visibility, Some(live));
            if live == Visibility::Limited && *visibility == Some(live) {
                c.recorded("audience", audience, live_audience);
            }
        }
        None if visibility.is_some() => c.unconfirmed.push("visibility"),
        None => {}
    }

    if has_cookie {
        let read = remote.confirmed();
        c.recorded("allow_copying", allow_copying, read.allow_copying);
        c.recorded("server_fill", server_fill, read.server_fill);
        c.recorded(
            "studio_access_to_apis_allowed",
            studio_access_to_apis_allowed,
            read.studio_access_to_apis_allowed,
        );
        c.recorded("beta_mode", beta_mode, read.beta_mode);
        c.recorded("genre", genre, read.genre);
        c.recorded("paid_access", paid_access, read.paid_access);
        c.recorded("avatar.type", kind, read.avatar_kind);
        c.recorded("avatar.animation", animation, read.avatar_animation);
        c.recorded("avatar.collision", collision, read.avatar_collision);
        c.recorded(
            "avatar.joint_positioning",
            joint_positioning,
            read.avatar_joint_positioning,
        );
    }

    let mut not_compared = Vec::new();
    if !c.unconfirmed.is_empty() {
        not_compared.push(format!(
            "not confirmed by Roblox: {}",
            c.unconfirmed.join(", ")
        ));
    }
    if !has_cookie {
        not_compared.push(format!(
            "needs the Studio cookie: {}",
            COOKIE_FIELDS.join(", ")
        ));
    }
    not_compared.push(format!(
        "never returned by Roblox: {}",
        WRITE_ONLY.join(", ")
    ));
    not_compared.push("media: icon and thumbnails".to_string());

    LiveReport {
        differences: c.differences,
        not_compared,
    }
}

#[derive(Default)]
struct Comparison {
    differences: Vec<String>,
    unconfirmed: Vec<&'static str>,
}

impl Comparison {
    /// A field the lockfile records only when the config manages it. Unset in
    /// the lock means unmanaged, so there is nothing to hold Roblox to.
    fn recorded<T: PartialEq + Serialize + Debug>(
        &mut self,
        field: &'static str,
        locked: &Option<T>,
        live: Option<T>,
    ) {
        let Some(locked) = locked else { return };
        match live {
            None => self.unconfirmed.push(field),
            Some(live) if &live != locked => self.differences.push(format!(
                "{field}: lockfile {}, Roblox {}",
                toml_spelling(locked),
                toml_spelling(&live)
            )),
            Some(_) => {}
        }
    }

    /// A field whose absence is itself a value.
    fn exact<T: PartialEq + Serialize + Debug>(
        &mut self,
        field: &'static str,
        locked: &Option<T>,
        live: &Option<T>,
    ) {
        if locked != live {
            self.differences.push(format!(
                "{field}: lockfile {}, Roblox {}",
                spell(locked),
                spell(live)
            ));
        }
    }
}

pub(crate) fn spell<T: Serialize + Debug>(value: &Option<T>) -> String {
    match value {
        Some(v) => toml_spelling(v),
        None => "none".to_string(),
    }
}

pub(crate) fn spell_link(link: &Option<SocialLink>) -> String {
    match link {
        Some(link) => format!("{:?} <{}>", link.title, link.url),
        None => "none".to_string(),
    }
}

#[cfg(test)]
mod tests;
