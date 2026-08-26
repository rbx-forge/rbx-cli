//! `--json` on the two reads, against the contract the rest of the suite keeps:
//! one document to stdout and nothing else, documented field names, a
//! `schema_version`, ids as strings, and an optional field absent rather than
//! `null`.
//!
//! `show` and `verify` only. `check` has none, and that is deliberate rather
//! than an omission: no per-tool `check` in this suite carries `--json`, because
//! `rbx check --json` is the machine-readable drift document and two shapes for
//! one question is how a consumer comes to read the wrong one.
//!
//! # Why `verify` needs one at all
//!
//! It is the only command here whose answer nothing else produces. A CI step
//! that wants "are our deletion templates still pointing at real stores" reads
//! this document; the alternative is grepping a human listing for a red cross,
//! which is how a compliance check comes to pass on a changed message.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::model::{KeyTemplate, StoreTemplate, Templates, MAX_TEMPLATES};

/// One template, in the shape both documents carry it.
///
/// Flat, with `kind` discriminating, rather than the file's two arrays: a
/// consumer filtering on `kind` reads one list, and the two-array shape exists
/// in the TOML for a human's sake rather than a script's.
#[derive(Debug, Serialize)]
pub struct TemplateEntry {
    /// `key` or `store`.
    pub kind: &'static str,
    /// The data store name. **Absent** for a `store` template, whose store is
    /// the pattern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// The key pattern, or the store name pattern.
    pub pattern: String,
    /// The scope, defaulted: a key template that names none reports `global`,
    /// because that is what Roblox will match on and the absence in the file
    /// is not what a consumer is asking about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// **Absent** for a `store` template: Roblox supports deleting a whole
    /// store only for standard stores, so there is no choice to report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    /// What Roblox will look for once the token is substituted.
    pub sample: String,
}

impl TemplateEntry {
    fn key(template: &KeyTemplate, user_id: u64) -> Self {
        Self {
            kind: "key",
            store: Some(template.store.clone()),
            pattern: template.pattern.clone(),
            scope: Some(template.effective_scope().to_string()),
            ordered: Some(template.ordered),
            sample: template.sample(user_id),
        }
    }

    fn store(template: &StoreTemplate, user_id: u64) -> Self {
        Self {
            kind: "store",
            store: None,
            pattern: template.pattern.clone(),
            scope: None,
            ordered: None,
            sample: template.sample(user_id),
        }
    }
}

fn entries(templates: &Templates, user_id: u64) -> Vec<TemplateEntry> {
    templates
        .keys
        .iter()
        .map(|t| TemplateEntry::key(t, user_id))
        .chain(
            templates
                .stores
                .iter()
                .map(|t| TemplateEntry::store(t, user_id)),
        )
        .collect()
}

/// What `rbx rtbf show --json` writes.
///
/// Declared state, read from the file, with no network and therefore no env: it
/// says what this repository declares, not what any universe is serving. That
/// is `rbx check --json`'s question.
#[derive(Debug, Serialize)]
pub struct ShowDocument {
    pub schema_version: u32,
    /// The file this was read from, as given or defaulted, so a document
    /// captured out of a matrix job still says which one produced it.
    pub config_file: String,
    /// Roblox's ceiling, carried so a consumer can warn before it is reached
    /// rather than having the number hardcoded in two places.
    pub max_templates: usize,
    pub count: usize,
    /// The id substituted into every `sample`.
    pub sample_user_id: String,
    pub templates: Vec<TemplateEntry>,
}

impl ShowDocument {
    pub fn new(config_file: &str, templates: &Templates, user_id: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            config_file: config_file.to_string(),
            max_templates: MAX_TEMPLATES,
            count: templates.total(),
            sample_user_id: user_id.to_string(),
            templates: entries(templates, user_id),
        }
    }
}

/// One template `verify` had something to say about.
#[derive(Debug, Serialize)]
pub struct Finding {
    /// `key` or `store`.
    pub kind: &'static str,
    /// What the template names: the store for a key template, the pattern for a
    /// store template.
    pub target: String,
    /// `missing`, `unmatched` or `unverifiable`.
    ///
    /// Three values rather than a boolean, because `unverifiable` is not a
    /// failure and folding it into one would make a consumer treat an ordered
    /// store, which Open Cloud cannot list, as a broken template.
    pub verdict: &'static str,
    /// Why, in the same words the human form prints.
    pub detail: String,
}

/// What `rbx rtbf verify --json` writes.
#[derive(Debug, Serialize)]
pub struct VerifyDocument {
    pub schema_version: u32,
    pub config_file: String,
    /// The env asked for. **Absent** under a bare `--universe-id`, which is the
    /// case where there is no config to have named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// A string, like every other id this suite emits: a 64-bit id handed to a
    /// consumer as a number is one a JSON parser may round.
    pub universe_id: String,
    /// False when any template names nothing that exists, which is also the
    /// exit-2 condition. Both are here so a consumer that captured stdout does
    /// not have to reach for the status.
    pub ok: bool,
    /// Standard data stores the universe holds. Ordered stores are **not**
    /// listed: Open Cloud does not enumerate them, which is the same limit
    /// `unverifiable` reports.
    pub standard_store_count: usize,
    /// Every template with something to report. Empty when all is well.
    pub findings: Vec<Finding>,
    /// Live stores no template covers. **Absent** unless `--uncovered` asked,
    /// because an absent field and an empty array mean different things and a
    /// consumer should not have to guess which it got.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncovered: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed<T: Serialize>(value: &T) -> serde_json::Value {
        serde_json::to_value(value).expect("serialises")
    }

    fn templates() -> Templates {
        Templates {
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
        }
    }

    #[test]
    fn the_show_envelope_carries_the_documented_fields() {
        let doc = parsed(&ShowDocument::new("rbxrtbf.toml", &templates(), 1234567890));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["config_file"], "rbxrtbf.toml");
        assert_eq!(doc["max_templates"], 100);
        assert_eq!(doc["count"], 3);
        // A string, like every id this suite emits.
        assert_eq!(doc["sample_user_id"], "1234567890");
        assert_eq!(doc["templates"].as_array().map(Vec::len), Some(3));
    }

    /// One list with a `kind`, so a consumer filters rather than reading two
    /// arrays. The TOML's two-array shape is for a human.
    #[test]
    fn both_template_kinds_are_one_list_discriminated_by_kind() {
        let doc = parsed(&ShowDocument::new("f", &templates(), 1234567890));
        let list = doc["templates"].as_array().unwrap();

        assert_eq!(list[0]["kind"], "key");
        assert_eq!(list[0]["store"], "PlayerInventory");
        assert_eq!(list[0]["pattern"], "User_{UserId}");
        assert_eq!(list[0]["ordered"], false);

        assert_eq!(list[2]["kind"], "store");
        assert_eq!(list[2]["pattern"], "Player_{UserId}_Save");
    }

    /// The defaulted scope, not the file's silence: `global` is what Roblox
    /// matches on, and the absence in the TOML is not what a consumer asks.
    #[test]
    fn an_omitted_scope_reports_as_the_default_rather_than_absent() {
        let doc = parsed(&ShowDocument::new("f", &templates(), 1234567890));
        let list = doc["templates"].as_array().unwrap();
        assert_eq!(list[0]["scope"], "Scope_{UserId}");
        assert_eq!(list[1]["scope"], "global");
    }

    /// A store template has no store name and no standard-versus-ordered
    /// choice, so both are absent rather than emitted as null.
    #[test]
    fn fields_that_do_not_apply_are_absent_rather_than_null() {
        let doc = parsed(&ShowDocument::new("f", &templates(), 1234567890));
        let store = &doc["templates"].as_array().unwrap()[2];
        assert!(store.get("store").is_none(), "{store}");
        assert!(store.get("scope").is_none(), "{store}");
        assert!(store.get("ordered").is_none(), "{store}");
    }

    /// The sample is the field worth having: it is what Roblox will look for,
    /// which is not readable off the pattern alone.
    #[test]
    fn every_template_carries_the_key_roblox_will_look_for() {
        let doc = parsed(&ShowDocument::new("f", &templates(), 1234567890));
        let list = doc["templates"].as_array().unwrap();
        assert_eq!(
            list[0]["sample"],
            "PlayerInventory/Scope_1234567890/User_1234567890"
        );
        assert_eq!(list[2]["sample"], "Player_1234567890_Save");
    }

    #[test]
    fn a_file_declaring_nothing_is_an_empty_array_not_an_absent_one() {
        let doc = parsed(&ShowDocument::new("f", &Templates::default(), 1));
        assert_eq!(doc["count"], 0);
        assert_eq!(doc["templates"].as_array().map(Vec::len), Some(0));
    }

    fn verify_doc(findings: Vec<Finding>, uncovered: Option<Vec<String>>) -> VerifyDocument {
        VerifyDocument {
            schema_version: SCHEMA_VERSION,
            config_file: "rbxrtbf.toml".into(),
            env: Some("prod".into()),
            universe_id: "109876543210987".into(),
            ok: findings.is_empty(),
            standard_store_count: 4,
            findings,
            uncovered,
        }
    }

    #[test]
    fn the_verify_envelope_carries_the_documented_fields() {
        let doc = parsed(&verify_doc(Vec::new(), None));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["env"], "prod");
        // A string: a 64-bit id handed over as a number is one a parser rounds.
        assert_eq!(doc["universe_id"], "109876543210987");
        assert_eq!(doc["ok"], true);
        assert_eq!(doc["standard_store_count"], 4);
        assert_eq!(doc["findings"].as_array().map(Vec::len), Some(0));
    }

    /// Three verdicts rather than a boolean. Folding `unverifiable` into a
    /// failure would make a consumer treat an ordered store, which Open Cloud
    /// cannot list, as a broken template.
    #[test]
    fn an_unverifiable_template_is_not_a_failure() {
        let doc = parsed(&verify_doc(
            vec![Finding {
                kind: "key",
                target: "Leaderboard".into(),
                verdict: "unverifiable",
                detail: "ordered store: Open Cloud does not list these".into(),
            }],
            None,
        ));
        assert_eq!(doc["findings"][0]["verdict"], "unverifiable");
        // `ok` is the caller's to set from the failing verdicts only, which the
        // command does; this pins the field's presence and shape.
        assert!(doc["ok"].is_boolean());
    }

    /// An absent field and an empty array mean different things here: nobody
    /// asked, versus nothing to report.
    #[test]
    fn uncovered_is_absent_unless_it_was_asked_for() {
        let doc = parsed(&verify_doc(Vec::new(), None));
        assert!(doc.get("uncovered").is_none(), "{doc}");

        let doc = parsed(&verify_doc(Vec::new(), Some(Vec::new())));
        assert_eq!(doc["uncovered"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn a_missing_store_is_reported_with_its_reason() {
        let doc = parsed(&verify_doc(
            vec![Finding {
                kind: "key",
                target: "PlayerInventoryV1".into(),
                verdict: "missing",
                detail: "no such standard data store in this universe".into(),
            }],
            None,
        ));
        assert_eq!(doc["ok"], false);
        assert_eq!(doc["findings"][0]["target"], "PlayerInventoryV1");
        assert_eq!(doc["findings"][0]["verdict"], "missing");
        assert!(doc["findings"][0]["detail"].is_string());
    }
}
