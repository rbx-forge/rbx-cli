//! Types for `server-management/v1` game servers.
//!
//! Every non-obvious decision in this file comes from a real response, not
//! from the OpenAPI spec. The spec is right about which fields exist and wrong
//! or silent about how several of them are encoded. Fixtures in
//! `tests/fixtures/` are recorded responses; the tests below are the contract.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Server status as Roblox reports it. `Unknown` is deliberate: this is a beta
/// API, and a status we have never seen must not turn a whole page into a
/// deserialization error.
///
/// `crashed` was confirmed against the live game. `out_of_memory`, `restarted`,
/// `roblox_restarted` and `moderated` are documented but have not been seen, so
/// they are named and otherwise untested.
/// `Serialize` as well as `Deserialize`, so `--json` reports a status with the
/// same spelling `--status` filters on and the CSV column carries.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    Active,
    ShutDown,
    Restarted,
    RobloxRestarted,
    Crashed,
    OutOfMemory,
    Moderated,
    #[serde(other)]
    Unknown,
}

impl ServerStatus {
    /// Whether this status means the server stopped for a bad reason. Used to
    /// decide what to highlight, and what an alert would count.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Crashed | Self::OutOfMemory)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::ShutDown => "shut_down",
            Self::Restarted => "restarted",
            Self::RobloxRestarted => "roblox_restarted",
            Self::Crashed => "crashed",
            Self::OutOfMemory => "out_of_memory",
            Self::Moderated => "moderated",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServer {
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub place_id: Option<String>,
    #[serde(default)]
    pub place_version: Option<String>,
    #[serde(default)]
    pub engine_version: Option<String>,
    #[serde(default)]
    pub create_time: Option<String>,
    #[serde(default)]
    pub termination_time: Option<String>,
    /// .NET TimeSpan text. Use [`GameServer::uptime_duration`] rather than
    /// reading this.
    #[serde(default)]
    pub uptime: Option<String>,
    #[serde(default)]
    pub memory_usage_bytes: Option<u64>,
    /// `null` on a server too young to have reported one, `0.0` on a server
    /// that has stopped. Those are different facts, so this stays an `Option`
    /// and is never defaulted to zero.
    #[serde(default)]
    pub frame_rate: Option<f32>,
    #[serde(default)]
    pub occupancy: Option<i32>,
    #[serde(default)]
    pub max_occupancy: Option<i32>,
    /// Integer enum 0..=5 in the spec, with no names given anywhere. Only 1 and
    /// 3 were ever observed. Kept raw rather than guessed at.
    #[serde(default)]
    pub r#type: Option<i32>,
    #[serde(default)]
    pub full: Option<bool>,
    #[serde(default)]
    pub shut_down: Option<bool>,
    #[serde(default)]
    pub status: Option<ServerStatus>,
    #[serde(default)]
    pub player_ids: Option<Vec<i64>>,
}

impl GameServer {
    pub fn uptime_duration(&self) -> Option<Duration> {
        self.uptime.as_deref().and_then(parse_timespan)
    }

    pub fn status(&self) -> ServerStatus {
        self.status.clone().unwrap_or(ServerStatus::Unknown)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerPage {
    #[serde(default)]
    pub game_servers: Vec<GameServer>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub previous_page_token: Option<String>,
    #[serde(default)]
    pub total_count: Option<i64>,
    /// True when shutdown rows could not be fetched and this page is therefore
    /// a partial slice. Roblox still answers 200. Ignoring these two fields is
    /// how you compute a crash rate that is quietly wrong.
    #[serde(default)]
    pub shutdown_servers_fetch_error: Option<bool>,
    #[serde(default)]
    pub active_servers_fetch_error: Option<bool>,
}

impl GameServerPage {
    /// The token to request the next page, or `None` when there is none.
    ///
    /// `server-management` sends `null` at the end, `cloud/v2` sends `""`.
    /// Treating an empty string as a token asks for the same page forever, so
    /// both collapse to `None` here.
    pub fn next_token(&self) -> Option<&str> {
        self.next_page_token
            .as_deref()
            .filter(|token| !token.is_empty())
    }

    /// Whether this page is knowingly incomplete.
    pub fn is_partial(&self) -> bool {
        self.shutdown_servers_fetch_error.unwrap_or(false)
            || self.active_servers_fetch_error.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterOptions {
    #[serde(default)]
    pub filters: FilterMap,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FilterMap {
    #[serde(default)]
    pub place_version: Option<FilterField>,
    #[serde(default)]
    pub engine_version: Option<FilterField>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterField {
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
}

impl FilterOptions {
    /// Place versions that actually have servers, newest first.
    ///
    /// This call is not optional. `ListGameServers` takes the version in its
    /// path and there is no "all versions" form, so a caller has no way to name
    /// a version without asking for this list first.
    pub fn place_versions(&self) -> Vec<String> {
        let mut versions: Vec<String> = self
            .filters
            .place_version
            .as_ref()
            .map(|field| {
                field
                    .values
                    .iter()
                    .filter_map(|value| match value {
                        serde_json::Value::Number(n) => Some(n.to_string()),
                        serde_json::Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Numeric where possible: "3991" must sort above "982".
        versions.sort_by(|a, b| match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(x), Ok(y)) => y.cmp(&x),
            _ => b.cmp(a),
        });
        versions
    }
}

/// Parse a .NET `TimeSpan`: `[d.]hh:mm:ss[.fffffff]`.
///
/// Not ISO 8601, despite the spec calling the field a duration. All three of
/// these are real:
///
/// ```text
/// 00:00:07             a server seven seconds old
/// 00:05:02.0020000     seven fractional digits (100ns ticks)
/// 1.02:03:04           a day component, from the spec's own example
/// ```
///
/// Only the first two were ever seen in production, because Roblox recycles
/// servers well before they reach a day. The third is handled anyway: the first
/// server that outlives a day would otherwise silently fail to parse.
///
/// Returns `None` rather than erroring. An unparseable uptime should degrade one
/// column of one row, not fail the command.
pub fn parse_timespan(text: &str) -> Option<Duration> {
    let text = text.trim();
    let mut parts = text.split(':');
    let head = parts.next()?;
    let minutes_text = parts.next()?;
    let tail = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    // A dot in the head is the day separator (`1.02`); a dot in the tail is the
    // fractional second (`02.0020000`). Same character, different meaning,
    // decided by position.
    let (days, hours_text) = match head.split_once('.') {
        Some((days, hours)) => (days.parse::<u64>().ok()?, hours),
        None => (0, head),
    };
    let (seconds_text, fraction_text) = match tail.split_once('.') {
        Some((seconds, fraction)) => (seconds, fraction),
        None => (tail, ""),
    };

    let hours = hours_text.parse::<u64>().ok()?;
    let minutes = minutes_text.parse::<u64>().ok()?;
    let seconds = seconds_text.parse::<u64>().ok()?;
    if minutes > 59 || seconds > 59 {
        return None;
    }

    let nanos = if fraction_text.is_empty() {
        0
    } else {
        if !fraction_text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // .NET writes 7 digits (100ns ticks); pad or truncate to nanoseconds.
        let mut digits = fraction_text.to_string();
        digits.truncate(9);
        while digits.len() < 9 {
            digits.push('0');
        }
        digits.parse::<u32>().ok()?
    };

    let total = days
        .checked_mul(86_400)?
        .checked_add(hours.checked_mul(3_600)?)?
        .checked_add(minutes.checked_mul(60)?)?
        .checked_add(seconds)?;
    Some(Duration::new(total, nanos))
}

/// Render a duration the way an operator reads it: `2h 05m`, `3d 04h`, `7s`.
pub fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let (days, hours, minutes, seconds) = (
        total / 86_400,
        total % 86_400 / 3_600,
        total % 3_600 / 60,
        total % 60,
    );
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// One line a server wrote. Available for 30 days after the server ends, the
/// same window as the server row itself.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerLog {
    #[serde(default)]
    pub message_timestamp_ms: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub place_version: Option<String>,
    /// `Output(0)`, `Info(1)`, `Warning(2)`, `Error(3)`. Unlike the server
    /// `type` enum, this one the spec actually names.
    #[serde(default)]
    pub severity: Option<i32>,
    #[serde(default)]
    pub message: Option<String>,
    /// Present on errors. This is the reason to reach for logs at all after a
    /// crash, so it is never truncated on display.
    #[serde(default)]
    pub stack_trace: Option<String>,
}

impl GameServerLog {
    pub fn severity_name(&self) -> &'static str {
        match self.severity {
            Some(0) => "output",
            Some(1) => "info",
            Some(2) => "warn",
            Some(3) => "error",
            _ => "?",
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Some(3)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerLogPage {
    #[serde(default)]
    pub game_server_logs: Vec<GameServerLog>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

impl GameServerLogPage {
    pub fn next_token(&self) -> Option<&str> {
        self.next_page_token
            .as_deref()
            .filter(|token| !token.is_empty())
    }
}

/// Map a severity name a person would type to the number Roblox filters on.
pub fn severity_from_name(name: &str) -> Option<i32> {
    match name.trim().to_ascii_lowercase().as_str() {
        "output" => Some(0),
        "info" => Some(1),
        "warn" | "warning" => Some(2),
        "error" => Some(3),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_plain_form_seen_on_a_young_server() {
        assert_eq!(parse_timespan("00:00:07"), Some(Duration::from_secs(7)));
    }

    #[test]
    fn parses_the_seven_digit_fraction_seen_on_a_stopped_server() {
        let parsed = parse_timespan("00:05:02.0020000").unwrap();
        assert_eq!(parsed.as_secs(), 302);
        assert_eq!(parsed.subsec_nanos(), 2_000_000);
    }

    #[test]
    fn parses_the_day_component_from_the_spec_example() {
        // Never observed in production. Handled so the first server that
        // outlives a day does not silently fail to parse.
        let parsed = parse_timespan("1.02:03:04").unwrap();
        assert_eq!(parsed.as_secs(), 86_400 + 2 * 3_600 + 3 * 60 + 4);
    }

    #[test]
    fn parses_the_longest_uptime_actually_observed() {
        let parsed = parse_timespan("02:05:41.4300000").unwrap();
        assert_eq!(parsed.as_secs(), 2 * 3_600 + 5 * 60 + 41);
    }

    #[test]
    fn a_dot_in_the_head_is_days_and_a_dot_in_the_tail_is_a_fraction() {
        // The distinction the whole parser turns on.
        assert_eq!(parse_timespan("1.00:00:00").unwrap().as_secs(), 86_400);
        assert_eq!(parse_timespan("00:00:01.5000000").unwrap().as_secs(), 1);
    }

    #[test]
    fn rejects_shapes_that_are_not_timespans() {
        for bad in [
            "", "abc", "1:2", "1:2:3:4", "00:60:00", "00:00:60", "00:00:0x",
        ] {
            assert!(parse_timespan(bad).is_none(), "should reject {bad:?}");
        }
    }

    #[test]
    fn an_empty_next_page_token_means_the_end_not_another_page() {
        // cloud/v2 sends "" where server-management sends null. Treating ""
        // as a token requests the same page forever.
        let page: GameServerPage = serde_json::from_str(r#"{"nextPageToken":""}"#).unwrap();
        assert_eq!(page.next_token(), None);
    }

    #[test]
    fn a_null_next_page_token_means_the_end() {
        let page: GameServerPage = serde_json::from_str(r#"{"nextPageToken":null}"#).unwrap();
        assert_eq!(page.next_token(), None);
    }

    #[test]
    fn a_real_token_is_returned() {
        let page: GameServerPage = serde_json::from_str(r#"{"nextPageToken":"abc"}"#).unwrap();
        assert_eq!(page.next_token(), Some("abc"));
    }

    #[test]
    fn null_fetch_error_flags_do_not_mean_partial() {
        let page: GameServerPage =
            serde_json::from_str(r#"{"shutdownServersFetchError":null}"#).unwrap();
        assert!(!page.is_partial());
    }

    #[test]
    fn a_raised_fetch_error_flag_marks_the_page_partial() {
        let page: GameServerPage =
            serde_json::from_str(r#"{"shutdownServersFetchError":true}"#).unwrap();
        assert!(page.is_partial(), "a 200 can still carry incomplete data");
    }

    #[test]
    fn an_unrecognised_status_does_not_fail_the_page() {
        let page: GameServerPage =
            serde_json::from_str(r#"{"gameServers":[{"status":"teleported_away"}]}"#).unwrap();
        assert_eq!(page.game_servers[0].status(), ServerStatus::Unknown);
    }

    #[test]
    fn crashed_and_out_of_memory_are_the_failure_statuses() {
        assert!(ServerStatus::Crashed.is_failure());
        assert!(ServerStatus::OutOfMemory.is_failure());
        assert!(!ServerStatus::ShutDown.is_failure());
        assert!(!ServerStatus::Active.is_failure());
    }

    #[test]
    fn a_null_frame_rate_is_kept_distinct_from_zero() {
        let page: GameServerPage =
            serde_json::from_str(r#"{"gameServers":[{"frameRate":null},{"frameRate":0}]}"#)
                .unwrap();
        assert_eq!(page.game_servers[0].frame_rate, None);
        assert_eq!(page.game_servers[1].frame_rate, Some(0.0));
    }

    #[test]
    fn durations_render_at_the_scale_an_operator_reads() {
        assert_eq!(format_duration(Duration::from_secs(7)), "7s");
        assert_eq!(format_duration(Duration::from_secs(302)), "5m 02s");
        assert_eq!(format_duration(Duration::from_secs(7541)), "2h 05m");
        assert_eq!(format_duration(Duration::from_secs(273_784)), "3d 04h");
    }

    #[test]
    fn place_versions_come_back_newest_first_and_numerically() {
        let options: FilterOptions =
            serde_json::from_str(r#"{"filters":{"PlaceVersion":{"values":[982,3991,3982]}}}"#)
                .unwrap();
        assert_eq!(options.place_versions(), vec!["3991", "3982", "982"]);
    }

    #[test]
    fn missing_filter_options_yield_no_versions_rather_than_an_error() {
        let options: FilterOptions = serde_json::from_str(r#"{"filters":{}}"#).unwrap();
        assert!(options.place_versions().is_empty());
    }

    #[test]
    fn severities_map_to_the_names_the_spec_gives_them() {
        for (value, name) in [(0, "output"), (1, "info"), (2, "warn"), (3, "error")] {
            let log = GameServerLog {
                severity: Some(value),
                message_timestamp_ms: None,
                job_id: None,
                place_version: None,
                message: None,
                stack_trace: None,
            };
            assert_eq!(log.severity_name(), name);
        }
    }

    #[test]
    fn an_unknown_severity_is_shown_rather_than_guessed_at() {
        let page: GameServerLogPage =
            serde_json::from_str(r#"{"gameServerLogs":[{"severity":99}]}"#).unwrap();
        assert_eq!(page.game_server_logs[0].severity_name(), "?");
        assert!(!page.game_server_logs[0].is_error());
    }

    #[test]
    fn severity_names_are_accepted_in_the_forms_people_type() {
        assert_eq!(severity_from_name("error"), Some(3));
        assert_eq!(severity_from_name("ERROR"), Some(3));
        assert_eq!(severity_from_name(" warning "), Some(2));
        assert_eq!(severity_from_name("warn"), Some(2));
        assert_eq!(severity_from_name("nonsense"), None);
    }

    #[test]
    fn a_log_page_ends_on_an_empty_token_like_every_other_listing() {
        let page: GameServerLogPage = serde_json::from_str(r#"{"nextPageToken":""}"#).unwrap();
        assert_eq!(page.next_token(), None);
    }
}
