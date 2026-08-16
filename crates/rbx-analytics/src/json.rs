//! What `rbx analytics query --json` and `rbx analytics metrics --json` write
//! to stdout.
//!
//! Separate from `model` on purpose. `model` describes the long-running
//! operation envelope Roblox sends, down to the camelCase `dataPoints` and the
//! untyped `breakdowns` array; this describes what we promise. A field renamed
//! upstream is a parsing change there, not a break in somebody's `jq` filter.
//!
//! The envelope follows `rbx check --json`: a `schema_version` first, then
//! named objects all the way down. Field names are documented in
//! `docs/ops/analytics.md` and are the compatibility surface.
//!
//! ## Holes in a series
//!
//! A time series from Roblox is not dense, and two different things look alike
//! if the document is careless about them. A bucket can come back with no
//! value, and a bucket can fail to come back at all. Conflating either with a
//! measured zero turns "the pipeline stopped reporting" into "nobody played",
//! which is the one mistake an alerting job must not make. So:
//!
//! - `"value": 0` means measured zero.
//! - a point with no `value` key means Roblox returned the bucket and put no
//!   number in it. `has("value")` is the test, in line with the general rule
//!   that an optional field is omitted rather than emitted as `null`.
//! - no point at all for a timestamp means Roblox returned nothing for it.
//!
//! What this module never does is invent the third case into the second. The
//! CLI does not know Roblox's bucket calendar for every granularity — funnel
//! metrics only accept `granularity: None`, a breakdown can be ragged across
//! series — so a synthesised bucket would be a guess presented as data. The
//! points are the ones Roblox sent, in the order it sent them, and a consumer
//! that wants a dense series reindexes against `start_time` and `end_time`,
//! which the document carries for exactly that reason.

use std::collections::BTreeMap;

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::model::{
    Granularity, MetricQuery, MetricResponse, MetricSeries, QueryFilter, KNOWN_METRICS,
};

/// One `analytics metrics` invocation.
#[derive(Debug, Serialize)]
pub struct MetricsDocument {
    pub schema_version: u32,
    /// Always `false`, and present so the caveat the human form prints as a
    /// footer survives into the document: Roblox publishes no list, these names
    /// were found by probing a live experience, and `--metric` forwards any
    /// string. A consumer must not validate a metric name against this list.
    pub exhaustive: bool,
    /// One object per known metric, in the order the human form prints them.
    pub metrics: Vec<Metric>,
}

#[derive(Debug, Serialize)]
pub struct Metric {
    /// What `--metric` takes, spelled the way Roblox expects it.
    pub name: String,
    /// One line, the same text the human form prints beside the name.
    pub description: String,
}

impl MetricsDocument {
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            exhaustive: false,
            metrics: KNOWN_METRICS
                .iter()
                .map(|(name, description)| Metric {
                    name: (*name).to_string(),
                    description: (*description).to_string(),
                })
                .collect(),
        }
    }
}

impl Default for MetricsDocument {
    fn default() -> Self {
        Self::new()
    }
}

/// One `analytics query` invocation.
///
/// The query itself is echoed back, not just its answer. `--days` is relative
/// to the moment the command ran, so a stored document that did not carry
/// `start_time` and `end_time` could never be lined up with another one; and a
/// consumer that reads `.metric` off the document does not have to keep the
/// invocation that produced it.
#[derive(Debug, Serialize)]
pub struct QueryDocument {
    pub schema_version: u32,
    /// The metric asked for, exactly as `--metric` spelled it.
    pub metric: String,
    /// The bucket size, in the spelling `--granularity` takes rather than the
    /// `OneDay` form sent on the wire, so the value round-trips back into a
    /// command line.
    pub granularity: String,
    /// The `--days` in force.
    pub days: i64,
    /// Start of the range, inclusive. RFC 3339, UTC.
    pub start_time: String,
    /// End of the range, exclusive. RFC 3339, UTC. Together with `start_time`
    /// this is what a consumer reindexes a sparse series against.
    pub end_time: String,
    /// The dimensions `--breakdown` asked for, in the order given. Empty when
    /// none were, which is the common case.
    pub breakdown: Vec<String>,
    /// The `--filter` clauses, parsed. Empty when none were given.
    pub filters: Vec<Filter>,
    /// True when Roblox did not answer inline and handed back an operation to
    /// poll. The same fact the human form prints as a waiting note, kept in the
    /// document because under `--json` that note is on stderr.
    pub queued: bool,
    pub totals: Totals,
    /// One object per series, in the order Roblox returned them. A single
    /// unnamed series when no `--breakdown` was asked for.
    pub series: Vec<Series>,
}

#[derive(Debug, Serialize)]
pub struct Filter {
    pub dimension: String,
    /// `In` for everything the CLI can express today.
    pub operation: String,
    pub values: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Totals {
    /// Entries in `series`.
    pub series: usize,
    /// Points across every series: what Roblox returned, not what a dense
    /// range would hold.
    pub points: usize,
    /// How many of `points` came back without a value. Non-zero means the
    /// series has holes in it that are not zeros, and any average computed by
    /// treating a hole as zero is wrong.
    pub missing: usize,
}

/// One series: the whole answer when nothing was broken down, one dimension
/// combination when something was.
#[derive(Debug, Serialize)]
pub struct Series {
    /// The same short label the table and the CSV print: `total` for an
    /// unbroken series, otherwise the dimension values joined with ` / `.
    /// Kept so a document lines up with the other two output forms, and so
    /// nothing is lost when Roblox returns more breakdown values than
    /// dimensions were asked for.
    pub label: String,
    /// The dimension values that identify this series, keyed by dimension
    /// name rather than positional, so `.dimensions.Platform` reads it. Empty
    /// when no `--breakdown` was asked for.
    pub dimensions: BTreeMap<String, String>,
    /// The points Roblox returned for this series, in its order. Empty is a
    /// real answer: the series exists and had nothing in the range.
    pub points: Vec<Point>,
}

/// One bucket.
///
/// Both fields are optional and both are omitted rather than emitted as
/// `null`. See the module docs: an omitted `value` is a reported bucket with no
/// number in it, which is not a zero and not the same as a bucket that never
/// came back.
#[derive(Debug, Serialize)]
pub struct Point {
    /// Start of the bucket, as Roblox timestamped it. **Absent** in the one
    /// case Roblox sends a point with no time, which the table prints as `-`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// The measurement. **Absent** when the bucket carries none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

/// Render one untyped breakdown value the way the label does: a JSON string
/// without its quotes, anything else as its JSON text.
fn scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

impl Series {
    fn new(series: &MetricSeries, breakdown: &[String]) -> Self {
        Self {
            label: series.label(),
            // Zipping stops at the shorter of the two. Roblox has only ever
            // returned one value per requested dimension; if it returned more,
            // the surplus is still in `label` rather than lost.
            dimensions: breakdown
                .iter()
                .cloned()
                .zip(series.breakdowns.iter().map(scalar))
                .collect(),
            points: series
                .data_points
                .iter()
                .map(|point| Point {
                    time: point.time.clone(),
                    value: point.value,
                })
                .collect(),
        }
    }
}

impl QueryDocument {
    /// Build the document from the query that was sent and the response that
    /// came back.
    ///
    /// Pure, and deliberately so: the renderer prints, this decides what the
    /// document says, and a test can therefore assert the shape without a
    /// process to capture.
    pub fn new(
        query: &MetricQuery,
        granularity: Granularity,
        days: i64,
        queued: bool,
        response: &MetricResponse,
    ) -> Self {
        let series: Vec<Series> = response
            .values
            .iter()
            .map(|series| Series::new(series, &query.breakdown))
            .collect();
        let points = series.iter().map(|s| s.points.len()).sum();
        let missing = series
            .iter()
            .flat_map(|s| s.points.iter())
            .filter(|point| point.value.is_none())
            .count();

        Self {
            schema_version: SCHEMA_VERSION,
            metric: query.metric.clone(),
            granularity: granularity.as_cli_str().to_string(),
            days,
            start_time: query.start_time.clone(),
            end_time: query.end_time.clone(),
            breakdown: query.breakdown.clone(),
            filters: query.filter.iter().map(Filter::from).collect(),
            queued,
            totals: Totals {
                series: series.len(),
                points,
                missing,
            },
            series,
        }
    }
}

impl From<&QueryFilter> for Filter {
    fn from(filter: &QueryFilter) -> Self {
        Self {
            dimension: filter.dimension.clone(),
            operation: filter.operation.clone(),
            values: filter.values.clone(),
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

    fn query(breakdown: &[&str], filter: Vec<QueryFilter>) -> MetricQuery {
        MetricQuery {
            metric: "DailyActiveUsers".into(),
            granularity: "OneDay".into(),
            start_time: "2026-07-27T00:00:00Z".into(),
            end_time: "2026-08-03T00:00:00Z".into(),
            breakdown: breakdown.iter().map(|d| (*d).to_string()).collect(),
            filter,
        }
    }

    fn response(json: &str) -> MetricResponse {
        serde_json::from_str(json).expect("fixture")
    }

    #[test]
    fn the_query_envelope_carries_the_documented_fields() {
        let response = response(
            r#"{"values":[{"breakdowns":[],"dataPoints":[
                {"time":"2026-07-28T00:00:00+00:00","value":1245},
                {"time":"2026-07-29T00:00:00+00:00","value":1310}
            ]}]}"#,
        );
        let doc = parsed(&QueryDocument::new(
            &query(&[], vec![]),
            Granularity::OneDay,
            7,
            false,
            &response,
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["metric"], "DailyActiveUsers");
        assert_eq!(doc["days"], 7);
        assert_eq!(doc["start_time"], "2026-07-27T00:00:00Z");
        assert_eq!(doc["end_time"], "2026-08-03T00:00:00Z");
        assert_eq!(doc["queued"], false);
        assert_eq!(doc["totals"]["series"], 1);
        assert_eq!(doc["totals"]["points"], 2);
        assert_eq!(doc["totals"]["missing"], 0);
        assert_eq!(doc["series"][0]["label"], "total");
        assert_eq!(
            doc["series"][0]["points"][0]["time"],
            "2026-07-28T00:00:00+00:00"
        );
        assert_eq!(doc["series"][0]["points"][1]["value"], 1310.0);
    }

    /// The granularity round-trips into a command line rather than out of one:
    /// the document says what `--granularity` would take, not the `OneDay`
    /// form that went over the wire.
    #[test]
    fn the_granularity_is_reported_in_the_spelling_the_flag_takes() {
        let doc = parsed(&QueryDocument::new(
            &query(&[], vec![]),
            Granularity::HalfHour,
            1,
            false,
            &MetricResponse::default(),
        ));

        assert_eq!(doc["granularity"], "half-hour");
    }

    /// The distinction the whole module exists for. A bucket reported with no
    /// number is not a zero, and a bucket that never came back is neither.
    #[test]
    fn a_missing_value_is_omitted_and_a_zero_one_is_emitted() {
        let response = response(
            r#"{"values":[{"breakdowns":[],"dataPoints":[
                {"time":"2026-07-27T00:00:00+00:00","value":0},
                {"time":"2026-07-28T00:00:00+00:00","value":null},
                {"time":"2026-07-30T00:00:00+00:00","value":1288}
            ]}]}"#,
        );
        let doc = parsed(&QueryDocument::new(
            &query(&[], vec![]),
            Granularity::OneDay,
            7,
            false,
            &response,
        ));

        let points = &doc["series"][0]["points"];
        // Measured zero: present, and zero.
        assert_eq!(points[0]["value"], 0.0);
        assert!(points[0]["value"].is_number());
        // Reported bucket, no number in it: the key is gone, not null.
        assert!(points[1].get("value").is_none(), "{points}");
        assert_eq!(points[1]["time"], "2026-07-28T00:00:00+00:00");
        // The 29th is simply not there: a gap is an absent point, never a
        // synthesised one.
        assert_eq!(points.as_array().map(Vec::len), Some(3));
        assert_eq!(points[2]["time"], "2026-07-30T00:00:00+00:00");
        // And the hole is counted, so a consumer notices without walking the
        // series itself.
        assert_eq!(doc["totals"]["points"], 3);
        assert_eq!(doc["totals"]["missing"], 1);
    }

    /// A point with no time at all — the case the table prints as `-` — omits
    /// the key rather than inventing a timestamp.
    #[test]
    fn a_point_with_no_time_omits_the_key() {
        let response = response(r#"{"values":[{"breakdowns":[],"dataPoints":[{"value":3}]}]}"#);
        let doc = parsed(&QueryDocument::new(
            &query(&[], vec![]),
            Granularity::None,
            30,
            false,
            &response,
        ));

        assert!(doc["series"][0]["points"][0].get("time").is_none(), "{doc}");
        assert_eq!(doc["series"][0]["points"][0]["value"], 3.0);
    }

    /// Breakdown values are keyed by the dimension that produced them. A
    /// positional array would put the lookup key in a column, and `Phone` on
    /// its own does not say which dimension it came from.
    #[test]
    fn a_broken_down_series_keys_its_values_by_dimension_name() {
        let response = response(
            r#"{"values":[
                {"breakdowns":["Phone","US"],"dataPoints":[{"time":"t","value":1}]},
                {"breakdowns":["Console","FR"],"dataPoints":[]}
            ]}"#,
        );
        let doc = parsed(&QueryDocument::new(
            &query(&["Platform", "Country"], vec![]),
            Granularity::OneDay,
            7,
            false,
            &response,
        ));

        assert_eq!(doc["breakdown"][0], "Platform");
        assert_eq!(doc["series"][0]["dimensions"]["Platform"], "Phone");
        assert_eq!(doc["series"][0]["dimensions"]["Country"], "US");
        assert_eq!(doc["series"][0]["label"], "Phone / US");
        assert_eq!(doc["series"][1]["dimensions"]["Platform"], "Console");
        // A series with nothing in it is an empty list, not an absent one.
        assert_eq!(doc["series"][1]["points"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["totals"]["series"], 2);
    }

    /// Nothing was broken down, so there is nothing to key by — an empty
    /// object, and the label still says which series this is.
    #[test]
    fn an_unbroken_series_has_no_dimensions_and_is_labelled_total() {
        let response = response(r#"{"values":[{"breakdowns":[],"dataPoints":[]}]}"#);
        let doc = parsed(&QueryDocument::new(
            &query(&[], vec![]),
            Granularity::OneDay,
            7,
            false,
            &response,
        ));

        assert_eq!(doc["series"][0]["label"], "total");
        assert_eq!(
            doc["series"][0]["dimensions"].as_object().map(|m| m.len()),
            Some(0)
        );
    }

    /// The query is echoed back because `--days` is relative to the moment the
    /// command ran: two stored documents cannot be lined up without it.
    #[test]
    fn the_filters_that_narrowed_the_query_are_part_of_the_document() {
        let doc = parsed(&QueryDocument::new(
            &query(
                &[],
                vec![QueryFilter::parse("Platform=Console,Phone").expect("parses")],
            ),
            Granularity::OneDay,
            5,
            false,
            &MetricResponse::default(),
        ));

        assert_eq!(doc["filters"][0]["dimension"], "Platform");
        assert_eq!(doc["filters"][0]["operation"], "In");
        assert_eq!(doc["filters"][0]["values"][1], "Phone");
    }

    /// An empty range is a document with no series rather than no document, so
    /// a scheduled job reads `.totals.points` instead of having to tell "no
    /// data" from "the command printed nothing".
    #[test]
    fn an_empty_answer_is_still_a_document() {
        let doc = parsed(&QueryDocument::new(
            &query(&[], vec![]),
            Granularity::OneDay,
            30,
            false,
            &MetricResponse::default(),
        ));

        assert_eq!(doc["series"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["totals"]["series"], 0);
        assert_eq!(doc["totals"]["points"], 0);
        assert_eq!(doc["totals"]["missing"], 0);
    }

    /// The waiting note goes to stderr under `--json`, so the fact that Roblox
    /// queued the query has to live in the document too.
    #[test]
    fn a_queued_query_says_so_in_the_document() {
        let doc = parsed(&QueryDocument::new(
            &query(&[], vec![]),
            Granularity::None,
            365,
            true,
            &MetricResponse::default(),
        ));

        assert_eq!(doc["queued"], true);
    }

    /// `--json` owns stdout, so nothing on either path may stop and ask a
    /// question: a prompt on stdout corrupts the document and a prompt on
    /// stderr hangs a pipeline.
    ///
    /// Analytics has nothing to ask today — no picker, no confirmation, and no
    /// `dialoguer` in its manifest — which is why the command branches on
    /// `is_json` and never on `may_prompt`. This is the test that says which
    /// way a question added later has to go.
    #[test]
    fn the_json_format_refuses_to_prompt() {
        assert!(!rbx_core::output::OutputFormat::Json.may_prompt());
    }

    #[test]
    fn the_metrics_document_lists_named_objects_and_admits_it_is_partial() {
        let doc = parsed(&MetricsDocument::new());

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        // `--metric` forwards any string, so a consumer must not treat this as
        // a whitelist. The document says so rather than leaving it to the man
        // page.
        assert_eq!(doc["exhaustive"], false);
        assert_eq!(doc["metrics"][0]["name"], "Visits");
        assert_eq!(doc["metrics"][0]["description"], "Sessions started");
        assert_eq!(
            doc["metrics"].as_array().map(Vec::len),
            Some(KNOWN_METRICS.len())
        );
    }
}
