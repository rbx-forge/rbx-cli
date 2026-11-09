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
use crate::config::{SocialLink, Visibility};
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
    let mut c = Comparison::default();
    let universe = &remote.universe;

    c.recorded("name", &lock.name, remote.name());
    c.recorded("description", &lock.description, remote.description());
    c.recorded("server_size", &lock.server_size, remote.place.server_size);
    c.recorded("voice_chat", &lock.voice_chat, universe.voice_chat_enabled);
    c.recorded(
        "devices.desktop",
        &lock.devices.desktop,
        universe.desktop_enabled,
    );
    c.recorded(
        "devices.mobile",
        &lock.devices.mobile,
        universe.mobile_enabled,
    );
    c.recorded(
        "devices.tablet",
        &lock.devices.tablet,
        universe.tablet_enabled,
    );
    c.recorded(
        "devices.console",
        &lock.devices.console,
        universe.console_enabled,
    );
    c.recorded("devices.vr", &lock.devices.vr, universe.vr_enabled);

    // Absence is a value for these two: no table means no private servers,
    // and a platform with no link has none on Roblox either.
    c.exact(
        "private_server",
        &lock.private_server,
        &remote.private_server(),
    );
    let links = &lock.social_links;
    for (platform, locked, live) in [
        ("facebook", &links.facebook, &universe.facebook_social_link),
        ("twitter", &links.twitter, &universe.twitter_social_link),
        ("youtube", &links.youtube, &universe.youtube_social_link),
        ("twitch", &links.twitch, &universe.twitch_social_link),
        ("discord", &links.discord, &universe.discord_social_link),
        (
            "roblox_group",
            &links.roblox_group,
            &universe.roblox_group_social_link,
        ),
        ("guilded", &links.guilded, &universe.guilded_social_link),
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
    let (visibility, audience) = remote.visibility_and_audience();
    match visibility {
        Some(live) => {
            c.recorded("visibility", &lock.visibility, Some(live));
            if live == Visibility::Limited && lock.visibility == Some(live) {
                c.recorded("audience", &lock.audience, audience);
            }
        }
        None if lock.visibility.is_some() => c.unconfirmed.push("visibility"),
        None => {}
    }

    if has_cookie {
        let read = remote.confirmed();
        c.recorded("allow_copying", &lock.allow_copying, read.allow_copying);
        c.recorded("server_fill", &lock.server_fill, read.server_fill);
        c.recorded(
            "studio_access_to_apis_allowed",
            &lock.studio_access_to_apis_allowed,
            read.studio_access_to_apis_allowed,
        );
        c.recorded("beta_mode", &lock.beta_mode, read.beta_mode);
        c.recorded("genre", &lock.genre, read.genre);
        c.recorded("paid_access", &lock.paid_access, read.paid_access);
        c.recorded("avatar.type", &lock.avatar.kind, read.avatar_kind);
        c.recorded(
            "avatar.animation",
            &lock.avatar.animation,
            read.avatar_animation,
        );
        c.recorded(
            "avatar.collision",
            &lock.avatar.collision,
            read.avatar_collision,
        );
        c.recorded(
            "avatar.joint_positioning",
            &lock.avatar.joint_positioning,
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

fn spell<T: Serialize + Debug>(value: &Option<T>) -> String {
    match value {
        Some(v) => toml_spelling(v),
        None => "none".to_string(),
    }
}

fn spell_link(link: &Option<SocialLink>) -> String {
    match link {
        Some(link) => format!("{:?} <{}>", link.title, link.url),
        None => "none".to_string(),
    }
}

#[cfg(test)]
mod tests;
