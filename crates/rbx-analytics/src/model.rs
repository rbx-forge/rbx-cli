//! Types for `analytics-query-api/v1`.
//!
//! The API is beta and its OpenAPI entry describes `metric` as a free string
//! with no list of valid values anywhere. The names below were found by asking
//! for a nonsense metric and reading the error, then probing candidates against
//! the live game. Anything not in this list is still accepted and forwarded, so
//! a metric Roblox adds tomorrow works without a release.

use serde::{Deserialize, Serialize};

/// Metric names confirmed against a live experience on 2026-08-03.
///
/// Not exhaustive and not enforced: `--metric` takes any string. This list is
/// what `rbx-ops analytics metrics` prints, so the common case does not
/// require guessing at a name Roblox documents nowhere.
pub const KNOWN_METRICS: &[(&str, &str)] = &[
    ("Visits", "Sessions started"),
    ("DailyActiveUsers", "Distinct players per day"),
    (
        "MonthlyActiveUsers",
        "Distinct players over the trailing month",
    ),
    ("D1Retention", "Share of new players returning the next day"),
    (
        "D7Retention",
        "Share of new players returning within seven days",
    ),
    (
        "D30Retention",
        "Share of new players returning within thirty days",
    ),
    ("AverageRevenuePerPayingUser", "ARPPU, in Robux"),
];

/// Bucket size for the returned series. Values are Roblox's, spelled its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Granularity {
    OneMinute,
    HalfHour,
    OneHour,
    OneDay,
    OneWeek,
    OneMonth,
    /// One bucket for the whole range.
    None,
}

impl Granularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneMinute => "OneMinute",
            Self::HalfHour => "HalfHour",
            Self::OneHour => "OneHour",
            Self::OneDay => "OneDay",
            Self::OneWeek => "OneWeek",
            Self::OneMonth => "OneMonth",
            Self::None => "None",
        }
    }

    /// The spelling `--granularity` takes, which is not the one Roblox does.
    ///
    /// `--json` reports this rather than the wire form, so the value in a
    /// stored document goes straight back onto a command line. Kept in step
    /// with the clap value-enum names by a test below rather than derived from
    /// clap, so the document does not silently follow a rename of the flag.
    pub fn as_cli_str(self) -> &'static str {
        match self {
            Self::OneMinute => "one-minute",
            Self::HalfHour => "half-hour",
            Self::OneHour => "one-hour",
            Self::OneDay => "one-day",
            Self::OneWeek => "one-week",
            Self::OneMonth => "one-month",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricQuery {
    pub metric: String,
    pub granularity: String,
    /// Inclusive.
    pub start_time: String,
    /// Exclusive.
    pub end_time: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breakdown: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<QueryFilter>,
}

/// Narrow a query to particular values of a dimension.
///
/// Not interchangeable with `breakdown`, and the difference is not cosmetic:
/// some dimensions are filter-only. Asking Roblox to break down by `FunnelName`
/// is refused outright —
/// "Dimension FunnelName is filter-only … Please use dimension-values to obtain
/// available values" — so a single funnel can only be isolated here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryFilter {
    pub dimension: String,
    pub values: Vec<String>,
    /// `In`, `NotIn`, `GreaterThan`, `GreaterThanOrEqual`, `LessThan`,
    /// `LessThanOrEqual`, `Match`.
    pub operation: String,
}

impl QueryFilter {
    /// Parse `Dimension=v1,v2`, the only form the CLI accepts.
    ///
    /// `In` is the operation because it is the one that answers "this funnel,
    /// these platforms". The comparison operations the API also has need a
    /// different surface than a `=` split, and no caller has wanted one.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let (dimension, values) = raw
            .split_once('=')
            .ok_or_else(|| format!("expected `Dimension=value[,value]`, got `{raw}`"))?;
        let dimension = dimension.trim();
        if dimension.is_empty() {
            return Err(format!("no dimension before `=` in `{raw}`"));
        }
        let values: Vec<String> = values
            .split(',')
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        if values.is_empty() {
            return Err(format!("no values after `=` in `{raw}`"));
        }
        Ok(Self {
            dimension: dimension.to_string(),
            values,
            operation: "In".to_string(),
        })
    }
}

/// Body of a `dimension-values` query: which values a dimension actually takes.
///
/// The companion to a filter-only dimension. You cannot filter on a funnel name
/// you do not know, and Roblox names this endpoint in the error it returns when
/// you try to break down by one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionValuesQuery {
    /// The metric giving the dimensions their meaning; required even though
    /// no metric values come back.
    pub metric: String,
    pub dimensions: Vec<String>,
    pub start_time: String,
    pub end_time: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DimensionValuesResponse {
    #[serde(default)]
    pub values: Vec<DimensionValues>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DimensionValues {
    #[serde(default)]
    pub dimension: String,
    #[serde(default)]
    pub values: Vec<DimensionValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionValue {
    #[serde(default)]
    pub value: String,
    /// Set when the value is an opaque id, which funnel steps are.
    #[serde(default)]
    pub display_value: Option<String>,
}

impl DimensionValue {
    /// What to show a person: the display value when Roblox provides one,
    /// since a raw funnel-step id tells nobody anything.
    pub fn label(&self) -> &str {
        match self.display_value.as_deref() {
            Some(display) if !display.is_empty() => display,
            _ => &self.value,
        }
    }
}

/// The envelope every analytics call returns.
///
/// It is a long-running-operation shape, but for the ranges a CLI asks for it
/// comes back `done: true` with the result already inline. The polling path
/// exists for the case where it does not, and is handled rather than assumed
/// away.
#[derive(Debug, Clone, Deserialize)]
pub struct Operation {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub response: Option<MetricResponse>,
    /// Present instead of `response` when the query failed. Note that this
    /// arrives with an HTTP 400, so the envelope is what an error body looks
    /// like, not only what a success looks like.
    #[serde(default)]
    pub error: Option<OperationError>,
}

/// The same envelope, carrying dimension values instead of a series.
#[derive(Debug, Clone, Deserialize)]
pub struct DimensionValuesOperation {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub response: Option<DimensionValuesResponse>,
    #[serde(default)]
    pub error: Option<OperationError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OperationError {
    #[serde(default)]
    pub code: Option<i64>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetricResponse {
    #[serde(default)]
    pub values: Vec<MetricSeries>,
}

#[derive(Debug, Clone, Deserialize)]
// Required, not decorative: the wire field is `dataPoints`. Without this the
// series parses fine and comes back with zero points, so the command prints
// "no data" for a metric that has data. `#[serde(deny_unknown_fields)]` is
// deliberately not used instead, because Roblox adds fields to beta responses.
#[serde(rename_all = "camelCase")]
pub struct MetricSeries {
    /// Dimension values identifying this series. Empty when no `--breakdown`
    /// was asked for, which is the common case.
    #[serde(default)]
    pub breakdowns: Vec<serde_json::Value>,
    #[serde(default)]
    pub data_points: Vec<DataPoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DataPoint {
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub value: Option<f64>,
}

impl MetricSeries {
    /// A short label for this series, for a table or a CSV column.
    pub fn label(&self) -> String {
        if self.breakdowns.is_empty() {
            return "total".to_string();
        }
        self.breakdowns
            .iter()
            .map(|value| match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_completed_query_carries_its_result_inline() {
        let body = r#"{"done":true,"response":{"values":[{"breakdowns":[],
            "dataPoints":[{"time":"2026-07-28T00:00:00+00:00","value":1245}]}]}}"#;
        let operation: Operation = serde_json::from_str(body).unwrap();
        assert!(operation.done);
        let series = &operation.response.unwrap().values[0];
        assert_eq!(series.data_points[0].value, Some(1245.0));
    }

    #[test]
    fn a_failed_query_uses_the_same_envelope_with_error_instead_of_response() {
        // This body arrives with HTTP 400, so the error path must parse the
        // envelope rather than assume a failure has no body.
        let body = r#"{"done":true,"error":{"code":2001,
            "message":"Query Error: The metric with name NoSuchMetric was not found."}}"#;
        let operation: Operation = serde_json::from_str(body).unwrap();
        assert!(operation.response.is_none());
        assert_eq!(operation.error.unwrap().code, Some(2001));
    }

    #[test]
    fn a_series_with_no_breakdown_is_labelled_total() {
        let series = MetricSeries {
            breakdowns: vec![],
            data_points: vec![],
        };
        assert_eq!(series.label(), "total");
    }

    #[test]
    fn a_broken_down_series_is_labelled_by_its_dimension_values() {
        let series: MetricSeries =
            serde_json::from_str(r#"{"breakdowns":["Phone","US"],"dataPoints":[]}"#).unwrap();
        assert_eq!(series.label(), "Phone / US");
    }

    #[test]
    fn granularity_spells_itself_the_way_roblox_expects() {
        assert_eq!(Granularity::OneDay.as_str(), "OneDay");
        assert_eq!(Granularity::HalfHour.as_str(), "HalfHour");
    }

    /// The document reports the flag's spelling, so it has to be the flag's
    /// spelling: every value must round-trip back through the value parser
    /// that produced it.
    #[test]
    fn every_granularity_round_trips_through_the_flag_it_came_from() {
        use clap::ValueEnum;

        for granularity in Granularity::value_variants() {
            let cli = granularity.as_cli_str();
            assert_eq!(
                Granularity::from_str(cli, false).expect("the CLI spelling must parse"),
                *granularity,
                "`{cli}` is not what --granularity calls {granularity:?}"
            );
        }
    }

    #[test]
    fn an_empty_breakdown_is_omitted_from_the_request_body() {
        // Roblox rejects unknown or empty optional arrays on some endpoints;
        // sending nothing is safer than sending [].
        let query = MetricQuery {
            metric: "Visits".into(),
            granularity: "OneDay".into(),
            start_time: "a".into(),
            end_time: "b".into(),
            breakdown: vec![],
            filter: vec![],
        };
        let json = serde_json::to_string(&query).unwrap();
        assert!(!json.contains("breakdown"), "got: {json}");
        assert!(!json.contains("filter"), "got: {json}");
    }

    #[test]
    fn a_filter_parses_into_an_in_clause() {
        let f = QueryFilter::parse("FunnelName=Tutorial").unwrap();
        assert_eq!(f.dimension, "FunnelName");
        assert_eq!(f.values, vec!["Tutorial"]);
        assert_eq!(f.operation, "In");
    }

    #[test]
    fn several_values_of_one_dimension_split_on_commas_and_trim() {
        let f = QueryFilter::parse("Platform= Windows , Android ").unwrap();
        assert_eq!(f.values, vec!["Windows", "Android"]);
    }

    #[test]
    fn a_filter_without_an_equals_is_rejected_with_the_form_it_wanted() {
        let err = QueryFilter::parse("FunnelName").unwrap_err();
        assert!(err.contains("Dimension=value"), "got: {err}");
    }

    #[test]
    fn a_filter_with_no_values_is_rejected_rather_than_sent_empty() {
        // `FunnelName=` would otherwise serialise to an empty `In` clause,
        // which matches nothing and looks like "this funnel has no data".
        assert!(QueryFilter::parse("FunnelName=").is_err());
        assert!(QueryFilter::parse("=Tutorial").is_err());
    }

    #[test]
    fn a_dimension_value_prefers_its_display_name_but_keeps_the_raw_id() {
        // Funnel step ids are opaque; the display value is the readable half,
        // and the raw one is what --filter takes.
        let v: DimensionValue =
            serde_json::from_str(r#"{"value":"abc123","displayValue":"Step 2: Move"}"#).unwrap();
        assert_eq!(v.label(), "Step 2: Move");
        assert_eq!(v.value, "abc123");

        let bare: DimensionValue = serde_json::from_str(r#"{"value":"Tutorial"}"#).unwrap();
        assert_eq!(bare.label(), "Tutorial");
    }
}
