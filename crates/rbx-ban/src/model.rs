//! Types for `cloud/v2` user restrictions.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// What Roblox stores for one player in one universe.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRestriction {
    #[serde(default)]
    pub path: Option<String>,
    /// `users/156`.
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub update_time: Option<String>,
    #[serde(default)]
    pub game_join_restriction: Option<GameJoinRestriction>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameJoinRestriction {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub start_time: Option<String>,
    /// Absent means permanent, which is the dangerous default. Never render it
    /// as "unknown" or blank: a reader has to see that nothing will lift it.
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub private_reason: Option<String>,
    #[serde(default)]
    pub display_reason: Option<String>,
    #[serde(default)]
    pub exclude_alt_accounts: Option<bool>,
    #[serde(default)]
    pub inherited: Option<bool>,
}

impl UserRestriction {
    /// The user id this restriction is about, parsed out of `users/156`.
    pub fn user_id(&self) -> Option<u64> {
        self.user.as_deref()?.strip_prefix("users/")?.parse().ok()
    }

    pub fn is_active(&self) -> bool {
        self.game_join_restriction
            .as_ref()
            .is_some_and(|r| r.active)
    }

    /// How long the ban runs, for display. `permanent` when Roblox returned no
    /// duration, because that is what no duration means.
    pub fn duration_label(&self) -> String {
        match self
            .game_join_restriction
            .as_ref()
            .and_then(|r| r.duration.as_deref())
        {
            Some(duration) => humanise_duration(duration),
            None => "permanent".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictionPage {
    #[serde(default)]
    pub user_restrictions: Vec<UserRestriction>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

impl RestrictionPage {
    /// `cloud/v2` ends a listing with an empty string rather than by omitting
    /// the field. Sending `""` back asks for the same page again, forever.
    pub fn next_token(&self) -> Option<&str> {
        self.next_page_token
            .as_deref()
            .filter(|token| !token.is_empty())
    }
}

/// One entry in the restriction audit trail.
///
/// Shape confirmed against a live universe on 2026-08-03, after a ban and an
/// unban produced two entries. It was rendered as raw JSON until then, because
/// nothing documents it and there had never been an entry to look at.
///
/// An unban is an entry with `active: false`, not the absence of one, so the
/// trail reads as a history rather than as a current state.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictionLog {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub create_time: Option<String>,
    /// Absent on an unban, and on a permanent ban.
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub private_reason: Option<String>,
    #[serde(default)]
    pub display_reason: Option<String>,
    #[serde(default)]
    pub exclude_alt_accounts: Option<bool>,
    /// Who did it. `place` is empty for a universe-wide restriction.
    #[serde(default)]
    pub moderator: Option<Moderator>,
    #[serde(default)]
    pub place: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moderator {
    /// `users/1234567890` when a person did it. Absent when Roblox did.
    #[serde(default)]
    pub roblox_user: Option<String>,
}

impl RestrictionLog {
    pub fn user_id(&self) -> Option<u64> {
        self.user.as_deref()?.strip_prefix("users/")?.parse().ok()
    }

    pub fn moderator_id(&self) -> Option<u64> {
        self.moderator
            .as_ref()?
            .roblox_user
            .as_deref()?
            .strip_prefix("users/")?
            .parse()
            .ok()
    }

    /// `banned` or `unbanned`, the two things an entry can record.
    pub fn action(&self) -> &'static str {
        if self.active {
            "banned"
        } else {
            "unbanned"
        }
    }

    pub fn duration_label(&self) -> String {
        match self.duration.as_deref() {
            Some(duration) => humanise_duration(duration),
            // Only meaningful on a ban; an unban has no duration to report.
            None if self.active => "permanent".to_string(),
            None => "-".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    #[serde(default)]
    pub logs: Vec<RestrictionLog>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

impl LogPage {
    pub fn next_token(&self) -> Option<&str> {
        self.next_page_token
            .as_deref()
            .filter(|token| !token.is_empty())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictionUpdate {
    pub game_join_restriction: GameJoinRestrictionUpdate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameJoinRestrictionUpdate {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_alt_accounts: Option<bool>,
}

/// Roblox's limits, from the API schema. Enforced here so an over-long reason
/// fails before the request rather than as a 400 that does not say which field.
pub const MAX_PRIVATE_REASON: usize = 1000;
pub const MAX_DISPLAY_REASON: usize = 400;

/// Parse `7d`, `12h`, `30m`, `2w`, `3600s` into the `"604800s"` Roblox wants.
///
/// The API takes whole seconds with an `s` suffix and rejects sub-second
/// precision, so this is the only accepted output shape. A bare number is read
/// as seconds.
pub fn parse_duration(input: &str) -> Result<String> {
    let input = input.trim().to_ascii_lowercase();
    if input.is_empty() {
        bail!("empty duration");
    }
    if input == "permanent" || input == "forever" {
        bail!("for a permanent restriction, omit --duration entirely");
    }

    let (number, unit) = match input.chars().last() {
        Some(last) if last.is_ascii_digit() => (input.as_str(), 's'),
        Some(last) => (&input[..input.len() - 1], last),
        None => unreachable!("checked non-empty above"),
    };

    let amount: u64 = number
        .parse()
        .map_err(|_| anyhow::anyhow!("`{input}` is not a duration. Try 30m, 12h, 7d, 2w."))?;
    if amount == 0 {
        bail!("a zero-length restriction does nothing; omit --duration for permanent");
    }

    // Checked, not plain multiplication: `999999999999999d` overflows u64,
    // which in a debug build panics and in a release build wraps to a small
    // number. Wrapping would be the worse outcome, quietly turning a typo into
    // a short ban rather than a rejected one.
    let scale = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        'w' => 604_800,
        other => bail!("unknown duration unit `{other}`. Use s, m, h, d or w."),
    };
    let Some(seconds) = amount.checked_mul(scale) else {
        bail!("`{input}` is far longer than Roblox allows (max 315576000000s)");
    };

    // Roblox's documented ceiling, about 10,000 years.
    if seconds > 315_576_000_000 {
        bail!("{seconds}s is longer than Roblox allows (max 315576000000s)");
    }
    Ok(format!("{seconds}s"))
}

/// Turn `604800s` back into `7d` for display.
pub fn humanise_duration(raw: &str) -> String {
    let Some(seconds) = raw.trim_end_matches('s').parse::<u64>().ok() else {
        return raw.to_string();
    };
    for (unit, size) in [("w", 604_800u64), ("d", 86_400), ("h", 3_600), ("m", 60)] {
        if seconds >= size && seconds % size == 0 {
            return format!("{}{unit}", seconds / size);
        }
    }
    format!("{seconds}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_convert_to_whole_seconds() {
        assert_eq!(parse_duration("30m").unwrap(), "1800s");
        assert_eq!(parse_duration("12h").unwrap(), "43200s");
        assert_eq!(parse_duration("7d").unwrap(), "604800s");
        assert_eq!(parse_duration("2w").unwrap(), "1209600s");
        assert_eq!(parse_duration("45s").unwrap(), "45s");
    }

    #[test]
    fn a_bare_number_is_seconds() {
        assert_eq!(parse_duration("3600").unwrap(), "3600s");
    }

    #[test]
    fn permanent_is_expressed_by_omission_not_by_a_word() {
        // Accepting "permanent" would make the most dangerous outcome reachable
        // by a typo in a value, rather than by leaving a flag out.
        let error = parse_duration("permanent").unwrap_err().to_string();
        assert!(error.contains("omit --duration"), "got: {error}");
    }

    #[test]
    fn nonsense_durations_are_rejected() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("0d").is_err());
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("7y").is_err());
        assert!(parse_duration("999999999999999d").is_err());
    }

    #[test]
    fn durations_render_back_in_the_largest_whole_unit() {
        // Not a strict round trip: `7d` and `1w` are the same duration, and
        // display picks the largest unit that divides evenly.
        assert_eq!(humanise_duration("1800s"), "30m");
        assert_eq!(humanise_duration("43200s"), "12h");
        assert_eq!(humanise_duration("604800s"), "1w");
        assert_eq!(humanise_duration("259200s"), "3d");
        assert_eq!(humanise_duration("45s"), "45s");
        // Anything that does not divide evenly stays in seconds rather than
        // being rounded into a number that is not the real ban length.
        assert_eq!(humanise_duration("3661s"), "3661s");
    }

    #[test]
    fn an_unparseable_duration_from_roblox_is_shown_verbatim() {
        assert_eq!(humanise_duration("what"), "what");
    }

    #[test]
    fn a_missing_duration_reads_as_permanent_not_as_blank() {
        let restriction = UserRestriction {
            game_join_restriction: Some(GameJoinRestriction {
                active: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(restriction.duration_label(), "permanent");
    }

    #[test]
    fn the_user_id_is_parsed_out_of_the_resource_path() {
        let restriction = UserRestriction {
            user: Some("users/156".into()),
            ..Default::default()
        };
        assert_eq!(restriction.user_id(), Some(156));
    }

    #[test]
    fn a_malformed_user_path_yields_no_id_rather_than_a_wrong_one() {
        for bad in ["156", "users/", "users/abc", ""] {
            let restriction = UserRestriction {
                user: Some(bad.into()),
                ..Default::default()
            };
            assert_eq!(restriction.user_id(), None, "{bad}");
        }
    }

    #[test]
    fn an_empty_next_page_token_ends_the_listing() {
        let page: RestrictionPage =
            serde_json::from_str(r#"{"userRestrictions":[],"nextPageToken":""}"#).unwrap();
        assert_eq!(page.next_token(), None);
    }

    #[test]
    fn an_inactive_restriction_is_not_a_ban() {
        let page: RestrictionPage = serde_json::from_str(
            r#"{"userRestrictions":[{"user":"users/1","gameJoinRestriction":{"active":false}}]}"#,
        )
        .unwrap();
        assert!(!page.user_restrictions[0].is_active());
    }

    #[test]
    fn optional_update_fields_are_omitted_rather_than_sent_as_null() {
        // updateMask plus a null would clear a field. Omitting leaves it alone.
        let update = RestrictionUpdate {
            game_join_restriction: GameJoinRestrictionUpdate {
                active: false,
                duration: None,
                private_reason: None,
                display_reason: None,
                exclude_alt_accounts: None,
            },
        };
        let json = serde_json::to_string(&update).unwrap();
        assert_eq!(json, r#"{"gameJoinRestriction":{"active":false}}"#);
    }
}
