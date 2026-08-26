//! What `rbx place --json` writes to stdout.
//!
//! Two kinds of document live here, and the second one is new to the tree.
//!
//! The read commands (`versions`, `places`) follow the pilots: a
//! `schema_version` first, then named objects all the way down, optional
//! fields omitted rather than emitted as `null`.
//!
//! The write commands (`upload`, `promote`, `rollback`) share one envelope,
//! `WriteDocument`, and it is deliberately a *receipt* rather than a status
//! line. Three decisions make it one:
//!
//! - **It reports what landed, not what was asked for.** `results` carries one
//!   entry per place that actually got a new version, with the version number
//!   Roblox assigned. That is the thing a pipeline cannot compute for itself
//!   and the reason the issue asks for these documents at all.
//! - **A run that fails partway still emits it**, with `ok` false and `error`
//!   set. `place upload --all-places` can write two places and then hit a Team
//!   Create lock on the third; those two writes happened, and a deploy log that
//!   loses them is worse than no log. The process still exits non-zero, so
//!   nobody reads success out of a document that says `"ok": false`.
//! - **The shape follows the invocation, never the data.** A single-target run
//!   fills the `place_id` and `version` shortcuts next to `results`; an
//!   `--all-places` run fills `results` only, even against an env with exactly
//!   one place. Same rule `rbx env get --json` uses for `value`, and for the
//!   same reason: a filter must not start working by accident and stop when a
//!   second place is added.
//!
//! That last rule is also why an `upload` that named several envs gets its own
//! envelope, [`MultiEnvWriteDocument`], instead of a plural `env` field: the
//! shape follows the invocation, so a single-env upload keeps emitting the
//! document it always has.
//!
//! Field names are documented in `docs/place.md` and are the compatibility
//! surface. Ids and version numbers are strings: they identify an asset, they
//! exceed 2^53 in the case of a place id, and a consumer that parses them as
//! JSON numbers would round them. Keeping versions in the same form means
//! `place upload --json | jq -r .version` feeds `rbx servers list --version`
//! without a conversion in between.

use std::collections::HashMap;

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::api::models::{PlaceEntry, VersionInfo};

/// One `place versions` invocation.
#[derive(Debug, Serialize)]
pub struct VersionsDocument {
    pub schema_version: u32,
    /// The env asked for, so a document captured out of a matrix job still
    /// says which leg produced it.
    pub env: String,
    /// The `rbxplace.toml` place name, after the `--place` defaulting rule.
    pub place: String,
    pub place_id: String,
    /// `all`, `published`, or `saved`: the `--filter` in force.
    pub filter: String,
    /// The `--count` in force, which is a maximum and not a promise.
    pub count: usize,
    /// True when the walk stopped because it hit `--count` rather than because
    /// the place ran out of versions. Raise `--count` to see the rest.
    pub count_reached: bool,
    /// Newest first, the order the human listing prints.
    pub versions: Vec<Version>,
}

/// One asset version of a place.
#[derive(Debug, Serialize)]
pub struct Version {
    /// The version number, as a string. `--version` and `--rollback` take it
    /// back verbatim.
    pub version: String,
    /// True for a version that is live, false for a draft. The human listing
    /// renders this as a `published` tag.
    pub published: bool,
    /// Exactly what Roblox sent, RFC 3339. The human listing rewrites it into
    /// `2024-01-15 14:30 UTC`; that is a rendering, and the document keeps the
    /// original so a consumer can parse a timestamp rather than a layout.
    pub create_time: String,
}

impl VersionsDocument {
    // `pub(crate)`, unlike the type: `VersionInfo` lives in the crate-private
    // `api` module, and a public constructor taking one would leak it.
    pub(crate) fn new(
        env: &str,
        place: &str,
        place_id: u64,
        filter: &str,
        count: usize,
        versions: &[VersionInfo],
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.to_string(),
            place: place.to_string(),
            place_id: place_id.to_string(),
            filter: filter.to_string(),
            count,
            count_reached: versions.len() >= count,
            versions: versions
                .iter()
                .map(|version| Version {
                    version: version.version_number.to_string(),
                    published: version.published,
                    create_time: version.create_time.clone(),
                })
                .collect(),
        }
    }
}

/// One `place places` invocation.
#[derive(Debug, Serialize)]
pub struct PlacesDocument {
    pub schema_version: u32,
    /// The env whose `rbxplace.toml` entry named the universe. **Absent** under
    /// a bare `--universe-id`, which is the case where there is no config to
    /// compare against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    pub universe_id: String,
    /// One object per place Roblox reports, in the order it returned them.
    pub places: Vec<Place>,
}

/// One place of a universe, as Roblox has it.
#[derive(Debug, Serialize)]
pub struct Place {
    /// **Absent** when the id could not be read out of the resource path,
    /// which the human listing renders as `?`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_id: Option<String>,
    /// The name Roblox shows, which is not the `rbxplace.toml` key.
    pub display_name: String,
    /// **Absent** when Roblox did not report one, which is every place from
    /// the universe listing this command uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_player_count: Option<u64>,
    /// The `rbxplace.toml` key this place is mapped to: the name `--place`
    /// takes. **Absent** when the file does not have it, and absent for every
    /// place under a bare `--universe-id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// Whether the file has this place. **Absent**, rather than false, under a
    /// bare `--universe-id`: with no config in play the question has no
    /// answer, and answering it false would report every place as missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured: Option<bool>,
}

impl PlacesDocument {
    /// `env` and `configured` travel together: both are `Some` exactly when the
    /// run resolved an env, because that is when there is a file to compare
    /// against.
    ///
    /// `pub(crate)` for the same reason as `VersionsDocument::new`.
    pub(crate) fn new(
        env: Option<&str>,
        universe_id: u64,
        entries: &[PlaceEntry],
        configured: Option<&HashMap<String, u64>>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.map(str::to_string),
            universe_id: universe_id.to_string(),
            places: entries
                .iter()
                .map(|entry| {
                    let id = entry.place_id();
                    let name = configured.and_then(|places| {
                        places
                            .iter()
                            .find(|(_, place_id)| Some(**place_id) == id)
                            .map(|(name, _)| name.clone())
                    });
                    Place {
                        place_id: id.map(|id| id.to_string()),
                        display_name: entry.display_name.clone(),
                        max_player_count: (entry.max_player_count > 0)
                            .then_some(entry.max_player_count),
                        configured: configured.map(|_| name.is_some()),
                        place: name,
                    }
                })
                .collect(),
        }
    }
}

/// Which write produced a `WriteDocument`.
///
/// Named rather than a bare string so the three commands cannot drift apart on
/// a typo, and so a consumer can dispatch on `.command` when it reads a mixed
/// stream of deploy receipts.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteCommand {
    Upload,
    Promote,
    Rollback,
}

/// What one `place upload`, `promote`, or `rollback` did.
///
/// Emitted once the run has started writing, after the confirmation gate, on
/// the way out, whether or not every target succeeded. A failure before that
/// point (a missing env, a refused confirmation, a source version that does not
/// exist) writes nothing to stdout at all: nothing happened, and an empty
/// stdout next to a non-zero exit says so without ambiguity.
#[derive(Debug, Serialize)]
pub struct WriteDocument {
    pub schema_version: u32,
    pub command: WriteCommand,
    /// False when a target failed. The process exit code says the same thing;
    /// this is here so a consumer that already captured stdout does not have to
    /// plumb `$?` through as well, the way `rbx check --json` carries
    /// `exit_code`.
    pub ok: bool,
    /// The env that was written to. For `promote`, the target, `from_env`
    /// carries the other one.
    pub env: String,
    /// The source env of a `promote`. **Absent** for `upload` and `rollback`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_env: Option<String>,
    /// The universe that was written to.
    pub universe_id: String,
    /// Whether the new versions are live. `--published` sets it; without the
    /// flag the write is a draft. `rollback` is always true: rolling back
    /// publishes.
    pub published: bool,
    /// The `rbxplace.toml` place name the bytes came from, after `--place`
    /// defaulting. **Absent** outside `promote`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_place: Option<String>,
    /// The place id the bytes came from. **Absent** outside `promote`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_place_id: Option<String>,
    /// The version the new one was made from: the promoted source version, or
    /// the version rolled back to. **Absent** for `upload`, whose source is a
    /// local file. Resolved, so `--from-published` and a bare latest report the
    /// number they actually picked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    /// The single-target shortcut: the place that was written. **Absent**
    /// under `--all-places`, and absent when the run wrote nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_id: Option<String>,
    /// The version Roblox assigned, for the same single-target form. This is
    /// the field a promote log wants and the reason these documents exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// One entry per place that got a new version, in the order they were
    /// written. Empty when the first target failed.
    pub results: Vec<WriteResult>,
    /// Why the run stopped. **Absent** when `ok` is true. The same text goes to
    /// stderr, where it is the process's error message; it is repeated here so
    /// a consumer reading only stdout can report the cause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Not serialized: it is how the document was asked for, not part of what
    /// it says. Kept on the struct so `landed` can fill the shortcut without
    /// every call site repeating the rule.
    #[serde(skip)]
    single: bool,
}

/// One place that received a new version.
#[derive(Debug, Serialize)]
pub struct WriteResult {
    /// The `rbxplace.toml` key.
    pub place: String,
    pub place_id: String,
    /// The version Roblox assigned to this write.
    pub version: String,
}

impl WriteDocument {
    /// Start a receipt. `single` is the invocation's shape, not the data's:
    /// true when the command targets one place by construction, false under
    /// `--all-places` however many places that turns out to be.
    pub fn new(
        command: WriteCommand,
        env: &str,
        universe_id: u64,
        published: bool,
        single: bool,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            ok: true,
            env: env.to_string(),
            from_env: None,
            universe_id: universe_id.to_string(),
            published,
            source_place: None,
            source_place_id: None,
            source_version: None,
            place_id: None,
            version: None,
            results: Vec::new(),
            error: None,
            single,
        }
    }

    /// Record where a `promote` read its bytes.
    pub fn promoted_from(mut self, env: &str, place: &str, place_id: u64, version: u64) -> Self {
        self.from_env = Some(env.to_string());
        self.source_place = Some(place.to_string());
        self.source_place_id = Some(place_id.to_string());
        self.source_version = Some(version.to_string());
        self
    }

    /// Record the version a `rollback` restored.
    pub fn rolled_back_to(mut self, version: u64) -> Self {
        self.source_version = Some(version.to_string());
        self
    }

    /// A place got a new version. Called as each write lands, so a run that
    /// stops halfway still reports the half that happened.
    pub fn landed(&mut self, place: &str, place_id: u64, version: u64) {
        if self.single {
            self.place_id = Some(place_id.to_string());
            self.version = Some(version.to_string());
        }
        self.results.push(WriteResult {
            place: place.to_string(),
            place_id: place_id.to_string(),
            version: version.to_string(),
        });
    }

    /// Close the receipt on a failure. `{:#}` rather than `{}` so the context
    /// chain survives, matching what the binary prints to stderr.
    pub fn failed(mut self, error: &anyhow::Error) -> Self {
        self.ok = false;
        self.error = Some(format!("{error:#}"));
        self
    }
}

/// What one `place upload` that named several envs did, one [`WriteDocument`]
/// per env.
///
/// A separate envelope rather than a plural `env` on [`WriteDocument`].
/// `promote` and `rollback` share that struct and act on one env by
/// construction, and every consumer already reads its `env` and `universe_id`
/// as scalars: widening them would break those readers in order to describe a
/// case they never asked about. So a single-env upload keeps emitting exactly
/// the document it always has, and only `--env all` (or a group) gets this
/// shape. `rbx check --json` sets the precedent: one document naming several
/// envs, each carrying its own per-env result.
///
/// `results` is in target order and stops where the run stopped. The walk
/// halts at the first env that fails, so the envs before it keep their
/// receipts, that env carries its own `error`, and the ones after it are
/// absent rather than reported as anything.
#[derive(Debug, Serialize)]
pub struct MultiEnvWriteDocument {
    pub schema_version: u32,
    pub command: WriteCommand,
    /// False when any env failed. Read off the per-env receipts rather than
    /// tracked separately, so the two cannot disagree.
    pub ok: bool,
    pub results: Vec<WriteDocument>,
}

impl MultiEnvWriteDocument {
    pub fn new(command: WriteCommand, results: Vec<WriteDocument>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            ok: results.iter().all(|receipt| receipt.ok),
            results,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(document: &impl Serialize) -> serde_json::Value {
        let mut buf = Vec::new();
        rbx_core::output::write_json(&mut buf, document).expect("write");
        serde_json::from_slice(&buf).expect("the document must be valid JSON")
    }

    fn version(number: u64, published: bool) -> VersionInfo {
        VersionInfo {
            version_number: number,
            create_time: format!("2024-01-0{number}T00:00:00Z"),
            published,
        }
    }

    fn entry(id: u64, name: &str, max: u64) -> PlaceEntry {
        PlaceEntry {
            path: format!("places/{id}"),
            display_name: name.to_string(),
            max_player_count: max,
        }
    }

    /// One env's finished upload receipt, the shape the fan-out envelope holds.
    fn upload_receipt(env: &str, place_id: u64, version: u64) -> WriteDocument {
        let mut doc = WriteDocument::new(WriteCommand::Upload, env, place_id * 10, false, true);
        doc.landed("main", place_id, version);
        doc
    }

    #[test]
    fn the_versions_envelope_carries_the_documented_fields() {
        let doc = parsed(&VersionsDocument::new(
            "prod",
            "main",
            123_456_789_012_345,
            "all",
            20,
            &[version(5, true), version(4, false)],
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["env"], "prod");
        assert_eq!(doc["place"], "main");
        assert_eq!(doc["place_id"], "123456789012345");
        assert_eq!(doc["filter"], "all");
        assert_eq!(doc["count"], 20);
        assert_eq!(doc["count_reached"], false);
        assert_eq!(doc["versions"][0]["version"], "5");
        assert_eq!(doc["versions"][0]["published"], true);
        assert_eq!(doc["versions"][1]["published"], false);
    }

    /// The human listing prints `2024-01-05 00:00 UTC`. That is a layout; the
    /// document keeps what Roblox sent so a consumer parses a timestamp.
    #[test]
    fn a_create_time_is_the_api_timestamp_and_not_the_human_rendering() {
        let doc = parsed(&VersionsDocument::new(
            "prod",
            "main",
            1,
            "all",
            20,
            &[version(5, true)],
        ));

        assert_eq!(doc["versions"][0]["create_time"], "2024-01-05T00:00:00Z");
    }

    #[test]
    fn hitting_the_count_is_reported_rather_than_left_to_be_inferred() {
        let rows = [version(5, true), version(4, true)];
        assert!(VersionsDocument::new("prod", "main", 1, "all", 2, &rows).count_reached);
        assert!(!VersionsDocument::new("prod", "main", 1, "all", 3, &rows).count_reached);
    }

    /// A place with no versions is a fact, not a failure: an empty array a
    /// consumer reads a zero off.
    #[test]
    fn no_versions_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&VersionsDocument::new("dev", "main", 1, "saved", 3, &[]));

        assert_eq!(doc["versions"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["count_reached"], false);
    }

    #[test]
    fn a_place_listing_maps_ids_back_onto_the_configured_names() {
        let configured = HashMap::from([("main".to_string(), 1001_u64)]);
        let doc = parsed(&PlacesDocument::new(
            Some("dev"),
            100,
            &[entry(1001, "Main", 50), entry(1002, "Lobby", 0)],
            Some(&configured),
        ));

        assert_eq!(doc["env"], "dev");
        assert_eq!(doc["universe_id"], "100");
        assert_eq!(doc["places"][0]["place_id"], "1001");
        assert_eq!(doc["places"][0]["display_name"], "Main");
        assert_eq!(doc["places"][0]["max_player_count"], 50);
        assert_eq!(doc["places"][0]["place"], "main");
        assert_eq!(doc["places"][0]["configured"], true);
        // Present on Roblox, missing from the file: the case the human listing
        // marks `NOT in toml`.
        assert_eq!(doc["places"][1]["configured"], false);
        assert!(doc["places"][1].get("place").is_none());
        // Zero is "Roblox did not say", not a real cap.
        assert!(doc["places"][1].get("max_player_count").is_none());
    }

    /// With no env there is no file to compare against, so `configured` is
    /// absent rather than false: reporting every place as missing would be a
    /// lie a monitoring script could act on.
    #[test]
    fn a_universe_id_listing_omits_the_configured_question_entirely() {
        let doc = parsed(&PlacesDocument::new(
            None,
            100,
            &[entry(1001, "Main", 0)],
            None,
        ));

        assert!(doc.get("env").is_none());
        assert!(doc["places"][0].get("configured").is_none());
        assert!(doc["places"][0].get("place").is_none());
    }

    #[test]
    fn an_upload_reports_the_version_roblox_assigned() {
        let mut doc = WriteDocument::new(WriteCommand::Upload, "prod", 100, false, true);
        doc.landed("main", 1001, 42);
        let doc = parsed(&doc);

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["command"], "upload");
        assert_eq!(doc["ok"], true);
        assert_eq!(doc["env"], "prod");
        assert_eq!(doc["universe_id"], "100");
        assert_eq!(doc["published"], false);
        assert_eq!(doc["place_id"], "1001");
        assert_eq!(doc["version"], "42");
        assert_eq!(doc["results"][0]["place"], "main");
        assert_eq!(doc["results"][0]["place_id"], "1001");
        assert_eq!(doc["results"][0]["version"], "42");
        assert!(doc.get("error").is_none());
        assert!(doc.get("from_env").is_none());
        assert!(doc.get("source_version").is_none());
    }

    /// The rule that keeps a filter honest: `--all-places` fills `results`
    /// only, even when the env has exactly one place, so a script written
    /// against today's file does not break when a second place lands.
    #[test]
    fn all_places_omits_the_shortcut_even_for_one_place() {
        let mut doc = WriteDocument::new(WriteCommand::Upload, "prod", 100, true, false);
        doc.landed("main", 1001, 42);
        let doc = parsed(&doc);

        assert!(doc.get("place_id").is_none(), "{doc}");
        assert!(doc.get("version").is_none(), "{doc}");
        assert_eq!(doc["results"].as_array().map(Vec::len), Some(1));
        assert_eq!(doc["results"][0]["version"], "42");
        assert_eq!(doc["published"], true);
    }

    /// The case the whole envelope is shaped around: two places written, the
    /// third locked. Those two versions exist, and a deploy log that loses them
    /// is worse than no log.
    #[test]
    fn a_partial_run_keeps_what_landed_and_says_it_did_not_finish() {
        let mut doc = WriteDocument::new(WriteCommand::Upload, "prod", 100, false, false);
        doc.landed("lobby", 1002, 7);
        doc.landed("main", 1001, 42);
        let doc = parsed(&doc.failed(&anyhow::anyhow!(
            "Place 1003 is locked by an active Team Create session."
        )));

        assert_eq!(doc["ok"], false);
        assert_eq!(doc["results"].as_array().map(Vec::len), Some(2));
        assert_eq!(doc["results"][0]["place"], "lobby");
        assert_eq!(doc["results"][1]["version"], "42");
        assert!(
            doc["error"]
                .as_str()
                .is_some_and(|e| e.contains("Team Create")),
            "{doc}"
        );
    }

    #[test]
    fn a_failure_before_the_first_write_is_a_document_with_no_results() {
        let doc = parsed(
            &WriteDocument::new(WriteCommand::Upload, "prod", 100, false, true)
                .failed(&anyhow::anyhow!("boom")),
        );

        assert_eq!(doc["ok"], false);
        assert_eq!(doc["results"].as_array().map(Vec::len), Some(0));
        // Nothing landed, so there is no version to shortcut to.
        assert!(doc.get("version").is_none(), "{doc}");
    }

    #[test]
    fn a_promote_carries_both_ends_of_the_move() {
        let mut doc = WriteDocument::new(WriteCommand::Promote, "prod", 200, true, true)
            .promoted_from("staging", "main", 1001, 172);
        doc.landed("main", 2001, 27);
        let doc = parsed(&doc);

        assert_eq!(doc["command"], "promote");
        assert_eq!(doc["from_env"], "staging");
        assert_eq!(doc["env"], "prod");
        assert_eq!(doc["source_place"], "main");
        assert_eq!(doc["source_place_id"], "1001");
        assert_eq!(doc["source_version"], "172");
        assert_eq!(doc["place_id"], "2001");
        assert_eq!(doc["version"], "27");
    }

    #[test]
    fn a_rollback_names_the_version_it_restored_and_the_one_it_created() {
        let mut doc =
            WriteDocument::new(WriteCommand::Rollback, "prod", 100, true, true).rolled_back_to(37);
        doc.landed("main", 1001, 44);
        let doc = parsed(&doc);

        assert_eq!(doc["command"], "rollback");
        assert_eq!(doc["source_version"], "37");
        assert_eq!(doc["version"], "44");
        // Rolling back publishes; the field says so rather than leaving a
        // consumer to know it.
        assert_eq!(doc["published"], true);
    }

    /// Ids and versions are strings everywhere, so nothing rounds a 64-bit
    /// place id and one filter reads a version out of any of these documents.
    #[test]
    fn every_id_and_version_is_a_string() {
        let mut doc = WriteDocument::new(WriteCommand::Upload, "prod", 100, false, true);
        doc.landed("main", 123_456_789_012_345, 42);
        let doc = parsed(&doc);

        assert!(doc["universe_id"].is_string(), "{doc}");
        assert!(doc["place_id"].is_string(), "{doc}");
        assert!(doc["version"].is_string(), "{doc}");
        assert!(doc["results"][0]["place_id"].is_string(), "{doc}");
        assert!(doc["results"][0]["version"].is_string(), "{doc}");
    }

    /// The exact bytes a one-env `place upload --json` puts on stdout.
    ///
    /// Spelled out rather than asserted field by field, because this document
    /// is the compatibility surface and fan-out was the change most likely to
    /// widen it by accident. A field added here breaks this test on purpose:
    /// the answer is a new envelope beside it, which is what
    /// [`MultiEnvWriteDocument`] is.
    #[test]
    fn a_single_env_upload_emits_the_document_it_always_has() {
        let mut doc = WriteDocument::new(
            WriteCommand::Upload,
            "prod",
            109_876_543_210_987,
            false,
            true,
        );
        doc.landed("main", 109_876_543_210_988, 42);

        let mut buf = Vec::new();
        rbx_core::output::write_json(&mut buf, &doc).expect("write");

        assert_eq!(
            String::from_utf8(buf).expect("utf-8"),
            concat!(
                "{\n",
                "  \"schema_version\": 1,\n",
                "  \"command\": \"upload\",\n",
                "  \"ok\": true,\n",
                "  \"env\": \"prod\",\n",
                "  \"universe_id\": \"109876543210987\",\n",
                "  \"published\": false,\n",
                "  \"place_id\": \"109876543210988\",\n",
                "  \"version\": \"42\",\n",
                "  \"results\": [\n",
                "    {\n",
                "      \"place\": \"main\",\n",
                "      \"place_id\": \"109876543210988\",\n",
                "      \"version\": \"42\"\n",
                "    }\n",
                "  ]\n",
                "}\n",
            )
        );
    }

    /// One receipt per env, in the order they were written, under an envelope
    /// that keeps the same `schema_version` and `command` a reader already
    /// dispatches on.
    #[test]
    fn a_fan_out_carries_one_receipt_per_env_in_target_order() {
        let doc = parsed(&MultiEnvWriteDocument::new(
            WriteCommand::Upload,
            vec![upload_receipt("dev", 11, 1), upload_receipt("prod", 22, 7)],
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["command"], "upload");
        assert_eq!(doc["ok"], true);
        assert_eq!(doc["results"].as_array().map(Vec::len), Some(2));
        assert_eq!(doc["results"][0]["env"], "dev");
        assert_eq!(doc["results"][0]["version"], "1");
        assert_eq!(doc["results"][1]["env"], "prod");
        assert_eq!(doc["results"][1]["version"], "7");
    }

    /// The fan-out case of the rule the whole envelope is shaped around: the
    /// second env failed, and the first one's version exists whatever happens
    /// next. A deploy log that loses it is worse than no log.
    #[test]
    fn a_fan_out_that_fails_partway_keeps_the_envs_that_landed() {
        let failed = upload_receipt("prod", 22, 7).failed(&anyhow::anyhow!(
            "Place 22 is locked by an active Team Create session."
        ));
        let doc = parsed(&MultiEnvWriteDocument::new(
            WriteCommand::Upload,
            vec![upload_receipt("dev", 11, 1), failed],
        ));

        assert_eq!(doc["ok"], false);
        assert_eq!(doc["results"].as_array().map(Vec::len), Some(2));
        // The env that landed keeps its version and its own `ok`.
        assert_eq!(doc["results"][0]["env"], "dev");
        assert_eq!(doc["results"][0]["ok"], true);
        assert_eq!(doc["results"][0]["version"], "1");
        // The env that failed carries the reason, next to what it had written
        // before it stopped.
        assert_eq!(doc["results"][1]["ok"], false);
        assert!(
            doc["results"][1]["error"]
                .as_str()
                .is_some_and(|e| e.contains("Team Create")),
            "{doc}"
        );
        // The envs after the failure are absent rather than reported as
        // anything: the walk stopped, so there is nothing to say about them.
        assert!(doc["results"][2].is_null(), "{doc}");
    }
}
