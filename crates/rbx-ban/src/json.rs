//! What `rbx ban list --json` and `rbx ban status --json` write to stdout.
//!
//! Separate from `model` on purpose. `model` describes what Roblox sends, down
//! to the camelCase `gameJoinRestriction` and the `users/156` resource path;
//! this describes what we promise. A field renamed upstream is a parsing change
//! there, not a break in somebody's `jq` filter.
//!
//! The envelope follows `rbx check --json`: a `schema_version` first, then
//! named objects all the way down, optional fields omitted rather than emitted
//! as `null`, ids as strings. Field names are documented in `docs/ops/ban.md`
//! and are the compatibility surface.
//!
//! ## Absence is the harshest answer, so it is also stated
//!
//! Roblox expresses a permanent restriction by sending no `duration` at all.
//! That is the one place where a missing field means the worst outcome rather
//! than "nothing to report", and a consumer that reads `.duration // "none"`
//! would report a permanent ban as no ban. So `permanent` is always present and
//! always says which of the two it is, and `duration`: the raw `604800s`
//! Roblox sent, not the `7d` the table renders: is absent exactly when
//! `permanent` is true. The human form takes the same care: `duration_label`
//! prints the word `permanent` rather than a blank.
//!
//! ## What is deliberately absent
//!
//! These commands are about real players being locked out of a real game, so a
//! document says no more than the human form already says out loud.
//!
//! `ban list --json` carries no `display_reason` and no `start_time`: the
//! listing prints neither, and the text a banned player is shown is not a field
//! a monitoring script asked for. `ban status --json` carries both, because
//! that is the command that prints them, under `player sees` and `since`.
//!
//! `inherited` and `exclude_alt_accounts` are on every restriction Roblox
//! returns and are in neither document. Nothing in `ban list` or `ban status`
//! has ever printed them, so nothing promises them. `path` and `update_time`
//! are absent for the duller version of the same reason.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;
use rbx_core::users::User;

use crate::model::UserRestriction;

/// One `ban list` invocation.
#[derive(Debug, Serialize)]
pub struct ListDocument {
    pub schema_version: u32,
    /// The env that named the universe. **Absent** under a bare
    /// `--universe-id`, where no env was resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    pub universe_id: String,
    /// Whether entries that exist but are not active were included.
    pub include_inactive: bool,
    /// The `--limit` in force, which is a maximum and not a promise.
    pub limit: u32,
    /// True when the walk stopped because it hit `--limit` rather than because
    /// the experience ran out of entries. Raise `--limit` to see the rest.
    pub limit_reached: bool,
    /// Rows in `restrictions`.
    pub count: usize,
    /// One object per row, in the order Roblox returned them: the order the
    /// human table prints.
    pub restrictions: Vec<Restriction>,
}

/// One restricted player, as the listing has them.
///
/// No name: this endpoint does not send one, which is the same reason the human
/// table prints ids and tells you to run `ban status` on a row you care about.
#[derive(Debug, Serialize)]
pub struct Restriction {
    /// **Absent** when the id could not be read out of the resource path, which
    /// the human table renders as `?`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// False for an entry that exists without currently locking anybody out.
    /// Only `--include-inactive` puts one of those in the listing, and without
    /// this field the two kinds of row are indistinguishable.
    pub active: bool,
    /// True when nothing will lift this restriction. See the module docs: this
    /// is stated rather than left to an absent `duration`.
    pub permanent: bool,
    /// How long, as Roblox sent it: `604800s`. The table renders that as `7d`;
    /// that is a rendering, and the document keeps the original. **Absent**
    /// when `permanent` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    /// The private note, the `REASON` column. **Absent** when there is none,
    /// which the table renders as `-`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_reason: Option<String>,
}

impl ListDocument {
    pub fn new(
        env: Option<&str>,
        universe_id: u64,
        limit: u32,
        include_inactive: bool,
        rows: &[UserRestriction],
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.map(str::to_string),
            universe_id: universe_id.to_string(),
            include_inactive,
            limit,
            limit_reached: rows.len() as u32 >= limit,
            count: rows.len(),
            restrictions: rows.iter().map(Restriction::from).collect(),
        }
    }
}

impl From<&UserRestriction> for Restriction {
    fn from(restriction: &UserRestriction) -> Self {
        let join = restriction.game_join_restriction.as_ref();
        let duration = join.and_then(|r| r.duration.clone());
        Self {
            user_id: restriction.user_id().map(|id| id.to_string()),
            active: restriction.is_active(),
            permanent: duration.is_none(),
            duration,
            private_reason: join.and_then(|r| r.private_reason.clone()),
        }
    }
}

/// One `ban status` invocation.
///
/// A document per run rather than per player: `ban status` takes several
/// players at once, and a consumer that asked about three of them should read
/// one document rather than have to tell three concatenated ones apart.
#[derive(Debug, Serialize)]
pub struct StatusDocument {
    pub schema_version: u32,
    /// The env that named the universe. **Absent** under a bare
    /// `--universe-id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    pub universe_id: String,
    /// Entries in `players`.
    pub count: usize,
    /// One object per player asked about, in the order they were given on the
    /// command line.
    pub players: Vec<Player>,
}

/// One player, resolved and looked up.
#[derive(Debug, Serialize)]
pub struct Player {
    pub user_id: String,
    /// The account name, which is what `ban add` takes back.
    pub username: String,
    /// The name Roblox shows other players. Not unique, and not what any of
    /// these commands resolve by.
    pub display_name: String,
    /// The link the human form prints under the name, so a report generated
    /// from this document can carry the same check.
    pub profile_url: String,
    /// The question the command was asked.
    pub restricted: bool,
    /// **Absent** when `restricted` is false: an unrestricted player has no
    /// length to report, and `false` there would read as a temporary ban.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permanent: Option<bool>,
    /// As Roblox sent it, `604800s`. **Absent** when the restriction is
    /// permanent, and when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    /// When the restriction started, the `since` line. **Absent** when Roblox
    /// sent none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    /// The private note, the `your note` line. **Absent** when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_reason: Option<String>,
    /// What the player is shown, the `player sees` line. **Absent** when there
    /// is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_reason: Option<String>,
}

impl StatusDocument {
    pub fn new(env: Option<&str>, universe_id: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.map(str::to_string),
            universe_id: universe_id.to_string(),
            count: 0,
            players: Vec::new(),
        }
    }

    /// Record one answered lookup. Called as each player comes back, so the
    /// document is built in the order the arguments were given.
    pub fn push(&mut self, user: &User, restriction: &UserRestriction) {
        let restricted = restriction.is_active();
        let join = restriction.game_join_restriction.as_ref();
        let duration = join.and_then(|r| r.duration.clone()).filter(|_| restricted);
        self.players.push(Player {
            user_id: user.id.to_string(),
            username: user.name.clone(),
            display_name: user.display_name.clone(),
            profile_url: user.profile_url(),
            restricted,
            permanent: restricted.then(|| duration.is_none()),
            duration,
            start_time: join
                .and_then(|r| r.start_time.clone())
                .filter(|_| restricted),
            private_reason: join
                .and_then(|r| r.private_reason.clone())
                .filter(|_| restricted),
            display_reason: join
                .and_then(|r| r.display_reason.clone())
                .filter(|_| restricted),
        });
        self.count = self.players.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GameJoinRestriction;

    fn parsed(document: &impl Serialize) -> serde_json::Value {
        let mut buf = Vec::new();
        rbx_core::output::write_json(&mut buf, document).expect("write");
        serde_json::from_slice(&buf).expect("the document must be valid JSON")
    }

    fn user(id: u64, name: &str, display: &str) -> User {
        User {
            id,
            name: name.to_string(),
            display_name: display.to_string(),
            has_verified_badge: false,
        }
    }

    fn restriction(json: &str) -> UserRestriction {
        serde_json::from_str(json).expect("fixture")
    }

    #[test]
    fn a_listing_carries_the_documented_fields() {
        let rows = [
            restriction(
                r#"{"user":"users/156","gameJoinRestriction":{"active":true,
                    "duration":"604800s","privateReason":"fly hack"}}"#,
            ),
            restriction(r#"{"user":"users/881","gameJoinRestriction":{"active":true}}"#),
        ];
        let doc = parsed(&ListDocument::new(
            Some("prod"),
            5544332211,
            100,
            false,
            &rows,
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["env"], "prod");
        assert_eq!(doc["universe_id"], "5544332211");
        assert_eq!(doc["include_inactive"], false);
        assert_eq!(doc["limit"], 100);
        assert_eq!(doc["limit_reached"], false);
        assert_eq!(doc["count"], 2);
        assert_eq!(doc["restrictions"][0]["user_id"], "156");
        assert_eq!(doc["restrictions"][0]["active"], true);
        assert_eq!(doc["restrictions"][0]["private_reason"], "fly hack");
    }

    /// The distinction the module exists for. A permanent restriction is the
    /// harshest outcome and Roblox expresses it by omission, so the document
    /// states it rather than passing the omission along.
    #[test]
    fn a_permanent_restriction_says_so_rather_than_omitting_a_duration() {
        let rows = [
            restriction(r#"{"user":"users/1","gameJoinRestriction":{"active":true}}"#),
            restriction(
                r#"{"user":"users/2","gameJoinRestriction":{"active":true,"duration":"604800s"}}"#,
            ),
        ];
        let doc = parsed(&ListDocument::new(None, 1, 100, false, &rows));

        assert_eq!(doc["restrictions"][0]["permanent"], true);
        assert!(doc["restrictions"][0].get("duration").is_none(), "{doc}");
        assert_eq!(doc["restrictions"][1]["permanent"], false);
        // Roblox's own spelling, not the `7d` the table renders.
        assert_eq!(doc["restrictions"][1]["duration"], "604800s");
    }

    /// `--include-inactive` is the only way one of these reaches the listing,
    /// and without the flag the two kinds of row look identical.
    #[test]
    fn an_inactive_entry_is_reported_as_inactive() {
        let rows = [restriction(
            r#"{"user":"users/1","gameJoinRestriction":{"active":false,"duration":"60s"}}"#,
        )];
        let doc = parsed(&ListDocument::new(None, 1, 100, true, &rows));

        assert_eq!(doc["include_inactive"], true);
        assert_eq!(doc["restrictions"][0]["active"], false);
    }

    /// The human table renders an unreadable resource path as `?`; the document
    /// omits the key rather than inventing an id.
    #[test]
    fn an_unreadable_user_path_omits_the_id() {
        let rows = [restriction(
            r#"{"user":"users/abc","gameJoinRestriction":{"active":true}}"#,
        )];
        let doc = parsed(&ListDocument::new(None, 1, 100, false, &rows));

        assert!(doc["restrictions"][0].get("user_id").is_none(), "{doc}");
    }

    /// Nobody restricted is a fact, not a failure: an empty array a consumer
    /// reads a zero off, and exit 0.
    #[test]
    fn nobody_restricted_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&ListDocument::new(Some("dev"), 1, 100, false, &[]));

        assert_eq!(doc["count"], 0);
        assert_eq!(doc["restrictions"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["limit_reached"], false);
    }

    #[test]
    fn hitting_the_limit_is_reported_rather_than_left_to_be_inferred() {
        let rows = [
            restriction(r#"{"user":"users/1"}"#),
            restriction(r#"{"user":"users/2"}"#),
        ];
        assert!(ListDocument::new(None, 1, 2, false, &rows).limit_reached);
        assert!(!ListDocument::new(None, 1, 3, false, &rows).limit_reached);
    }

    /// The listing prints neither of these, so it promises neither. `status`
    /// prints both and carries both.
    #[test]
    fn the_listing_never_carries_what_the_player_was_shown() {
        let rows = [restriction(
            r#"{"path":"universes/1/user-restrictions/156","updateTime":"2026-08-01T00:00:00Z",
                "user":"users/156","gameJoinRestriction":{"active":true,"duration":"60s",
                "startTime":"2026-08-01T10:12:03Z","privateReason":"fly hack",
                "displayReason":"Banned 7 days for cheating","excludeAltAccounts":true,
                "inherited":false}}"#,
        )];
        let rendered = parsed(&ListDocument::new(None, 1, 100, false, &rows)).to_string();

        for absent in [
            "display_reason",
            "Banned 7 days",
            "start_time",
            "exclude_alt_accounts",
            "inherited",
            "path",
            "update_time",
        ] {
            assert!(!rendered.contains(absent), "{absent} leaked: {rendered}");
        }
    }

    #[test]
    fn a_status_document_carries_what_the_human_form_prints_per_player() {
        let mut doc = StatusDocument::new(Some("prod"), 5544332211);
        doc.push(
            &user(156, "builderman", "Builder Man"),
            &restriction(
                r#"{"user":"users/156","gameJoinRestriction":{"active":true,
                    "duration":"604800s","startTime":"2026-08-01T10:12:03Z",
                    "privateReason":"fly hack, clip of 3 Aug",
                    "displayReason":"Banned 7 days for cheating"}}"#,
            ),
        );
        let doc = parsed(&doc);

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["env"], "prod");
        assert_eq!(doc["universe_id"], "5544332211");
        assert_eq!(doc["count"], 1);
        assert_eq!(doc["players"][0]["user_id"], "156");
        assert_eq!(doc["players"][0]["username"], "builderman");
        assert_eq!(doc["players"][0]["display_name"], "Builder Man");
        assert_eq!(
            doc["players"][0]["profile_url"],
            "https://www.roblox.com/users/156/profile"
        );
        assert_eq!(doc["players"][0]["restricted"], true);
        assert_eq!(doc["players"][0]["permanent"], false);
        assert_eq!(doc["players"][0]["duration"], "604800s");
        assert_eq!(doc["players"][0]["start_time"], "2026-08-01T10:12:03Z");
        assert_eq!(
            doc["players"][0]["private_reason"],
            "fly hack, clip of 3 Aug"
        );
        assert_eq!(
            doc["players"][0]["display_reason"],
            "Banned 7 days for cheating"
        );
    }

    /// An unrestricted player is a full answer with nothing hanging off it.
    /// `permanent` is absent rather than false: false would read as "restricted,
    /// but not for ever".
    #[test]
    fn an_unrestricted_player_carries_no_restriction_fields() {
        let mut doc = StatusDocument::new(None, 1);
        doc.push(
            &user(881, "someone", "someone"),
            &UserRestriction::default(),
        );
        let doc = parsed(&doc);

        assert_eq!(doc["players"][0]["restricted"], false);
        for absent in [
            "permanent",
            "duration",
            "start_time",
            "private_reason",
            "display_reason",
        ] {
            assert!(doc["players"][0].get(absent).is_none(), "{absent}: {doc}");
        }
    }

    /// A lifted restriction leaves its record behind, reasons included. The
    /// human form prints `not restricted` and nothing else for one of those, so
    /// neither does the document.
    #[test]
    fn a_lifted_restriction_reports_only_that_it_is_lifted() {
        let mut doc = StatusDocument::new(None, 1);
        doc.push(
            &user(881, "someone", "someone"),
            &restriction(
                r#"{"user":"users/881","gameJoinRestriction":{"active":false,
                    "privateReason":"lifted on appeal","displayReason":"old text"}}"#,
            ),
        );
        let doc = parsed(&doc);

        assert_eq!(doc["players"][0]["restricted"], false);
        assert!(!doc.to_string().contains("lifted on appeal"), "{doc}");
        assert!(!doc.to_string().contains("old text"), "{doc}");
    }

    /// Several players asked about is one document, not three concatenated.
    #[test]
    fn several_players_are_one_document_in_the_order_they_were_given() {
        let mut doc = StatusDocument::new(None, 1);
        doc.push(&user(1, "a", "a"), &UserRestriction::default());
        doc.push(&user(2, "b", "b"), &UserRestriction::default());
        let doc = parsed(&doc);

        assert_eq!(doc["count"], 2);
        assert_eq!(doc["players"][0]["username"], "a");
        assert_eq!(doc["players"][1]["username"], "b");
    }

    /// A permanent restriction, seen from `status`.
    #[test]
    fn a_permanently_restricted_player_says_permanent() {
        let mut doc = StatusDocument::new(None, 1);
        doc.push(
            &user(156, "builderman", "builderman"),
            &UserRestriction {
                game_join_restriction: Some(GameJoinRestriction {
                    active: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        let doc = parsed(&doc);

        assert_eq!(doc["players"][0]["restricted"], true);
        assert_eq!(doc["players"][0]["permanent"], true);
        assert!(doc["players"][0].get("duration").is_none(), "{doc}");
    }

    /// Every id is a string, so nothing rounds a 64-bit universe id and one
    /// filter reads an id out of either document.
    #[test]
    fn every_id_is_a_string() {
        let rows = [restriction(
            r#"{"user":"users/123456789012345","gameJoinRestriction":{"active":true}}"#,
        )];
        let doc = parsed(&ListDocument::new(
            None,
            123_456_789_012_345,
            100,
            false,
            &rows,
        ));

        assert!(doc["universe_id"].is_string(), "{doc}");
        assert!(doc["restrictions"][0]["user_id"].is_string(), "{doc}");
    }

    /// `--json` owns stdout, so nothing on either path may stop and ask a
    /// question. The two reading subcommands have nothing to ask; the two
    /// writing ones ask through `confirm_always` and therefore do not carry the
    /// flag at all. This is the test that says which way a question added later
    /// has to go.
    #[test]
    fn the_json_format_refuses_to_prompt() {
        assert!(!rbx_core::output::OutputFormat::Json.may_prompt());
    }
}
