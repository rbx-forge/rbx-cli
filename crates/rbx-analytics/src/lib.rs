//! `rbx-ops analytics` : read an experience's own metrics.
//!
//! The Creator Dashboard already charts these. The reasons to pull them through
//! an API are the three it cannot do: keep history past Roblox's own window,
//! join the numbers with data of your own, and alert on a change without a
//! human looking. CSV output exists for exactly that, so the series can land in
//! a file and a real charting tool can read it. Do not build charts here.

pub mod json;
pub mod model;

use anyhow::{bail, Context, Result};
use chrono::{Duration, SecondsFormat, Utc};
use clap::{Args, Subcommand};
use colored::Colorize;
use reqwest::Client;

use rbx_core::api::{
    build_client, execute_with_retry, explain_missing_scope, require_api_key, ApiBase,
};
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use crate::json::{MetricsDocument, QueryDocument};
use crate::model::{
    DimensionValuesOperation, DimensionValuesQuery, Granularity, MetricQuery, MetricResponse,
    Operation, QueryFilter, KNOWN_METRICS,
};

#[derive(Args, Debug)]
pub struct AnalyticsCli {
    #[command(subcommand)]
    command: Command,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

impl AnalyticsCli {
    /// Point this invocation at a mock host. Tests only; `base_url` stays a
    /// hidden flag so nothing in production can set it by accident.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print the metric names known to work
    ///
    /// Roblox publishes no list. These were confirmed by querying a live
    /// experience. `metrics --metric` accepts any name, so a metric added
    /// later works without waiting for a release.
    Metrics {
        /// Write the list to stdout as one JSON document instead of a table.
        ///
        /// The same names and descriptions, plus `exhaustive: false`: this
        /// list is what was confirmed against a live experience, not what
        /// exists, so it is not a whitelist to validate `--metric` against.
        /// Field names are documented in docs/ops/analytics.md.
        #[arg(long)]
        json: bool,
    },

    /// Query one metric over a time range
    Query {
        /// Metric name, e.g. `DailyActiveUsers`. See `analytics metrics`.
        #[arg(long)]
        metric: String,

        /// Bucket size.
        #[arg(long, value_enum, default_value = "one-day")]
        granularity: Granularity,

        /// How many days back from now to query.
        #[arg(long, default_value_t = 30)]
        days: i64,

        /// Group the series by these dimensions, e.g. `--breakdown Platform`.
        #[arg(long)]
        breakdown: Vec<String>,

        /// Narrow to particular values: `--filter FunnelName=Tutorial`.
        ///
        /// Repeatable, and comma-separated for several values of one
        /// dimension. Not the same thing as `--breakdown`: some dimensions are
        /// filter-only, `FunnelName` among them, so isolating a single funnel
        /// is only possible here. Use `analytics dimensions` to find the values.
        #[arg(long, value_name = "DIMENSION=VALUE[,VALUE]")]
        filter: Vec<String>,

        /// Emit CSV instead of a table, for a file or a charting tool.
        #[arg(long)]
        csv: bool,

        /// Write the result to stdout as one JSON document instead of a table.
        ///
        /// Carries what CSV cannot: the query that produced the numbers, the
        /// series structure a breakdown creates, and the difference between a
        /// measured zero and a bucket with no value. stdout carries the
        /// document and nothing else; the waiting note and every warning stay
        /// on stderr. Field names are documented in docs/ops/analytics.md.
        ///
        /// Rejected together with `--csv`: the JSON document already carries
        /// everything the CSV does, so asking for both is a mistake rather
        /// than a question of which one wins.
        #[arg(long, conflicts_with = "csv")]
        json: bool,
    },

    /// List the values a dimension actually takes
    ///
    /// The companion to `--filter`: you cannot filter on a funnel name you do
    /// not know. Roblox names this endpoint itself when it refuses to break
    /// down by a filter-only dimension.
    Dimensions {
        /// Metric giving the dimensions their meaning, e.g.
        /// `FunnelUserTotalCount`. Required by Roblox even though no metric
        /// values come back.
        #[arg(long)]
        metric: String,

        /// Dimensions to enumerate, e.g. `--dimension FunnelName`.
        #[arg(long = "dimension", value_name = "DIMENSION")]
        dimensions: Vec<String>,

        /// How many days back from now to look.
        #[arg(long, default_value_t = 30)]
        days: i64,
    },
}

pub async fn run(cli: AnalyticsCli, global: &GlobalFlags) -> Result<()> {
    match cli.command {
        Command::Metrics { json } => {
            if OutputFormat::from_json_flag(json).is_json() {
                return output::emit(&MetricsDocument::new());
            }
            println!("{}", "metrics confirmed against a live experience".bold());
            for (name, description) in KNOWN_METRICS {
                println!("  {name:<28} {}", description.dimmed());
            }
            println!();
            println!(
                "{}",
                "Any other name is forwarded as-is; Roblox answers with an error naming it."
                    .dimmed()
            );
            Ok(())
        }

        Command::Query {
            metric,
            granularity,
            days,
            breakdown,
            filter,
            csv,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            if global.env.as_deref() == Some("all") {
                bail!("`--env all` is not supported here: name a single env.");
            }
            if days <= 0 {
                bail!("--days must be positive, got {days}");
            }

            let filter = filter
                .iter()
                .map(|raw| QueryFilter::parse(raw).map_err(|e| anyhow::anyhow!("--filter: {e}")))
                .collect::<Result<Vec<_>>>()?;

            let universe_id = global.single_universe()?;

            let end = Utc::now();
            let start = end - Duration::days(days);
            let query = MetricQuery {
                metric: metric.clone(),
                granularity: granularity.as_str().to_string(),
                start_time: start.to_rfc3339_opts(SecondsFormat::Secs, true),
                end_time: end.to_rfc3339_opts(SecondsFormat::Secs, true),
                breakdown,
                filter,
            };

            let api_key = require_api_key(global.api_key.as_deref())?;
            let base = match &cli.base_url {
                Some(url) => ApiBase::new(url.clone()),
                None => ApiBase::default(),
            };
            let client = build_client();

            let mut operation = query_metric(&client, &base, api_key, universe_id, &query)
                .await
                .with_context(|| format!("querying metric `{metric}`"))?;

            // Wide ranges come back unfinished, and funnel metrics only accept
            // `granularity: None`, so this is their ordinary path rather than
            // an edge case.
            let queued = !operation.done;
            if queued {
                let Some(path) = operation.path.clone() else {
                    bail!("Roblox returned an unfinished query with no path to poll");
                };
                // A note, not the result. Under `--json` it goes to stderr,
                // where it cannot corrupt the document; the same fact is in
                // the document as `queued`.
                format.note("Query queued by Roblox; waiting for it…".dimmed());
                operation = await_operation(&client, &base, api_key, &path)
                    .await
                    .with_context(|| format!("polling metric `{metric}`"))?;
            }

            let response = finished(&operation)?;
            if format.is_json() {
                if response.values.iter().all(|s| s.data_points.is_empty()) {
                    format.note(EMPTY_RANGE.dimmed());
                }
                // An empty range is an empty document rather than no document,
                // so a scheduled job reads `.totals.points` instead of having
                // to tell "no data" from "the command printed nothing".
                return output::emit(&QueryDocument::new(
                    &query,
                    granularity,
                    days,
                    queued,
                    response,
                ));
            }

            render(response, &metric, csv)
        }

        Command::Dimensions {
            metric,
            dimensions,
            days,
        } => {
            if global.env.as_deref() == Some("all") {
                bail!("`--env all` is not supported here: name a single env.");
            }
            if days <= 0 {
                bail!("--days must be positive, got {days}");
            }
            if dimensions.is_empty() {
                bail!("name at least one dimension, e.g. `--dimension FunnelName`");
            }

            let universe_id = global.single_universe()?;
            let end = Utc::now();
            let start = end - Duration::days(days);
            let query = DimensionValuesQuery {
                metric: metric.clone(),
                dimensions,
                start_time: start.to_rfc3339_opts(SecondsFormat::Secs, true),
                end_time: end.to_rfc3339_opts(SecondsFormat::Secs, true),
            };

            let api_key = require_api_key(global.api_key.as_deref())?;
            let base = match &cli.base_url {
                Some(url) => ApiBase::new(url.clone()),
                None => ApiBase::default(),
            };
            let client = build_client();

            let mut operation =
                query_dimension_values(&client, &base, api_key, universe_id, &query)
                    .await
                    .with_context(|| format!("listing dimension values for `{metric}`"))?;

            if !operation.done {
                let Some(path) = operation.path.clone() else {
                    bail!("Roblox returned an unfinished query with no path to poll");
                };
                println!("{}", "Query queued by Roblox; waiting for it…".dimmed());
                operation = await_operation(&client, &base, api_key, &path).await?;
            }

            if let Some(failure) = &operation.error {
                bail!(
                    "{}",
                    failure
                        .message
                        .clone()
                        .unwrap_or_else(|| "dimension-values query rejected".into())
                );
            }

            let Some(response) = &operation.response else {
                bail!("the query completed with neither a result nor an error");
            };
            if response.values.iter().all(|d| d.values.is_empty()) {
                println!(
                    "{}",
                    "No values in that range. The experience may not log this dimension.".dimmed()
                );
                return Ok(());
            }

            for dimension in &response.values {
                println!("{}", dimension.dimension.bold());
                for value in &dimension.values {
                    // The raw value is what `--filter` takes; the label is what
                    // a person recognises. Print both when they differ.
                    if value.label() == value.value {
                        println!("  {}", value.value);
                    } else {
                        println!("  {:<40} {}", value.label(), value.value.dimmed());
                    }
                }
                println!();
            }
            Ok(())
        }
    }
}

async fn query_metric(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    universe_id: u64,
    query: &MetricQuery,
) -> Result<Operation> {
    let url = base.join(&format!(
        "/analytics-query-api/v1/universes/{universe_id}/metrics"
    ));

    let response = execute_with_retry(|| {
        let request = client.post(&url).header("x-api-key", api_key).json(query);
        async move { request.send().await.map_err(Into::into) }
    })
    .await;

    // A rejected query answers 400 with the operation envelope in the body,
    // carrying a message that names what was wrong. Surfacing "API error 400"
    // and dropping that message would throw away the only useful part.
    let body = match response {
        Ok(response) => response.text().await?,
        Err(error) => {
            let text = error.to_string();
            if let Some(start) = text.find('{') {
                if let Ok(operation) = serde_json::from_str::<Operation>(&text[start..]) {
                    if let Some(failure) = operation.error {
                        bail!(
                            "{}",
                            failure
                                .message
                                .unwrap_or_else(|| "analytics query rejected".into())
                        );
                    }
                }
            }
            return Err(explain_missing_scope(error));
        }
    };

    serde_json::from_str(&body).with_context(|| format!("parsing analytics response: {body}"))
}

/// How long to keep polling an unfinished query before giving up.
///
/// The comment this replaces said an unfinished operation was "never seen for
/// CLI-sized ranges". That was wrong: a 365-day funnel query returns one
/// immediately, and funnel metrics only accept `granularity: None`, so wide
/// ranges are their normal case rather than an exotic one.
const POLL_ATTEMPTS: usize = 30;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Follow `operations/metrics/{id}` until the query finishes.
///
/// `path` comes back relative (`v1/universes/…`), so it is joined onto the
/// service prefix rather than the host.
async fn await_operation<T: serde::de::DeserializeOwned>(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    path: &str,
) -> Result<T> {
    let url = base.join(&format!(
        "/analytics-query-api/{}",
        path.trim_start_matches('/')
    ));

    for attempt in 0..POLL_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        let response = execute_with_retry(|| {
            let request = client.get(&url).header("x-api-key", api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;

        let body = response.text().await?;
        let value: serde_json::Value = serde_json::from_str(&body)
            .with_context(|| format!("parsing polled analytics response: {body}"))?;
        if value.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
            return serde_json::from_value(value)
                .with_context(|| format!("parsing polled analytics response: {body}"));
        }
    }

    bail!(
        "Roblox did not finish this query within {}s. It is still running at {path}; \
         narrow the range with --days and try again.",
        POLL_ATTEMPTS as u64 * POLL_INTERVAL.as_secs()
    )
}

async fn query_dimension_values(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    universe_id: u64,
    query: &DimensionValuesQuery,
) -> Result<DimensionValuesOperation> {
    let url = base.join(&format!(
        "/analytics-query-api/v1/universes/{universe_id}/dimension-values"
    ));

    let response = execute_with_retry(|| {
        let request = client.post(&url).header("x-api-key", api_key).json(query);
        async move { request.send().await.map_err(Into::into) }
    })
    .await;

    let body = match response {
        Ok(response) => response.text().await?,
        Err(error) => {
            let text = error.to_string();
            if let Some(start) = text.find('{') {
                if let Ok(op) = serde_json::from_str::<DimensionValuesOperation>(&text[start..]) {
                    if let Some(failure) = op.error {
                        bail!(
                            "{}",
                            failure
                                .message
                                .unwrap_or_else(|| "dimension-values query rejected".into())
                        );
                    }
                }
            }
            return Err(explain_missing_scope(error));
        }
    };

    serde_json::from_str(&body)
        .with_context(|| format!("parsing dimension-values response: {body}"))
}

/// What a query with nothing in its range says, in every format. Under
/// `--json` it is a note on stderr beside an empty document; otherwise it is
/// the whole output.
const EMPTY_RANGE: &str = "No data points in that range. A quiet experience or too short a window.";

/// The result of a finished operation, or the error Roblox put in the envelope
/// instead of one.
///
/// Split out of the renderer because both output formats need the same two
/// failure cases handled the same way, and only one of them prints.
fn finished(operation: &Operation) -> Result<&MetricResponse> {
    if let Some(failure) = &operation.error {
        bail!(
            "{}",
            failure
                .message
                .clone()
                .unwrap_or_else(|| "analytics query rejected".into())
        );
    }

    operation
        .response
        .as_ref()
        .context("the query completed with neither a result nor an error")
}

fn render(response: &MetricResponse, metric: &str, csv: bool) -> Result<()> {
    if response.values.iter().all(|s| s.data_points.is_empty()) {
        println!("{}", EMPTY_RANGE.dimmed());
        return Ok(());
    }

    if csv {
        println!("time,series,metric,value");
        for series in &response.values {
            for point in &series.data_points {
                println!(
                    "{},{},{},{}",
                    point.time.as_deref().unwrap_or(""),
                    series.label(),
                    metric,
                    point.value.map(|v| v.to_string()).unwrap_or_default()
                );
            }
        }
        return Ok(());
    }

    for series in &response.values {
        println!(
            "{} {}",
            metric.bold(),
            format!("({})", series.label()).dimmed()
        );
        for point in &series.data_points {
            let time = point.time.as_deref().unwrap_or("-");
            // Keep the date, drop the time-of-day: every granularity a person
            // reads at the command line is daily or coarser.
            let day = time.split('T').next().unwrap_or(time);
            match point.value {
                Some(value) => println!("  {day}  {value:>12.2}"),
                None => println!("  {day}  {:>12}", "-"),
            }
        }
        println!();
    }

    Ok(())
}
