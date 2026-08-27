//! Deletion templates: what `rbxrtbf.toml` declares, and what Roblox stores.
//!
//! # What these are for
//!
//! When Roblox processes a right-to-be-forgotten request for one of your
//! players, it does not know where that player's data lives. A template tells
//! it: a data store name and a key pattern holding a `{UserId}` token, which
//! Roblox substitutes with the requester's id and then deletes what matches.
//!
//! # Why this file is typed rather than left to `rbx config`
//!
//! The templates live in the `DataStoresConfig` repository of the same Configs
//! API `rbx config` drives, under a single entry named `user_data_templates`, so
//! `rbx config sync --repository DataStoresConfig` could push them today. What
//! it could not do is check them, because its entry model holds an opaque
//! `toml::Value`.
//!
//! Every rule below is one this tool can check and that command cannot, and the
//! failure mode is why it is worth the crate: a template that matches nothing
//! deletes nothing, in silence, and you find out when a legal request goes
//! unfulfilled. Roblox's own guidance is to compare the patterns against your
//! live Luau by hand in the Creator Hub and then to confirm within 30 days that
//! the data went, which is an admission that nothing verifies it for you.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The single entry name the templates are stored under.
pub const ENTRY_KEY: &str = "user_data_templates";

/// The token Roblox substitutes with the requester's user id.
///
/// **Case-sensitive**, which is the first thing Roblox's own best-practices
/// list says. `{userId}` is not accepted and does not warn: the template simply
/// matches nothing.
pub const USER_ID_TOKEN: &str = "{UserId}";

/// Roblox's stated ceiling on how many templates one universe may declare.
pub const MAX_TEMPLATES: usize = 100;

/// The scope a key template targets when it names none.
///
/// Roblox's rule, stated in the RTBF guide: an omitted or blank `scope_pattern`
/// defaults to `global`. Written out rather than left implicit, because a store
/// using a non-default scope and a template silently defaulting to `global` is
/// one of the ways a pattern matches nothing.
pub const DEFAULT_SCOPE: &str = "global";

/// One template naming specific keys inside a data store.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyTemplate {
    /// The data store holding the key. An exact name, not a pattern: Roblox
    /// matches the store by name and only the key and scope by pattern.
    pub store: String,

    /// The key pattern, which must contain `{UserId}`.
    pub pattern: String,

    /// The scope pattern. Omitted means [`DEFAULT_SCOPE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,

    /// Whether this is an ordered data store rather than a standard one.
    ///
    /// A bool rather than the API's `STANDARD` / `ORDERED` string, because
    /// those are the only two values and a bool cannot be misspelled.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ordered: bool,
}

/// One template naming a whole data store, by pattern.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreTemplate {
    /// The data store name pattern, which must contain `{UserId}`.
    pub pattern: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl KeyTemplate {
    /// The scope this template targets, defaulted.
    ///
    /// Blank counts as omitted, matching Roblox's rule: a `scope = ""` in the
    /// file means `global` on the wire, and pretending otherwise locally would
    /// make the two disagree about the same declaration.
    pub fn effective_scope(&self) -> &str {
        match self.scope.as_deref() {
            Some(scope) if !scope.trim().is_empty() => scope,
            _ => DEFAULT_SCOPE,
        }
    }

    fn data_store_type(&self) -> &'static str {
        if self.ordered {
            "ORDERED"
        } else {
            "STANDARD"
        }
    }

    /// What this template looks like once Roblox substitutes a real id.
    ///
    /// Shown by `check` and `verify` rather than only the pattern, because a
    /// pattern is read for what it was meant to say and a sample is read for
    /// what it will actually match.
    pub fn sample(&self, user_id: u64) -> String {
        format!(
            "{}/{}/{}",
            self.store,
            substitute(self.effective_scope(), user_id),
            substitute(&self.pattern, user_id)
        )
    }
}

impl StoreTemplate {
    pub fn sample(&self, user_id: u64) -> String {
        substitute(&self.pattern, user_id)
    }
}

/// Replace the token with a concrete id, the way Roblox will.
pub fn substitute(pattern: &str, user_id: u64) -> String {
    pattern.replace(USER_ID_TOKEN, &user_id.to_string())
}

/// Every template a universe declares.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Templates {
    /// Templates naming keys inside a named store.
    #[serde(default, rename = "key", skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyTemplate>,

    /// Templates naming a whole store by pattern.
    ///
    /// Deliberately a second array rather than one list with a discriminator.
    /// Order across the two kinds carries no meaning (deletion is a match, not
    /// a sequence), and two arrays read as what they are: the keys you delete
    /// and the stores you delete.
    #[serde(default, rename = "store", skip_serializing_if = "Vec::is_empty")]
    pub stores: Vec<StoreTemplate>,
}

impl Templates {
    pub fn total(&self) -> usize {
        self.keys.len() + self.stores.len()
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    /// Refuse a template set Roblox would take and then never match.
    ///
    /// The distinction that matters: Roblox rejects very little here. Almost
    /// every mistake in this file is *accepted*, stored, and then quietly
    /// matches nothing. So these are not a mirror of the server's validation,
    /// they are the checks nobody else performs.
    pub fn validate(&self) -> Result<()> {
        if self.total() > MAX_TEMPLATES {
            bail!(
                "{} templates, over Roblox's limit of {MAX_TEMPLATES}. \
                 A template can cover many keys: widen the patterns rather than \
                 listing each one.",
                self.total()
            );
        }

        for (index, key) in self.keys.iter().enumerate() {
            let at = format!("[[key]] #{}", index + 1);
            if key.store.trim().is_empty() {
                bail!("{at}: `store` is empty. Name the data store the key lives in.");
            }
            if key.pattern.trim().is_empty() {
                bail!("{at}: `pattern` is empty.");
            }

            // Both fields, and **regardless of whether a correct token is also
            // present**. `User_{UserId}_{userid}` used to pass, because the
            // first correct token short-circuited the scan; Roblox substitutes
            // the first and leaves `{userid}` as literal text, so the key it
            // looks for matches nothing real. That is the crate's whole failure
            // mode, reached through a merge or a copy-paste.
            check_no_near_miss(&at, "pattern", &key.pattern)?;
            if let Some(scope) = key.scope.as_deref() {
                check_no_near_miss(&at, "scope", scope)?;
            }

            // **The token may live in either field.** Roblox's eligibility rule
            // is that the user id must be part of the *name or the scope* of
            // the key, so a store keyed by a constant under a per-user scope
            // (`pattern = "Data"`, `scope = "User_{UserId}"`) is a documented,
            // working configuration. Requiring it in `pattern` refused that,
            // and told the author Roblox would delete nothing when Roblox
            // would in fact delete: the scope carries the id.
            let in_pattern = key.pattern.contains(USER_ID_TOKEN);
            let in_scope = key
                .scope
                .as_deref()
                .is_some_and(|scope| scope.contains(USER_ID_TOKEN));
            if !in_pattern && !in_scope {
                bail!(
                    "{at}: neither `pattern` nor `scope` has a \"{USER_ID_TOKEN}\" token, so \
                     this names one fixed key rather than a user's. Roblox needs the id in \
                     the key name or the scope; as written it would accept this and delete \
                     nothing."
                );
            }
        }

        for (index, store) in self.stores.iter().enumerate() {
            let at = format!("[[store]] #{}", index + 1);
            check_no_near_miss(&at, "pattern", &store.pattern)?;
            // A store template has no scope, so the name is the only place the
            // id can be.
            check_token(&at, "pattern", &store.pattern)?;
        }

        Ok(())
    }

    /// The `entries` payload the Configs API takes.
    ///
    /// One key, `user_data_templates`, holding the array of tagged objects the
    /// RTBF guide documents. Built from the typed form rather than written by
    /// hand so the two cannot drift.
    pub fn to_entries(&self) -> BTreeMap<String, Json> {
        let mut list: Vec<Json> = Vec::with_capacity(self.total());
        for key in &self.keys {
            list.push(serde_json::json!({
                "key_template": {
                    "data_store_type": key.data_store_type(),
                    "data_store_name": key.store,
                    "key_pattern": key.pattern,
                    "scope_pattern": key.effective_scope(),
                }
            }));
        }
        for store in &self.stores {
            list.push(serde_json::json!({
                // Only STANDARD is supported for a whole-store template, which
                // is why `[[store]]` has no `ordered` field to send.
                "data_store_template": {
                    "data_store_type": "STANDARD",
                    "data_store_pattern": store.pattern,
                }
            }));
        }
        BTreeMap::from([(ENTRY_KEY.to_string(), Json::Array(list))])
    }

    /// Read the templates back out of a published config.
    ///
    /// Lenient on purpose: an entry shape this build does not recognise is
    /// skipped and counted rather than failing the read, because `pull` and
    /// `check` have to keep working against a universe configured by a newer
    /// release or by the Creator Hub. The count is what the caller reports.
    pub fn from_entries(entries: &BTreeMap<String, Json>) -> (Self, usize) {
        let Some(Json::Array(list)) = entries.get(ENTRY_KEY) else {
            return (Self::default(), 0);
        };

        let mut templates = Self::default();
        let mut unrecognised = 0;
        for item in list {
            if let Some(key) = item.get("key_template") {
                let store = string_at(key, "data_store_name");
                let pattern = string_at(key, "key_pattern");
                if store.is_empty() || pattern.is_empty() {
                    unrecognised += 1;
                    continue;
                }
                let scope = string_at(key, "scope_pattern");
                templates.keys.push(KeyTemplate {
                    store,
                    pattern,
                    // `global` is the default, so it is dropped on the way in:
                    // writing it back would put a line in the file that says
                    // what its absence already says.
                    scope: (scope != DEFAULT_SCOPE && !scope.is_empty()).then_some(scope),
                    ordered: string_at(key, "data_store_type") == "ORDERED",
                });
            } else if let Some(store) = item.get("data_store_template") {
                let pattern = string_at(store, "data_store_pattern");
                if pattern.is_empty() {
                    unrecognised += 1;
                    continue;
                }
                templates.stores.push(StoreTemplate { pattern });
            } else {
                unrecognised += 1;
            }
        }
        (templates, unrecognised)
    }
}

fn string_at(value: &Json, field: &str) -> String {
    value
        .get(field)
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Refuse a pattern with no usable `{UserId}`.
///
/// A pattern without the token is a constant, so Roblox stores it, matches at
/// most one key belonging to nobody in particular, and reports nothing.
///
/// The near-miss case is handled by [`check_no_near_miss`], which callers run
/// **first** and unconditionally: this one only answers "is there a correct
/// token at all".
fn check_token(at: &str, field: &str, pattern: &str) -> Result<()> {
    if pattern.trim().is_empty() {
        bail!("{at}: `{field}` is empty.");
    }
    if pattern.contains(USER_ID_TOKEN) {
        return Ok(());
    }
    bail!(
        "{at}: `{field}` has no \"{USER_ID_TOKEN}\" token, so it names one fixed key \
         rather than a user's. Roblox would accept it and delete nothing."
    )
}

/// Refuse a miscased token anywhere in a value.
///
/// **Runs whether or not a correct token is also present.** It used to
/// short-circuit on the first correct one, which let `User_{UserId}_{userid}`
/// through: Roblox substitutes the first and leaves the second as literal text,
/// so the key it looks for matches nothing real. That is the crate's own
/// failure mode, and a merge or a copy-paste of a second identifier is how it
/// arrives.
fn check_no_near_miss(at: &str, field: &str, value: &str) -> Result<()> {
    if let Some(found) = near_miss(value) {
        bail!(
            "{at}: `{field}` contains \"{found}\", and the token is case-sensitive: it must \
             be exactly \"{USER_ID_TOKEN}\". As written, Roblox would store this template \
             and it would match nothing."
        );
    }
    Ok(())
}

/// A brace-delimited run that looks like the token but is not it.
///
/// Compared case-insensitively against the token's own text, so `{userid}`,
/// `{USERID}` and `{userId}` are all caught while an unrelated `{version}` is
/// left alone. This is the mistake Roblox's best-practices list puts first, and
/// it is invisible: the wrong case is stored happily and matches nothing.
///
/// Each `}` is paired with the **nearest preceding** `{`. Pairing the first
/// opener with the first closer let a stray brace swallow a real run:
/// `Player{_{userId}` produced the inner text `_{userId`, which matches
/// nothing, and the miscased token went out. `{{userId}}` hid the same way.
fn near_miss(value: &str) -> Option<String> {
    let inner_token = USER_ID_TOKEN.trim_matches(|c| c == '{' || c == '}');
    let mut open: Option<usize> = None;
    // `char_indices` rather than byte scanning: the braces are single-byte, but
    // the text between them need not be, and the slice below is taken on those
    // offsets.
    for (at, character) in value.char_indices() {
        match character {
            '{' => open = Some(at),
            '}' => {
                if let Some(start) = open.take() {
                    let inner = &value[start + 1..at];
                    if inner.eq_ignore_ascii_case(inner_token) && inner != inner_token {
                        return Some(format!("{{{inner}}}"));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pattern: &str) -> KeyTemplate {
        KeyTemplate {
            store: "PlayerInventory".into(),
            pattern: pattern.into(),
            scope: None,
            ordered: false,
        }
    }

    #[test]
    fn a_declared_template_becomes_the_payload_the_guide_documents() {
        let templates = Templates {
            keys: vec![
                KeyTemplate {
                    store: "PlayerInventory".into(),
                    pattern: "User_{UserId}".into(),
                    scope: Some("Scope_{UserId}".into()),
                    ordered: false,
                },
                KeyTemplate {
                    store: "PlayerLeaderboard".into(),
                    pattern: "User_{UserId}".into(),
                    scope: None,
                    ordered: true,
                },
            ],
            stores: vec![StoreTemplate {
                pattern: "Player_{UserId}_Save".into(),
            }],
        };

        let entries = templates.to_entries();
        assert_eq!(entries.len(), 1, "one entry, whatever the template count");
        assert_eq!(
            entries[ENTRY_KEY],
            serde_json::json!([
                {"key_template": {
                    "data_store_type": "STANDARD",
                    "data_store_name": "PlayerInventory",
                    "key_pattern": "User_{UserId}",
                    "scope_pattern": "Scope_{UserId}"
                }},
                {"key_template": {
                    "data_store_type": "ORDERED",
                    "data_store_name": "PlayerLeaderboard",
                    "key_pattern": "User_{UserId}",
                    "scope_pattern": "global"
                }},
                {"data_store_template": {
                    "data_store_type": "STANDARD",
                    "data_store_pattern": "Player_{UserId}_Save"
                }}
            ])
        );
    }

    /// An omitted scope is `global` on the wire, and comes back omitted, so a
    /// pull of a synced file is a no-op rather than a line that says what its
    /// own absence already said.
    #[test]
    fn the_default_scope_round_trips_as_an_absence() {
        let before = Templates {
            keys: vec![key("User_{UserId}")],
            stores: vec![],
        };
        let (after, unrecognised) = Templates::from_entries(&before.to_entries());
        assert_eq!(after, before);
        assert_eq!(unrecognised, 0);
        assert_eq!(before.keys[0].effective_scope(), "global");
    }

    #[test]
    fn a_blank_scope_is_the_default_rather_than_a_blank_scope() {
        let mut template = key("User_{UserId}");
        template.scope = Some("   ".into());
        assert_eq!(template.effective_scope(), DEFAULT_SCOPE);
    }

    #[test]
    fn everything_round_trips_including_ordered_and_an_explicit_scope() {
        let before = Templates {
            keys: vec![
                KeyTemplate {
                    store: "A".into(),
                    pattern: "User_{UserId}".into(),
                    scope: Some("Scope_{UserId}".into()),
                    ordered: true,
                },
                key("Player_{UserId}"),
            ],
            stores: vec![StoreTemplate {
                pattern: "Save_{UserId}".into(),
            }],
        };
        let (after, unrecognised) = Templates::from_entries(&before.to_entries());
        assert_eq!(after, before);
        assert_eq!(unrecognised, 0);
    }

    /// The mistake Roblox's own best-practices list puts first, and the reason
    /// this crate exists rather than a `rbx config` invocation: the wrong case
    /// is accepted, stored, and matches nothing.
    #[test]
    fn a_miscased_token_is_refused_and_the_message_says_it_is_case_sensitive() {
        for wrong in ["User_{userId}", "User_{userid}", "User_{USERID}"] {
            let templates = Templates {
                keys: vec![key(wrong)],
                stores: vec![],
            };
            let err = templates.validate().unwrap_err().to_string();
            assert!(err.contains("case-sensitive"), "{wrong}: {err}");
            assert!(err.contains("match nothing"), "{wrong}: {err}");
        }
    }

    #[test]
    fn a_key_with_the_token_in_neither_field_says_it_would_delete_nothing() {
        let templates = Templates {
            keys: vec![key("PlayerData")],
            stores: vec![],
        };
        let err = templates.validate().unwrap_err().to_string();
        assert!(err.contains("neither `pattern` nor `scope`"), "{err}");
        assert!(err.contains("delete nothing"), "{err}");
    }

    /// Roblox's eligibility rule is that the user id must be part of the
    /// **name or the scope**, so a store keyed by a constant under a per-user
    /// scope is a documented, working configuration. Requiring the token in
    /// `pattern` refused it and told the author Roblox would delete nothing,
    /// which was wrong on its own terms: the scope carries the id.
    #[test]
    fn the_token_may_live_in_the_scope_instead_of_the_pattern() {
        let mut template = key("Data");
        template.scope = Some("User_{UserId}".into());
        assert!(Templates {
            keys: vec![template],
            stores: vec![]
        }
        .validate()
        .is_ok());
    }

    /// A store template has no scope, so its name is the only place the id can
    /// be, and the rule stays strict there.
    #[test]
    fn a_store_pattern_still_needs_the_token_itself() {
        let err = Templates {
            keys: vec![],
            stores: vec![StoreTemplate {
                pattern: "PlayerSaves".into(),
            }],
        }
        .validate()
        .unwrap_err()
        .to_string();
        assert!(err.contains("no \"{UserId}\" token"), "{err}");
    }

    /// The scan used to stop at the first correct token, so this passed and was
    /// published. Roblox substitutes the first and leaves `{userid}` as literal
    /// text, so the key it looks for matches nothing: the crate's own failure
    /// mode, reached by a merge or a copy-paste of a second identifier.
    #[test]
    fn a_miscased_token_beside_a_correct_one_is_still_refused() {
        for wrong in [
            "User_{UserId}_{userid}",
            "{userId}_User_{UserId}",
            "{UserId}{USERID}",
        ] {
            let err = Templates {
                keys: vec![key(wrong)],
                stores: vec![],
            }
            .validate()
            .unwrap_err()
            .to_string();
            assert!(err.contains("case-sensitive"), "{wrong}: {err}");
        }

        // And the same in a scope, where there is no second rule to catch it.
        let mut template = key("User_{UserId}");
        template.scope = Some("{UserId}_{userId}".into());
        let err = Templates {
            keys: vec![template],
            stores: vec![],
        }
        .validate()
        .unwrap_err()
        .to_string();
        assert!(err.contains("case-sensitive"), "{err}");
    }

    /// Each `}` pairs with the nearest preceding `{`. Pairing the first opener
    /// with the first closer let a stray brace swallow a real run, and on a
    /// `scope` that was a silent pass.
    #[test]
    fn a_stray_brace_does_not_hide_a_miscased_token_behind_it() {
        assert_eq!(near_miss("Player{_{userId}"), Some("{userId}".into()));
        assert_eq!(near_miss("{{userId}}"), Some("{userId}".into()));

        let mut template = key("User_{UserId}");
        template.scope = Some("Player{_{userId}".into());
        assert!(
            Templates {
                keys: vec![template],
                stores: vec![]
            }
            .validate()
            .is_err(),
            "a stray brace must not launder a miscased token"
        );
    }

    /// The braces are single-byte but the text between them need not be, and
    /// the scan slices on those offsets. A panic on user input would be worse
    /// than any rule this file enforces.
    #[test]
    fn a_multi_byte_character_inside_the_braces_does_not_panic() {
        assert_eq!(near_miss("{UsérId}"), None);
        assert_eq!(near_miss("héllo {userid} wörld"), Some("{userid}".into()));
        assert_eq!(near_miss("{}"), None);
    }

    /// An unrelated placeholder is somebody's naming scheme, not a typo.
    #[test]
    fn an_unrelated_brace_run_is_left_alone() {
        assert_eq!(near_miss("User_{version}_{UserId}"), None);
        assert_eq!(near_miss("no braces here"), None);
        assert_eq!(near_miss("unclosed {UserI"), None);
        assert_eq!(near_miss("{userid}"), Some("{userid}".into()));
    }

    /// A constant scope is legitimate, so the token is not required there. A
    /// *misspelled* one still is a mistake: it says a per-user scope was meant.
    #[test]
    fn a_constant_scope_is_fine_but_a_miscased_one_is_not() {
        let mut ok = key("User_{UserId}");
        ok.scope = Some("global".into());
        assert!(Templates {
            keys: vec![ok],
            stores: vec![]
        }
        .validate()
        .is_ok());

        let mut wrong = key("User_{UserId}");
        wrong.scope = Some("Scope_{userId}".into());
        let err = Templates {
            keys: vec![wrong],
            stores: vec![],
        }
        .validate()
        .unwrap_err()
        .to_string();
        assert!(err.contains("case-sensitive"), "{err}");
    }

    #[test]
    fn a_key_template_with_no_store_is_refused() {
        let mut template = key("User_{UserId}");
        template.store = "  ".into();
        let err = Templates {
            keys: vec![template],
            stores: vec![],
        }
        .validate()
        .unwrap_err()
        .to_string();
        assert!(err.contains("`store` is empty"), "{err}");
    }

    #[test]
    fn the_template_ceiling_is_roblox_s_and_counts_both_kinds() {
        let fits = Templates {
            keys: vec![key("User_{UserId}"); MAX_TEMPLATES - 1],
            stores: vec![StoreTemplate {
                pattern: "S_{UserId}".into(),
            }],
        };
        assert_eq!(fits.total(), MAX_TEMPLATES);
        assert!(fits.validate().is_ok());

        let over = Templates {
            keys: vec![key("User_{UserId}"); MAX_TEMPLATES],
            stores: vec![StoreTemplate {
                pattern: "S_{UserId}".into(),
            }],
        };
        let err = over.validate().unwrap_err().to_string();
        assert!(err.contains("101 templates"), "{err}");
        assert!(err.contains("limit of 100"), "{err}");
    }

    /// The refusal has to say *which* template, because a hundred of them look
    /// alike in a diff.
    #[test]
    fn a_refusal_names_the_template_by_kind_and_position() {
        let templates = Templates {
            keys: vec![key("User_{UserId}"), key("bad")],
            stores: vec![],
        };
        let err = templates.validate().unwrap_err().to_string();
        assert!(err.contains("[[key]] #2"), "{err}");

        let templates = Templates {
            keys: vec![],
            stores: vec![
                StoreTemplate {
                    pattern: "A_{UserId}".into(),
                },
                StoreTemplate {
                    pattern: "B".into(),
                },
            ],
        };
        let err = templates.validate().unwrap_err().to_string();
        assert!(err.contains("[[store]] #2"), "{err}");
    }

    /// What the Creator Hub calls the sample output, and the only way to read a
    /// template for what it will actually match.
    #[test]
    fn a_sample_shows_what_roblox_will_look_for() {
        let mut template = key("User_{UserId}");
        template.scope = Some("Scope_{UserId}".into());
        assert_eq!(
            template.sample(1234567890),
            "PlayerInventory/Scope_1234567890/User_1234567890"
        );

        assert_eq!(
            key("User_{UserId}").sample(1234567890).split('/').nth(1),
            Some("global")
        );

        assert_eq!(
            StoreTemplate {
                pattern: "Player_{UserId}_Save".into()
            }
            .sample(1234567890),
            "Player_1234567890_Save"
        );
    }

    /// A universe configured by a newer release, or by hand in the Creator Hub,
    /// must stay readable: an entry this build does not know is counted, not
    /// fatal.
    #[test]
    fn an_unrecognised_entry_is_counted_rather_than_failing_the_read() {
        let entries = BTreeMap::from([(
            ENTRY_KEY.to_string(),
            serde_json::json!([
                {"key_template": {"data_store_name": "A", "key_pattern": "U_{UserId}"}},
                {"future_template": {"whatever": true}},
                {"key_template": {"key_pattern": "no store name"}}
            ]),
        )]);
        let (templates, unrecognised) = Templates::from_entries(&entries);
        assert_eq!(templates.keys.len(), 1);
        assert_eq!(unrecognised, 2);
    }

    #[test]
    fn a_universe_with_no_templates_reads_as_empty_rather_than_failing() {
        let (templates, unrecognised) = Templates::from_entries(&BTreeMap::new());
        assert!(templates.is_empty());
        assert_eq!(unrecognised, 0);

        let other = BTreeMap::from([("something.else".to_string(), Json::from(1))]);
        assert!(Templates::from_entries(&other).0.is_empty());
    }

    /// An empty file is a legitimate state, not an error: it is what a project
    /// that has not onboarded yet has, and `sync` publishing it is how you
    /// clear templates you no longer want.
    #[test]
    fn no_templates_at_all_validates() {
        assert!(Templates::default().validate().is_ok());
        assert_eq!(
            Templates::default().to_entries()[ENTRY_KEY],
            serde_json::json!([])
        );
    }
}
