//! `rbx-ops servers` : what the live servers of an experience are doing, and
//! how the ones that stopped ended.
//!
//! Roblox keeps a rolling 30-day window of terminated servers. Anything older
//! is gone, which is the whole argument for pulling this on a schedule rather
//! than looking at it when something already went wrong.

pub mod api;
pub mod json;
pub mod model;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;

use rbx_core::api::{require_api_key, ApiBase};
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use crate::api::{ServersApi, MAX_PAGE_SIZE};
use crate::json::{LogsDocument, ServersDocument, VersionsDocument};
use crate::model::{format_duration, severity_from_name, GameServer, ServerStatus};

/// Rows fetched when `--limit` is not given.
///
/// A live experience can have tens of thousands of rows for a single place
/// version, which at the maximum page size is hundreds of requests at
/// the maximum page size. A command that quietly does that is a command nobody
/// can safely run, so the default is small and going further is explicit.
const DEFAULT_LIMIT: u32 = 50;

#[derive(Args, Debug)]
pub struct ServersCli {
    #[command(subcommand)]
    command: Command,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List the place versions that have servers
    ///
    /// Run this first: `list` needs a version, and there is no way to ask for
    /// every version at once.
    Versions {
        /// Write the result to stdout as one JSON document instead of a list.
        ///
        /// Carries the versions and names the default one, so a script can
        /// feed `servers list` without knowing that the order is newest
        /// first. Field names are documented in docs/ops/servers.md.
        #[arg(long)]
        json: bool,
    },

    /// List servers for a place version
    List {
        /// Place version. Defaults to the newest one that has servers.
        #[arg(long)]
        version: Option<String>,

        /// Only show servers with this status, e.g. `crashed`, `out_of_memory`.
        #[arg(long)]
        status: Option<String>,

        /// Maximum rows to fetch.
        #[arg(long, default_value_t = DEFAULT_LIMIT)]
        limit: u32,

        /// Show whole job ids rather than the first eight characters.
        ///
        /// `servers logs` needs the whole one, and the truncated form is only
        /// there because a full uuid per row makes the table unreadable.
        #[arg(long)]
        full: bool,

        /// Emit CSV instead of a table.
        ///
        /// Every field Roblox returns, not just the six the table shows, so the
        /// output is worth keeping: Roblox discards a terminated server after
        /// thirty days and nothing brings it back.
        #[arg(long, conflicts_with = "full")]
        csv: bool,

        /// Write the result to stdout as one JSON document instead of a table.
        ///
        /// The same fields CSV carries plus the player ids, and the page-level
        /// facts a row cannot hold: whether the page was partial, and whether
        /// --limit was reached. stdout carries the document and nothing else;
        /// diagnostics stay on stderr. Field names are documented in
        /// docs/ops/servers.md.
        #[arg(long, conflicts_with_all = ["csv", "full"])]
        json: bool,
    },

    /// Show what one server logged before it stopped
    ///
    /// This is the other half of a crash investigation: `list --status crashed`
    /// finds the server, this says what it was doing. Roblox keeps logs for the
    /// same 30 days as the server row.
    Logs {
        /// Job id, in full. Get it from `servers list --full`.
        job_id: String,

        /// Place version the server ran. Defaults to the newest with servers.
        #[arg(long)]
        version: Option<String>,

        /// Only this severity: `output`, `info`, `warn`, `error`.
        #[arg(long)]
        severity: Option<String>,

        /// Maximum lines to fetch.
        #[arg(long, default_value_t = 200)]
        limit: u32,

        /// Emit CSV instead of formatted lines.
        #[arg(long)]
        csv: bool,

        /// Write the result to stdout as one JSON document instead of lines.
        ///
        /// One document, not one object per line: this reads a bounded slice
        /// of a finished log rather than following a live one, so a line
        /// stream would buy no earlier output and would lose the facts about
        /// the run. `jq -c '.lines[]'` produces JSON Lines from it. Field
        /// names are documented in docs/ops/servers.md.
        #[arg(long, conflicts_with = "csv")]
        json: bool,
    },
}

/// What one paginated walk of the server listing collected.
#[derive(Debug, Default)]
pub struct Walk {
    pub rows: Vec<GameServer>,
    /// Roblox reported a page as incomplete.
    pub partial: bool,
    /// The total the first page claimed, if it claimed one.
    pub total: Option<i64>,
}

/// Follow the listing pages until `limit` rows are collected.
///
/// A function rather than a loop inside `run`, so the three properties that
/// make it correct can be asserted — none of them were, and one was lost in a
/// port before this existed.
///
/// **The page size is fixed for the whole walk.** The endpoint states the rule
/// itself: "When paginating, all other parameters provided to the subsequent
/// call must match the call that provided the page token." A second page asking
/// for a different size than the call that issued its token is a request Roblox
/// may reject, and only on listings long enough to page — the ones nobody tries
/// by hand.
///
/// **So the result is truncated.** A fixed page size overshoots on the last page
/// by construction. The two belong together and neither is right alone: carrying
/// the rule to another crate without the truncate turned `--limit 150` into 200
/// rows.
///
/// **An empty page carrying a token ends the walk.** Otherwise `rows` never
/// grows, the loop condition never fails, and the command hangs issuing
/// requests. Emptiness is measured on the page rather than on what survived the
/// status filter: a page whose rows were all discarded is still progress, and
/// stopping there would cut the search short without saying so.
pub async fn walk_servers(
    api: &ServersApi,
    universe_id: u64,
    place_id: u64,
    version: &str,
    limit: u32,
    wanted: Option<&str>,
) -> Result<Walk> {
    let mut walk = Walk::default();
    let mut token: Option<String> = None;
    let page_size = 1u32.max(limit.min(MAX_PAGE_SIZE));

    while (walk.rows.len() as u32) < limit {
        let page = api
            .list_page(universe_id, place_id, version, page_size, token.as_deref())
            .await?;

        walk.partial |= page.is_partial();
        walk.total = walk.total.or(page.total_count);

        for server in &page.game_servers {
            if wanted.is_none_or(|want| server.status().as_str() == want) {
                walk.rows.push(server.clone());
            }
        }

        let page_was_empty = page.game_servers.is_empty();
        match page.next_token() {
            Some(next) if !page_was_empty => token = Some(next.to_string()),
            _ => break,
        }
    }

    walk.rows.truncate(limit as usize);
    Ok(walk)
}

pub async fn run(cli: ServersCli, global: &GlobalFlags) -> Result<()> {
    let Some(env) = global.env.as_deref() else {
        bail!("`rbx-ops servers` needs an env. Pass --env <name>.");
    };
    if env == "all" {
        bail!("`--env all` is not supported here: each env is a different experience with its own place versions. Name one env.");
    }

    let (universe_id, place_id) = global
        .resolve_place(env)
        .with_context(|| format!("resolving env `{env}`"))?;
    let api_key = require_api_key(global.api_key.as_deref())?;
    let base = match &cli.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let api = ServersApi::new(api_key, base);

    match cli.command {
        Command::Versions { json } => {
            let format = OutputFormat::from_json_flag(json);
            let options = api.filter_options(universe_id, place_id).await?;
            let versions = options.place_versions();

            if versions.is_empty() {
                // Not an error, and under `--json` not silence either: the
                // same empty document `list` emits for the same non-event, so
                // a consumer reads `.place_versions | length` instead of
                // having to tell "none" from "the command printed nothing".
                format.note(
                    "No place version has servers. Nothing has run in the last 30 days.".dimmed(),
                );
                if format.is_json() {
                    output::emit(&VersionsDocument::new(versions))?;
                }
                return Ok(());
            }

            if format.is_json() {
                output::emit(&VersionsDocument::new(versions))?;
                return Ok(());
            }

            println!("{}", "place versions with servers (newest first)".bold());
            for (index, version) in versions.iter().enumerate() {
                let marker = if index == 0 { "*" } else { " " };
                println!("  {marker} {version}");
            }
        }

        Command::List {
            version,
            status,
            limit,
            full,
            csv,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);

            let version = match version {
                Some(version) => version,
                None => {
                    let options = api.filter_options(universe_id, place_id).await?;
                    match options.place_versions().into_iter().next() {
                        Some(newest) => newest,
                        // Not an error: an experience that has run nothing in
                        // thirty days has no servers to list, and a scheduled
                        // job asking for them should not be paged about it.
                        // `servers versions` treats the same case the same way.
                        //
                        // Under `--json` the same non-event is an empty
                        // document rather than no document, so a consumer
                        // reads `.servers | length` instead of having to tell
                        // "no servers" from "the command printed nothing".
                        None => {
                            format.note(
                                "No place version has servers. Nothing has run in the last 30 days."
                                    .dimmed(),
                            );
                            if format.is_json() {
                                output::emit(&ServersDocument::new(&[], None, None, limit, false))?;
                            }
                            return Ok(());
                        }
                    }
                }
            };

            let wanted = status.as_deref().map(str::to_ascii_lowercase);
            let Walk {
                rows,
                partial,
                total,
            } = walk_servers(
                &api,
                universe_id,
                place_id,
                &version,
                limit,
                wanted.as_deref(),
            )
            .await?;

            if format.is_json() {
                // The warning is emitted here rather than inside `render`,
                // which the JSON path never reaches. It goes to stderr in both
                // formats, so stdout stays parsable either way.
                if partial {
                    warn_partial();
                }
                output::emit(&ServersDocument::new(
                    &rows,
                    Some(version),
                    total,
                    limit,
                    partial,
                ))?;
            } else if csv {
                render_csv(&rows);
            } else {
                render(&rows, &version, total, limit, partial, full);
            }
        }

        Command::Logs {
            job_id,
            version,
            severity,
            limit,
            csv,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let wanted = match severity.as_deref() {
                Some(name) => Some(severity_from_name(name).with_context(|| {
                    format!("`{name}` is not a severity. Use output, info, warn or error.")
                })?),
                None => None,
            };

            let version = match version {
                Some(version) => version,
                None => {
                    let options = api.filter_options(universe_id, place_id).await?;
                    options
                        .place_versions()
                        .into_iter()
                        .next()
                        .context("no place version has servers in the last 30 days")?
                }
            };

            let mut lines = Vec::new();
            let mut token: Option<String> = None;
            // Same three rules as `walk_servers`, which the doc comment on
            // that function states at length. Not shared with it: the two
            // walks collect different types and filter on different fields,
            // and a generic over both would be longer than the duplication.
            let page_size = 1u32.max(limit.min(MAX_PAGE_SIZE));

            while (lines.len() as u32) < limit {
                let page = api
                    .list_logs(
                        universe_id,
                        place_id,
                        &version,
                        &job_id,
                        page_size,
                        token.as_deref(),
                    )
                    .await?;
                let next = page.next_token().map(str::to_string);
                let page_was_empty = page.game_server_logs.is_empty();
                for line in page.game_server_logs {
                    if wanted.is_none_or(|want| line.severity == Some(want)) {
                        lines.push(line);
                    }
                }
                match next {
                    Some(value) if !page_was_empty => token = Some(value),
                    _ => break,
                }
            }

            lines.truncate(limit as usize);

            if lines.is_empty() {
                // The advice is worth keeping under `--json` too — a job id
                // from the wrong version returns nothing and no error, which
                // is the mistake this sentence exists to catch — so it goes to
                // stderr and the document still comes out on stdout.
                format.note(
                    "No logs for that server. Check the job id is complete and the version \
                     matches the row it came from, and that the server is inside the 30-day \
                     window."
                        .dimmed(),
                );
                if format.is_json() {
                    output::emit(&LogsDocument::new(&lines, job_id, version, wanted, limit))?;
                }
                return Ok(());
            }

            if format.is_json() {
                output::emit(&LogsDocument::new(&lines, job_id, version, wanted, limit))?;
                return Ok(());
            }

            if csv {
                println!("time,severity,jobId,placeVersion,message,stackTrace");
                for line in &lines {
                    let cells = [
                        line.message_timestamp_ms.clone().unwrap_or_default(),
                        line.severity_name().to_string(),
                        line.job_id.clone().unwrap_or_default(),
                        line.place_version.clone().unwrap_or_default(),
                        line.message.clone().unwrap_or_default(),
                        line.stack_trace.clone().unwrap_or_default(),
                    ];
                    println!(
                        "{}",
                        cells
                            .iter()
                            .map(|c| csv_field(c))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                }
                return Ok(());
            }

            for line in &lines {
                let severity = line.severity_name();
                let tag = if line.is_error() {
                    severity.red().bold()
                } else if severity == "warn" {
                    severity.yellow()
                } else {
                    severity.dimmed()
                };
                let when = line
                    .message_timestamp_ms
                    .as_deref()
                    .and_then(|stamp| stamp.split('T').nth(1))
                    .and_then(|time| time.split('.').next())
                    .unwrap_or("--:--:--");
                println!(
                    "{when}  {tag:<7}  {}",
                    line.message.as_deref().unwrap_or("")
                );
                // Never truncated: after a crash the stack trace is the whole
                // reason for running this command.
                if let Some(trace) = line.stack_trace.as_deref().filter(|t| !t.is_empty()) {
                    for trace_line in trace.lines() {
                        println!("          {}", trace_line.dimmed());
                    }
                }
            }
            println!();
            println!("{} lines for job {job_id}", lines.len());
        }
    }

    Ok(())
}

/// Roblox answered 200 while telling us one of its two sources failed.
/// Anything computed from these rows is a lower bound, so say so before the
/// numbers rather than after.
///
/// stderr in every format, including `--json`, where the same fact is also in
/// the document as `partial`. A warning on stdout is what breaks `jq`, and the
/// point of having one function is that there is one place to get that wrong.
fn warn_partial() {
    eprintln!(
        "{} this page is incomplete: Roblox reported a fetch error for one of its \
         sources, so rows are missing and any rate computed from them is wrong.",
        "warning:".yellow().bold()
    );
}

/// Quote a CSV field.
///
/// Log messages contain commas and quotes routinely, and a stack trace contains
/// newlines, so this is not optional decoration: an unquoted field silently
/// shifts every column after it.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Every field Roblox returns, not the six the table shows.
///
/// The table is for reading; this is for keeping. A terminated server is gone
/// from Roblox after thirty days, so a scheduled export is the only way to have
/// any history beyond that window.
fn render_csv(rows: &[GameServer]) {
    println!(
        "jobId,status,placeId,placeVersion,engineVersion,createTime,terminationTime,\
         uptimeSeconds,memoryBytes,frameRate,occupancy,maxOccupancy,full,shutDown,type,playerCount"
    );
    for server in rows {
        let cells = [
            server.job_id.clone().unwrap_or_default(),
            server.status().as_str().to_string(),
            server.place_id.clone().unwrap_or_default(),
            server.place_version.clone().unwrap_or_default(),
            server.engine_version.clone().unwrap_or_default(),
            server.create_time.clone().unwrap_or_default(),
            server.termination_time.clone().unwrap_or_default(),
            // Seconds, not the .NET TimeSpan text: a spreadsheet can add up
            // seconds and cannot add up "00:05:02.0020000".
            server
                .uptime_duration()
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default(),
            server
                .memory_usage_bytes
                .map(|b| b.to_string())
                .unwrap_or_default(),
            // Left empty when Roblox reported nothing, so that a null stays
            // distinguishable from a real zero after the export too.
            server.frame_rate.map(|r| r.to_string()).unwrap_or_default(),
            server.occupancy.map(|o| o.to_string()).unwrap_or_default(),
            server
                .max_occupancy
                .map(|o| o.to_string())
                .unwrap_or_default(),
            server.full.map(|f| f.to_string()).unwrap_or_default(),
            server.shut_down.map(|s| s.to_string()).unwrap_or_default(),
            server.r#type.map(|t| t.to_string()).unwrap_or_default(),
            server
                .player_ids
                .as_ref()
                .map(|ids| ids.len().to_string())
                .unwrap_or_default(),
        ];
        println!(
            "{}",
            cells
                .iter()
                .map(|c| csv_field(c))
                .collect::<Vec<_>>()
                .join(",")
        );
    }
}

fn render(
    rows: &[GameServer],
    version: &str,
    total: Option<i64>,
    limit: u32,
    partial: bool,
    full: bool,
) {
    if partial {
        warn_partial();
    }

    if rows.is_empty() {
        println!("{}", "No servers matched.".dimmed());
        return;
    }

    // Width from the data, not a constant. A truncated job id is 8 characters
    // and a full one is 36, so a fixed width misaligns the header and the
    // footer against the rows as soon as `--full` is passed. `colored` wraps
    // its output in escape sequences, which `{:<n}` counts as width, so the
    // header cells are padded before colouring rather than after.
    let job_width = rows
        .iter()
        .map(|server| server.job_id.as_deref().map_or(1, str::len))
        .max()
        .unwrap_or(8)
        .max("JOB".len());

    println!(
        "{}  {}  {}  {}  {}  {}",
        format_args!("{:<10}", "STATUS").to_string().bold(),
        format_args!("{:<job_width$}", "JOB").to_string().bold(),
        format_args!("{:>9}", "UPTIME").to_string().bold(),
        format_args!("{:>7}", "MEMORY").to_string().bold(),
        format_args!("{:>6}", "FPS").to_string().bold(),
        format_args!("{:<12}", "PLAYERS").to_string().bold()
    );

    for server in rows {
        let status = server.status();
        let status_text = match () {
            _ if status.is_failure() => status.to_string().red().bold(),
            _ if status == ServerStatus::Active => status.to_string().green(),
            _ => status.to_string().normal(),
        };
        let job = server
            .job_id
            .as_deref()
            .map(|id| {
                if full {
                    id.to_string()
                } else {
                    id.chars().take(8).collect()
                }
            })
            .unwrap_or_else(|| "-".into());
        let uptime = server
            .uptime_duration()
            .map(format_duration)
            .unwrap_or_else(|| "-".into());
        let memory = server
            .memory_usage_bytes
            .map(|bytes| format!("{:.0} MB", bytes as f64 / 1_048_576.0))
            .unwrap_or_else(|| "-".into());
        // `null` means the server never reported one; `0` means it reported
        // zero. Rendering both as "0" would hide a stalled server.
        let fps = server
            .frame_rate
            .map(|rate| format!("{rate:.0}"))
            .unwrap_or_else(|| "-".into());
        let players = match (server.occupancy, server.max_occupancy) {
            (Some(current), Some(max)) => format!("{current}/{max}"),
            (Some(current), None) => current.to_string(),
            _ => "-".into(),
        };

        // `status_text` is already coloured, so it is padded here by hand:
        // `{:<10}` would count the escape sequences towards the width and pad
        // by too little.
        let pad = 10usize.saturating_sub(status.as_str().len());
        println!(
            "{status_text}{:pad$}  {job:<job_width$}  {uptime:>9}  {memory:>7}  {fps:>6}  {players:<12}",
            ""
        );
    }

    let failures = rows.iter().filter(|s| s.status().is_failure()).count();
    println!();
    print!("{} rows for place version {version}", rows.len());
    if let Some(total) = total {
        print!(", {total} exist");
        if (rows.len() as u32) >= limit {
            print!(" ({})", format!("--limit {limit} reached").yellow());
        }
    }
    println!();
    if failures > 0 {
        println!(
            "{}",
            format!("{failures} ended in a crash or out-of-memory")
                .red()
                .bold()
        );
    }
}
