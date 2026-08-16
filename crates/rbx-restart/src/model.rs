//! Types for `server-management/v1` restarts.
//!
//! Roblox keys several of these by place id in a JSON object rather than
//! returning an array, so they are maps here rather than lists.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What a restart would cost, per place, before anything is done.
///
/// This is Roblox's own count, not an estimate computed here, which is why
/// `restart launch` without `--apply` shows it: the dry run is a real answer
/// rather than a guess about one.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Forecast {
    #[serde(default)]
    pub place_forecasts: BTreeMap<String, PlaceForecast>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceForecast {
    /// Players who would be kicked. Not the same as `total_players`: a server
    /// already on the newest version is not restarted, so its players are
    /// counted in the total and not in the impact.
    #[serde(default)]
    pub players_impacted: i32,
    #[serde(default)]
    pub total_players: i32,
    #[serde(default)]
    pub instances_impacted: i32,
    #[serde(default)]
    pub total_instances: i32,
    #[serde(default)]
    pub latest_place_version: Option<String>,
}

impl Forecast {
    pub fn total_players_impacted(&self) -> i32 {
        self.place_forecasts
            .values()
            .map(|p| p.players_impacted)
            .sum()
    }

    pub fn total_instances_impacted(&self) -> i32 {
        self.place_forecasts
            .values()
            .map(|p| p.instances_impacted)
            .sum()
    }

    /// Nothing to restart. Every server is already on the newest version, which
    /// is the normal state some hours after a publish.
    pub fn is_noop(&self) -> bool {
        self.total_instances_impacted() == 0
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartRequest {
    /// Minutes during which players stop being matchmade to servers due for
    /// restart, so most of them leave on their own before anything is closed.
    /// Roblox accepts 1 to 240.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bleed_off_duration_minutes: Option<i32>,
}

/// What Roblox says it actually scheduled.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartLaunched {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub players_impacted: i32,
    #[serde(default)]
    pub instances_impacted: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartStatuses {
    #[serde(default)]
    pub restart_statuses: BTreeMap<String, RestartStatus>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartStatus {
    #[serde(default)]
    pub universe_id: Option<String>,
    /// When the restart was asked for.
    #[serde(default)]
    pub scheduled_time: Option<String>,
    /// When the bleed-off ends and servers actually start closing.
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub place_restart_statuses: BTreeMap<String, PlaceRestartStatus>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceRestartStatus {
    /// `DELAYING` is the bleed-off, `RESTARTING` is servers closing,
    /// `SUCCEEDED` is done. Anything else is passed through rather than
    /// rejected, since this is a beta API.
    #[serde(default)]
    pub state: Option<String>,
}

/// Roblox's stated bounds. Checked here so a bad value fails before the request
/// rather than as a 400 that does not say which field.
pub const MIN_BLEED_OFF_MINUTES: i32 = 1;
pub const MAX_BLEED_OFF_MINUTES: i32 = 240;

pub fn validate_bleed_off(minutes: i32) -> anyhow::Result<()> {
    if !(MIN_BLEED_OFF_MINUTES..=MAX_BLEED_OFF_MINUTES).contains(&minutes) {
        anyhow::bail!(
            "--bleed-off must be between {MIN_BLEED_OFF_MINUTES} and \
             {MAX_BLEED_OFF_MINUTES} minutes, got {minutes}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forecast(json: &str) -> Forecast {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn impact_is_summed_across_places() {
        let f = forecast(
            r#"{"placeForecasts":{
                "1":{"playersImpacted":10,"totalPlayers":40,"instancesImpacted":3,"totalInstances":9},
                "2":{"playersImpacted":5,"totalPlayers":5,"instancesImpacted":2,"totalInstances":2}
            }}"#,
        );
        assert_eq!(f.total_players_impacted(), 15);
        assert_eq!(f.total_instances_impacted(), 5);
        assert!(!f.is_noop());
    }

    #[test]
    fn impacted_is_not_the_same_as_total() {
        // Servers already on the newest version are not restarted, so their
        // players count towards the total and not towards the damage.
        let f = forecast(
            r#"{"placeForecasts":{"1":{"playersImpacted":0,"totalPlayers":120,
                "instancesImpacted":0,"totalInstances":30}}}"#,
        );
        assert_eq!(f.total_players_impacted(), 0);
        assert_eq!(f.place_forecasts["1"].total_players, 120);
    }

    #[test]
    fn nothing_to_restart_is_recognised_rather_than_reported_as_zero_damage() {
        let f = forecast(r#"{"placeForecasts":{"1":{"instancesImpacted":0}}}"#);
        assert!(f.is_noop());
    }

    #[test]
    fn an_empty_forecast_is_a_noop_not_a_parse_failure() {
        assert!(forecast("{}").is_noop());
    }

    #[test]
    fn bleed_off_bounds_come_from_roblox() {
        assert!(validate_bleed_off(1).is_ok());
        assert!(validate_bleed_off(240).is_ok());
        assert!(validate_bleed_off(0).is_err());
        assert!(validate_bleed_off(241).is_err());
        assert!(validate_bleed_off(-5).is_err());
    }

    #[test]
    fn an_absent_bleed_off_is_omitted_so_roblox_picks_its_default() {
        let body = serde_json::to_string(&RestartRequest {
            bleed_off_duration_minutes: None,
        })
        .unwrap();
        assert_eq!(body, "{}");
    }

    #[test]
    fn statuses_are_keyed_by_place() {
        let s: RestartStatuses = serde_json::from_str(
            r#"{"restartStatuses":{"abc":{"scheduledTime":"2026-08-03T20:00:00Z",
                "placeRestartStatuses":{"123":{"state":"DELAYING"}}}}}"#,
        )
        .unwrap();
        assert_eq!(
            s.restart_statuses["abc"].place_restart_statuses["123"].state,
            Some("DELAYING".into())
        );
    }

    #[test]
    fn an_unfamiliar_state_is_passed_through_rather_than_rejected() {
        let s: RestartStatuses = serde_json::from_str(
            r#"{"restartStatuses":{"a":{"placeRestartStatuses":{"1":{"state":"WHATEVER"}}}}}"#,
        )
        .unwrap();
        assert_eq!(
            s.restart_statuses["a"].place_restart_statuses["1"].state,
            Some("WHATEVER".into())
        );
    }
}
