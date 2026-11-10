//! What `sync` plans against: the lock section, brought up to date with Roblox.
//!
//! Three sides. The config says what is wanted, the lockfile what this
//! repository last saw Roblox hold, and the read what Roblox holds now. `sync`
//! writes the difference between the first and the third, so a value changed
//! on the Creator Hub is seen, and not reported as in sync because the two
//! local files agree.
//!
//! The lockfile is what tells a change made here from a change made there. A
//! field Roblox holds differently from what the lockfile recorded was changed
//! on Roblox, and a plan that would write over it is refused unless asked for:
//! see [`Refreshed::newer`].
//!
//! It is also what says a field is managed at all. That matters for the two
//! settings whose absence is a value, private servers and social links: one
//! added on the Creator Hub that neither the config nor the lockfile ever
//! mentioned is left alone, where comparing the config with Roblox alone would
//! plan to remove it.

use std::collections::BTreeSet;
use std::fmt::Debug;

use serde::Serialize;

use crate::api::models::ApiSocialLink;
use crate::commands::pull::{toml_spelling, RemoteRead};
use crate::config::{Access, Audience, Avatar, Devices, Game, SocialLink, SocialLinks, Visibility};
use crate::live::{spell, spell_link};
use crate::lockfile::GameLock;

/// One env's lock section as Roblox stands now.
#[derive(Debug, PartialEq)]
pub(crate) struct Refreshed {
    /// What `build_plan` compares the config with, and what the lockfile
    /// records once the run is allowed to write.
    pub lock: GameLock,
    /// Fields changed on Roblox since the lockfile recorded them, which the
    /// config would write over. One line each, spelled as `meta/live` spells a
    /// difference.
    pub newer: Vec<String>,
}

/// Bring one env's lock section up to date with what Roblox returned.
///
/// The rule, per field:
///
/// - unset in both the config and the lockfile: not managed, left unrecorded;
/// - not confirmed by Roblox (the read failed, needed a cookie that was not
///   given, or no endpoint returns the field): the lockfile stands;
/// - otherwise: what Roblox has.
pub(crate) fn refresh(game: &Game, lock: &GameLock, remote: &RemoteRead) -> Refreshed {
    // Destructured without `..`, as in `live::compare_read`: a field added to
    // `GameLock`, or to a table inside it, does not compile here until it is
    // given a rule below.
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
        permissions,
        avatar:
            Avatar {
                kind,
                animation,
                collision,
                joint_positioning,
                min_scale,
                max_scale,
                asset_overrides,
            },
        paid_access,
        genre,
        engine_avatar_settings_hash,
    } = lock;
    let universe = &remote.universe;
    // `None` here is the unconfirmed case, whatever caused it, and the lockfile
    // then stands. That is every cookie-only field when no cookie was given,
    // so no branch on the cookie is needed. `access` is read anonymously, and
    // is `None` only when that read failed.
    let read = remote.confirmed();
    let mut walk = Walk::default();

    let (visibility, audience) = walk.access(game.access(), *visibility, audience, read.access);

    let lock = GameLock {
        name: walk.recorded("name", game.name.as_ref(), name, remote.name()),
        description: walk.recorded(
            "description",
            game.description.as_ref(),
            description,
            remote.description(),
        ),
        server_size: walk.recorded(
            "server_size",
            game.server_size.as_ref(),
            server_size,
            remote.place.server_size,
        ),
        voice_chat: walk.recorded(
            "voice_chat",
            game.voice_chat.as_ref(),
            voice_chat,
            universe.voice_chat_enabled,
        ),
        allow_copying: walk.recorded(
            "allow_copying",
            game.allow_copying.as_ref(),
            allow_copying,
            read.allow_copying,
        ),
        visibility,
        audience,
        studio_access_to_apis_allowed: walk.recorded(
            "studio_access_to_apis_allowed",
            game.studio_access_to_apis_allowed.as_ref(),
            studio_access_to_apis_allowed,
            read.studio_access_to_apis_allowed,
        ),
        beta_mode: walk.recorded(
            "beta_mode",
            game.beta_mode.as_ref(),
            beta_mode,
            read.beta_mode,
        ),
        private_server: walk.exact(
            "private_server",
            game.private_server.as_ref(),
            private_server,
            remote.private_server(),
            spell,
        ),
        devices: Devices {
            desktop: walk.recorded(
                "devices.desktop",
                game.devices.desktop.as_ref(),
                desktop,
                universe.desktop_enabled,
            ),
            mobile: walk.recorded(
                "devices.mobile",
                game.devices.mobile.as_ref(),
                mobile,
                universe.mobile_enabled,
            ),
            tablet: walk.recorded(
                "devices.tablet",
                game.devices.tablet.as_ref(),
                tablet,
                universe.tablet_enabled,
            ),
            console: walk.recorded(
                "devices.console",
                game.devices.console.as_ref(),
                console,
                universe.console_enabled,
            ),
            vr: walk.recorded(
                "devices.vr",
                game.devices.vr.as_ref(),
                vr,
                universe.vr_enabled,
            ),
        },
        social_links: SocialLinks {
            facebook: walk.link(
                "facebook",
                &game.social_links.facebook,
                facebook,
                &universe.facebook_social_link,
            ),
            twitter: walk.link(
                "twitter",
                &game.social_links.twitter,
                twitter,
                &universe.twitter_social_link,
            ),
            youtube: walk.link(
                "youtube",
                &game.social_links.youtube,
                youtube,
                &universe.youtube_social_link,
            ),
            twitch: walk.link(
                "twitch",
                &game.social_links.twitch,
                twitch,
                &universe.twitch_social_link,
            ),
            discord: walk.link(
                "discord",
                &game.social_links.discord,
                discord,
                &universe.discord_social_link,
            ),
            roblox_group: walk.link(
                "roblox_group",
                &game.social_links.roblox_group,
                roblox_group,
                &universe.roblox_group_social_link,
            ),
            guilded: walk.link(
                "guilded",
                &game.social_links.guilded,
                guilded,
                &universe.guilded_social_link,
            ),
        },
        server_fill: walk.recorded(
            "server_fill",
            game.server_fill.as_ref(),
            server_fill,
            read.server_fill,
        ),
        // Returned by no endpoint: only the lockfile can speak for them.
        permissions: *permissions,
        avatar: Avatar {
            kind: walk.recorded(
                "avatar.type",
                game.avatar.kind.as_ref(),
                kind,
                read.avatar_kind,
            ),
            animation: walk.recorded(
                "avatar.animation",
                game.avatar.animation.as_ref(),
                animation,
                read.avatar_animation,
            ),
            collision: walk.recorded(
                "avatar.collision",
                game.avatar.collision.as_ref(),
                collision,
                read.avatar_collision,
            ),
            joint_positioning: walk.recorded(
                "avatar.joint_positioning",
                game.avatar.joint_positioning.as_ref(),
                joint_positioning,
                read.avatar_joint_positioning,
            ),
            min_scale: *min_scale,
            max_scale: *max_scale,
            asset_overrides: *asset_overrides,
        },
        paid_access: walk.recorded(
            "paid_access",
            game.paid_access.as_ref(),
            paid_access,
            read.paid_access,
        ),
        genre: walk.recorded("genre", game.genre.as_ref(), genre, read.genre),
        engine_avatar_settings_hash: engine_avatar_settings_hash.clone(),
    };

    Refreshed {
        lock,
        newer: walk.newer,
    }
}

#[derive(Default)]
struct Walk {
    newer: Vec<String>,
}

impl Walk {
    /// A field Roblox may not have confirmed, where `None` from the read means
    /// exactly that.
    fn recorded<T: Clone + PartialEq + Serialize + Debug>(
        &mut self,
        field: &str,
        wanted: Option<&T>,
        locked: &Option<T>,
        live: Option<T>,
    ) -> Option<T> {
        if wanted.is_none() && locked.is_none() {
            return None;
        }
        let Some(live) = live else {
            return locked.clone();
        };
        if let Some(locked) = locked {
            if *locked != live && wanted.is_some_and(|wanted| *wanted != live) {
                self.newer.push(format!(
                    "{field}: lockfile {}, Roblox {}",
                    toml_spelling(locked),
                    toml_spelling(&live)
                ));
            }
        }
        Some(live)
    }

    /// A field whose absence is itself a value, so Roblox's answer is always
    /// known: no table means no private servers, and a platform with no link
    /// has none on Roblox either.
    ///
    /// A config that stopped declaring it writes too, as a removal, so the
    /// write that could replace a newer value is any difference between the
    /// config and Roblox.
    fn exact<T: Clone + PartialEq>(
        &mut self,
        field: &str,
        wanted: Option<&T>,
        locked: &Option<T>,
        live: Option<T>,
        spell: impl Fn(&Option<T>) -> String,
    ) -> Option<T> {
        if wanted.is_none() && locked.is_none() {
            return None;
        }
        if locked.is_some() && *locked != live && wanted != live.as_ref() {
            self.newer.push(format!(
                "{field}: lockfile {}, Roblox {}",
                spell(locked),
                spell(&live)
            ));
        }
        live
    }

    fn link(
        &mut self,
        platform: &str,
        wanted: &Option<SocialLink>,
        locked: &Option<SocialLink>,
        live: &Option<ApiSocialLink>,
    ) -> Option<SocialLink> {
        self.exact(
            &format!("social_links.{platform}"),
            wanted.as_ref(),
            locked,
            live.as_ref().map(SocialLink::from),
            spell_link,
        )
    }

    /// `visibility` and `audience`, compared as the one setting they are, the
    /// way `build_visibility_change` compares them. Unconfirmed, the two keys
    /// come back exactly as the lockfile spells them.
    fn access(
        &mut self,
        wanted: Option<Access>,
        locked_visibility: Option<Visibility>,
        locked_audience: &Option<BTreeSet<Audience>>,
        live: Option<Access>,
    ) -> (Option<Visibility>, Option<BTreeSet<Audience>>) {
        let locked = Access::from_parts(locked_visibility, locked_audience.as_ref());
        if wanted.is_none() && locked.is_none() {
            return (None, None);
        }
        let Some(live) = live else {
            return (locked_visibility, locked_audience.clone());
        };
        if let Some(locked) = &locked {
            if *locked != live && wanted.as_ref().is_some_and(|wanted| *wanted != live) {
                self.newer
                    .push(format!("visibility: lockfile {locked}, Roblox {live}"));
            }
        }
        let (visibility, audience) = live.into_parts();
        (Some(visibility), audience)
    }
}

#[cfg(test)]
mod tests;
