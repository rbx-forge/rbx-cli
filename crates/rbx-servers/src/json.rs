//! What `rbx servers versions|list|logs --json` write to stdout.
//!
//! Separate from `model` on purpose. `model` describes what Roblox sends, down
//! to the .NET TimeSpan text and the PascalCase filter map; this describes what
//! we promise, and the two are allowed to drift. A field renamed upstream is a
//! parsing change here, not a break in somebody's `jq` filter.
//!
//! The envelope follows `rbx check --json`: a `schema_version` first, a
//! `totals` object, then the rows. Field names are documented in
//! `docs/ops/servers.md` and are the compatibility surface.
//!
//! # Why `logs` is a document and not JSON Lines
//!
//! A log stream is the one thing here that could reasonably be emitted a line
//! at a time, so the choice is worth stating rather than leaving to be
//! inferred. `servers logs` is not a tail: it reads a bounded slice of a log
//! Roblox has already finished writing, it has no `--follow`, and it has
//! nothing to print until pagination has stopped at `--limit`. Streaming would
//! therefore buy no earlier output at all, and would cost the envelope, which
//! job id, which place version, which severity filter, whether `--limit`
//! truncated the answer, since a line stream has nowhere to put a fact about
//! the run. One document per invocation is also what every other `--json` in
//! this tool emits, so one filter shape reads them all.
//!
//! A consumer that wants a line stream gets one with `jq -c '.lines[]'`. The
//! day a `--follow` exists, JSON Lines is what it should emit, and that is a
//! new mode rather than a change to this one.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::model::{GameServer, GameServerLog, ServerStatus};

/// One `servers versions` invocation.
///
/// The list `list` and `logs` cannot be used without: `ListGameServers` takes a
/// place version in its path and offers no "every version" form, so a script
/// reads this first and feeds a version to the next call.
#[derive(Debug, Serialize)]
pub struct VersionsDocument {
    pub schema_version: u32,
    /// The version `list` and `logs` use when `--version` is not given, which
    /// is the newest one: the entry the human listing marks with `*`. Named
    /// rather than left as "index 0" so a consumer does not have to know that
    /// the order is newest first to pick the right one. **Absent** when no
    /// version has servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_place_version: Option<String>,
    /// Every place version that has servers in the 30-day window, newest
    /// first. Strings, because that is what the other two documents call a
    /// place version and what the API path takes.
    pub place_versions: Vec<String>,
}

impl VersionsDocument {
    /// Build the document from the versions `filter_options` reported, which
    /// the caller has already ordered newest first.
    pub fn new(versions: Vec<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            default_place_version: versions.first().cloned(),
            place_versions: versions,
        }
    }
}

/// One `servers list` invocation.
#[derive(Debug, Serialize)]
pub struct ServersDocument {
    pub schema_version: u32,
    /// The place version the rows are for. **Absent** when no place version
    /// has servers at all, which is the one case where there was nothing to
    /// list rather than nothing matching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_version: Option<String>,
    /// True when Roblox answered 200 while reporting a fetch error for one of
    /// its two sources. The rows are then a subset of what ran, so any rate
    /// computed from them is a lower bound. Ignoring this field is how a crash
    /// rate ends up quietly wrong.
    pub partial: bool,
    /// The `--limit` in force for this run.
    pub limit: u32,
    /// True when the run stopped because it hit `--limit` rather than because
    /// it ran out of rows. Raise `--limit` to see the rest.
    pub limit_reached: bool,
    pub totals: Totals,
    /// One object per server, in the order Roblox returned them.
    pub servers: Vec<Server>,
}

#[derive(Debug, Serialize)]
pub struct Totals {
    /// Rows in `servers`: fetched, then filtered by `--status`.
    pub returned: usize,
    /// How many of `returned` ended in a crash or out-of-memory.
    pub failed: usize,
    /// How many servers exist for this place version before `--status` and
    /// `--limit`, as Roblox counts them. **Absent** when Roblox did not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available: Option<i64>,
}

/// One server.
///
/// Every field Roblox returns, like the CSV and unlike the table: a terminated
/// server is gone from Roblox after thirty days and nothing brings it back, so
/// what this emits is what a scheduled export gets to keep.
///
/// Nulls are omitted rather than emitted, so `has("frame_rate")` is a usable
/// test, and it needs to be, because a null frame rate (never reported) and a
/// zero one (reported, stalled) are different facts.
#[derive(Debug, Serialize)]
pub struct Server {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// `active`, `shut_down`, `crashed`, `out_of_memory`, `restarted`,
    /// `roblox_restarted`, `moderated`, or `unknown` for a status this build
    /// has never seen. Same spelling `--status` takes.
    pub status: ServerStatus,
    /// True for the statuses that mean the server stopped for a bad reason,
    /// so a consumer does not have to keep its own list of which those are.
    pub failure: bool,
    /// Ids, as strings. Roblox sends them as strings and they exceed 2^53, so
    /// they stay strings rather than becoming JSON numbers a JavaScript
    /// consumer would silently round.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_time: Option<String>,
    /// Seconds, not the .NET TimeSpan text Roblox sends. Absent when the
    /// uptime was missing or did not parse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_rate: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occupancy: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_occupancy: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shut_down: Option<bool>,
    /// Integer enum 0..=5 in the spec, named nowhere. Only 1 and 3 were ever
    /// observed. Passed through raw rather than guessed at.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<i32>,
    /// How many players the server reported. Absent when it reported no list
    /// at all, which is not the same as an empty one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_count: Option<usize>,
    /// The player ids themselves, which the CSV drops for width. Absent when
    /// Roblox sent no list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_ids: Option<Vec<i64>>,
}

impl From<&GameServer> for Server {
    fn from(server: &GameServer) -> Self {
        let status = server.status();
        Self {
            job_id: server.job_id.clone(),
            failure: status.is_failure(),
            status,
            place_id: server.place_id.clone(),
            place_version: server.place_version.clone(),
            engine_version: server.engine_version.clone(),
            create_time: server.create_time.clone(),
            termination_time: server.termination_time.clone(),
            uptime_seconds: server.uptime_duration().map(|d| d.as_secs()),
            memory_bytes: server.memory_usage_bytes,
            frame_rate: server.frame_rate,
            occupancy: server.occupancy,
            max_occupancy: server.max_occupancy,
            full: server.full,
            shut_down: server.shut_down,
            kind: server.r#type,
            player_count: server.player_ids.as_ref().map(Vec::len),
            player_ids: server.player_ids.clone(),
        }
    }
}

impl ServersDocument {
    /// Build the document from what `list` gathered.
    ///
    /// Pure, and deliberately so: the renderer prints, this decides what the
    /// document says, and a test can therefore assert the shape without a
    /// process to capture.
    pub fn new(
        rows: &[GameServer],
        version: Option<String>,
        available: Option<i64>,
        limit: u32,
        partial: bool,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            place_version: version,
            partial,
            limit,
            limit_reached: rows.len() as u32 >= limit,
            totals: Totals {
                returned: rows.len(),
                failed: rows.iter().filter(|s| s.status().is_failure()).count(),
                available,
            },
            servers: rows.iter().map(Server::from).collect(),
        }
    }
}

/// One `servers logs` invocation: the whole slice that was fetched, plus the
/// facts about the run that a single line has nowhere to carry.
///
/// See the module comment for why this is one document rather than a line per
/// entry.
#[derive(Debug, Serialize)]
pub struct LogsDocument {
    pub schema_version: u32,
    /// The job id asked for, in full. Present even when nothing came back, so
    /// a document that answers "no lines" still says what it answered about.
    pub job_id: String,
    /// The place version the logs were read from, whether it was given with
    /// `--version` or defaulted to the newest. A job id queried against the
    /// wrong version returns nothing and no error, so the version the run
    /// actually used belongs in the document.
    pub place_version: String,
    /// The `--severity` in force, in its canonical spelling: `output`, `info`,
    /// `warn` or `error`. **Absent** when no filter was asked for, which is
    /// not the same as a filter that matched everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity_filter: Option<&'static str>,
    /// The `--limit` in force for this run.
    pub limit: u32,
    /// True when the run stopped because it hit `--limit` rather than because
    /// the server had nothing more to say. A crash investigation that reads
    /// the last line of a truncated slice is reading the wrong line.
    pub limit_reached: bool,
    pub totals: LogTotals,
    /// One object per line, in the order Roblox returned them.
    pub lines: Vec<LogLine>,
}

#[derive(Debug, Serialize)]
pub struct LogTotals {
    /// Rows in `lines`: fetched, then filtered by `--severity`.
    pub returned: usize,
    /// How many of `returned` are errors, so an alert can branch without
    /// knowing which severity spelling means trouble.
    pub errors: usize,
}

/// One line a server wrote.
///
/// `job_id` and `place_version` repeat what the envelope says because Roblox
/// sends them per line and the CSV export carries them as columns: this stays a
/// superset of `--csv`, so nothing is lost by choosing JSON.
#[derive(Debug, Serialize)]
pub struct LogLine {
    /// The timestamp exactly as Roblox sends it, RFC 3339 despite the
    /// `messageTimestampMs` name on the wire. **Absent** when a line carried
    /// none; the human renderer prints `--:--:--` for the same case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// `output`, `info`, `warn`, `error`, or `unknown` for a severity this
    /// build has never seen. Same spelling `--severity` takes, and the same
    /// `unknown` convention `servers list` uses for a status.
    pub severity: &'static str,
    /// The raw integer Roblox sent. Kept because `unknown` covers two
    /// different facts and this tells them apart: **absent** means the line
    /// carried no severity at all, a present number this build does not name
    /// means Roblox has added one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity_code: Option<i32>,
    /// True for `error` alone, mirroring `servers list`'s `failure`.
    pub error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Present on errors, and never truncated: after a crash this is the whole
    /// reason for running the command. Newlines are real newlines inside the
    /// JSON string rather than the quoted mess CSV has to make of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_version: Option<String>,
}

/// The name a document gives a severity code.
///
/// Deliberately not [`GameServerLog::severity_name`], which answers `?` for a
/// code it does not know. `?` is a glyph for a table column; a document needs a
/// word a filter can compare, and `unknown` is the one `servers list` already
/// uses for a status this build has not seen.
fn severity_label(code: Option<i32>) -> &'static str {
    match code {
        Some(0) => "output",
        Some(1) => "info",
        Some(2) => "warn",
        Some(3) => "error",
        _ => "unknown",
    }
}

impl From<&GameServerLog> for LogLine {
    fn from(line: &GameServerLog) -> Self {
        Self {
            time: line.message_timestamp_ms.clone(),
            severity: severity_label(line.severity),
            severity_code: line.severity,
            error: line.is_error(),
            message: line.message.clone(),
            // An empty trace is dropped rather than emitted as "": the human
            // renderer already treats the two the same, and `has("stack_trace")`
            // is only a useful test if it means there is one to read.
            stack_trace: line.stack_trace.clone().filter(|trace| !trace.is_empty()),
            job_id: line.job_id.clone(),
            place_version: line.place_version.clone(),
        }
    }
}

impl LogsDocument {
    /// Build the document from what `logs` gathered.
    ///
    /// `severity` is the code the CLI parsed, not the text the caller typed, so
    /// `--severity WARNING` and `--severity warn` produce the same document.
    pub fn new(
        lines: &[GameServerLog],
        job_id: String,
        place_version: String,
        severity: Option<i32>,
        limit: u32,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            job_id,
            place_version,
            severity_filter: severity.map(|code| severity_label(Some(code))),
            limit,
            limit_reached: lines.len() as u32 >= limit,
            totals: LogTotals {
                returned: lines.len(),
                errors: lines.iter().filter(|line| line.is_error()).count(),
            },
            lines: lines.iter().map(LogLine::from).collect(),
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

    fn page(json: &str) -> Vec<GameServer> {
        let page: crate::model::GameServerPage = serde_json::from_str(json).expect("fixture");
        page.game_servers
    }

    fn log_page(json: &str) -> Vec<GameServerLog> {
        let page: crate::model::GameServerLogPage = serde_json::from_str(json).expect("fixture");
        page.game_server_logs
    }

    #[test]
    fn the_envelope_carries_the_documented_fields() {
        let rows = page(
            r#"{"gameServers":[
                {"jobId":"a","status":"crashed","uptime":"00:05:02.0020000",
                 "memoryUsageBytes":1024,"frameRate":0,"occupancy":3,"maxOccupancy":50,
                 "playerIds":[1,2,3]},
                {"jobId":"b","status":"active","frameRate":59.5}
            ]}"#,
        );

        let doc = parsed(&ServersDocument::new(
            &rows,
            Some("3991".into()),
            Some(83_711),
            50,
            false,
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["place_version"], "3991");
        assert_eq!(doc["partial"], false);
        assert_eq!(doc["limit"], 50);
        assert_eq!(doc["limit_reached"], false);
        assert_eq!(doc["totals"]["returned"], 2);
        assert_eq!(doc["totals"]["failed"], 1);
        assert_eq!(doc["totals"]["available"], 83_711);
        assert_eq!(doc["servers"][0]["job_id"], "a");
        assert_eq!(doc["servers"][0]["status"], "crashed");
        assert_eq!(doc["servers"][0]["failure"], true);
        assert_eq!(doc["servers"][0]["uptime_seconds"], 302);
        assert_eq!(doc["servers"][0]["memory_bytes"], 1024);
        assert_eq!(doc["servers"][0]["player_count"], 3);
        assert_eq!(doc["servers"][0]["player_ids"][2], 3);
        assert_eq!(doc["servers"][1]["failure"], false);
    }

    /// The distinction the human renderer prints as `-`: never reported is not
    /// the same as reported zero, and the document has to keep them apart.
    #[test]
    fn a_null_frame_rate_is_omitted_and_a_zero_one_is_emitted() {
        let rows = page(r#"{"gameServers":[{"frameRate":null},{"frameRate":0}]}"#);
        let doc = parsed(&ServersDocument::new(&rows, None, None, 50, false));

        assert!(doc["servers"][0].get("frame_rate").is_none());
        assert_eq!(doc["servers"][1]["frame_rate"], 0.0);
    }

    /// A place version with no servers is not an error, so it is a document
    /// with no rows rather than a missing document.
    #[test]
    fn no_rows_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&ServersDocument::new(&[], None, None, 50, false));

        assert_eq!(doc["servers"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["totals"]["returned"], 0);
        assert!(doc.get("place_version").is_none());
        assert!(doc["totals"].get("available").is_none());
    }

    /// The whole point of `--json`: the warning about a partial page goes to
    /// stderr, so stdout still parses, and the fact is in the document too.
    #[test]
    fn a_partial_page_stays_parsable_and_says_so_in_the_document() {
        let rows = page(r#"{"gameServers":[{"jobId":"a","status":"active"}]}"#);
        let doc = parsed(&ServersDocument::new(
            &rows,
            Some("3991".into()),
            None,
            50,
            true,
        ));

        assert_eq!(doc["partial"], true);
        assert_eq!(doc["servers"][0]["job_id"], "a");
    }

    #[test]
    fn hitting_the_limit_is_reported_rather_than_left_to_be_inferred() {
        let rows = page(r#"{"gameServers":[{"jobId":"a"},{"jobId":"b"}]}"#);
        assert!(ServersDocument::new(&rows, None, None, 2, false).limit_reached);
        assert!(!ServersDocument::new(&rows, None, None, 3, false).limit_reached);
    }

    #[test]
    fn the_versions_envelope_names_the_default_rather_than_implying_it() {
        let doc = parsed(&VersionsDocument::new(vec![
            "3991".to_string(),
            "3982".to_string(),
        ]));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["default_place_version"], "3991");
        assert_eq!(doc["place_versions"][0], "3991");
        assert_eq!(doc["place_versions"][1], "3982");
    }

    /// Nothing has run in thirty days. Not an error: an empty list, and no
    /// default to point at rather than an invented one.
    #[test]
    fn no_version_with_servers_is_an_empty_list_and_no_default() {
        let doc = parsed(&VersionsDocument::new(Vec::new()));

        assert_eq!(doc["place_versions"].as_array().map(Vec::len), Some(0));
        assert!(doc.get("default_place_version").is_none(), "{doc}");
    }

    #[test]
    fn the_logs_envelope_carries_the_run_and_the_lines() {
        let lines = log_page(
            r#"{"gameServerLogs":[
                {"messageTimestampMs":"2025-08-14T13:46:53.481Z","severity":3,
                 "jobId":"aba9aeae","placeVersion":"3982",
                 "message":"attempt to index nil","stackTrace":"Stack Begin\nStack End"},
                {"messageTimestampMs":"2025-08-14T13:47:01.002Z","severity":0,
                 "message":"hello"}
            ]}"#,
        );

        let doc = parsed(&LogsDocument::new(
            &lines,
            "aba9aeae".to_string(),
            "3982".to_string(),
            None,
            200,
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["job_id"], "aba9aeae");
        assert_eq!(doc["place_version"], "3982");
        assert_eq!(doc["limit"], 200);
        assert_eq!(doc["limit_reached"], false);
        assert_eq!(doc["totals"]["returned"], 2);
        assert_eq!(doc["totals"]["errors"], 1);
        assert!(doc.get("severity_filter").is_none(), "{doc}");
        assert_eq!(doc["lines"][0]["time"], "2025-08-14T13:46:53.481Z");
        assert_eq!(doc["lines"][0]["severity"], "error");
        assert_eq!(doc["lines"][0]["severity_code"], 3);
        assert_eq!(doc["lines"][0]["error"], true);
        assert_eq!(doc["lines"][0]["message"], "attempt to index nil");
        // Real newlines inside the string, not the quoted mess CSV makes.
        assert_eq!(doc["lines"][0]["stack_trace"], "Stack Begin\nStack End");
        assert_eq!(doc["lines"][0]["job_id"], "aba9aeae");
        assert_eq!(doc["lines"][0]["place_version"], "3982");
        assert_eq!(doc["lines"][1]["severity"], "output");
        assert_eq!(doc["lines"][1]["error"], false);
        assert!(doc["lines"][1].get("stack_trace").is_none(), "{doc}");
    }

    /// `?` is a table glyph. A document says `unknown`, and keeps the raw code
    /// so "Roblox added a severity" stays distinguishable from "the line
    /// carried none".
    #[test]
    fn an_unnamed_severity_is_a_word_plus_its_raw_code() {
        let lines = log_page(r#"{"gameServerLogs":[{"severity":99},{"message":"x"}]}"#);
        let doc = parsed(&LogsDocument::new(
            &lines,
            "a".to_string(),
            "1".to_string(),
            None,
            200,
        ));

        assert_eq!(doc["lines"][0]["severity"], "unknown");
        assert_eq!(doc["lines"][0]["severity_code"], 99);
        assert_eq!(doc["lines"][1]["severity"], "unknown");
        assert!(doc["lines"][1].get("severity_code").is_none(), "{doc}");
    }

    /// The filter is reported in the spelling the document uses for a line, not
    /// in whatever case the caller typed.
    #[test]
    fn the_severity_filter_is_canonical_rather_than_echoed() {
        let doc = parsed(&LogsDocument::new(
            &[],
            "a".to_string(),
            "1".to_string(),
            crate::model::severity_from_name("WARNING"),
            200,
        ));

        assert_eq!(doc["severity_filter"], "warn");
    }

    /// A truncated slice must say so: a crash investigation that reads the last
    /// line of a page cut short by `--limit` is reading the wrong line.
    #[test]
    fn a_truncated_log_slice_reports_the_limit_it_hit() {
        let lines = log_page(r#"{"gameServerLogs":[{"message":"a"},{"message":"b"}]}"#);
        assert!(LogsDocument::new(&lines, "a".into(), "1".into(), None, 2).limit_reached);
        assert!(!LogsDocument::new(&lines, "a".into(), "1".into(), None, 3).limit_reached);
    }

    /// A server that logged nothing is a document with no lines rather than no
    /// document, and it still names what was asked for.
    #[test]
    fn no_lines_still_answers_which_server_and_which_version() {
        let doc = parsed(&LogsDocument::new(
            &[],
            "aba9aeae".to_string(),
            "3982".to_string(),
            None,
            200,
        ));

        assert_eq!(doc["lines"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["totals"]["returned"], 0);
        assert_eq!(doc["totals"]["errors"], 0);
        assert_eq!(doc["job_id"], "aba9aeae");
        assert_eq!(doc["place_version"], "3982");
    }
}
