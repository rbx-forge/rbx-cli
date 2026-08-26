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

    /// Free JSON object handed to the game servers being closed.
    ///
    /// Servers scheduled to close fire
    /// `game.ServerRestartScheduled(restartTime, source, attributes)`, and this
    /// is the third argument: a restart reason, an urgency, a message to show
    /// players.
    ///
    /// `None` when the caller asked for nothing, and then the key is left out
    /// of the body entirely rather than sent as `{}`: both arrive in-experience
    /// as an empty table, and the shorter body is the one that cannot be
    /// misread. An explicit `--payload '{}'` is a different thing and is sent
    /// as written, because the caller said so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
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

/// Roblox's stated ceiling on the attributes object, serialised as UTF-8.
pub const MAX_ATTRIBUTE_BYTES: usize = 500;

/// Refuse an attributes payload Roblox would refuse.
///
/// Two rules, both from the vendored `LaunchRestartRequest` schema: it must be
/// a JSON object, and it must serialise to at most
/// [`MAX_ATTRIBUTE_BYTES`] bytes. Checked here for the reason `--bleed-off` is:
/// a restart is launched from inside a deploy, and a 400 that names neither
/// field arrives at the worst possible moment.
///
/// An object is required rather than coerced. A bare string or array would
/// reach the game as something `ServerRestartScheduled` cannot index, and
/// wrapping it in a key this tool invented would put a name in the payload that
/// the caller never wrote.
pub fn validate_attributes(value: &serde_json::Value) -> anyhow::Result<String> {
    if !value.is_object() {
        anyhow::bail!(
            "the attributes payload must be a JSON object: the game receives it as the third \
             argument of `ServerRestartScheduled` and indexes it by key. Got {}.",
            match value {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "a boolean",
                serde_json::Value::Number(_) => "a number",
                serde_json::Value::String(_) => "a string",
                serde_json::Value::Array(_) => "an array",
                serde_json::Value::Object(_) => unreachable!("checked above"),
            }
        );
    }
    let text = serde_json::to_string(value)?;
    let size = text.len();
    if size > MAX_ATTRIBUTE_BYTES {
        anyhow::bail!(
            "the attributes payload is {size} bytes, over Roblox's \
             {MAX_ATTRIBUTE_BYTES}-byte limit. Send a reference rather than the content: a \
             version string or a key the game can look up."
        );
    }
    Ok(text)
}

/// Fold repeated `--attribute k=v` pairs into one JSON object.
///
/// Values stay strings. Guessing at types would make `--attribute count=10`
/// and `--attribute count=1.0` arrive as different Lua types than the text
/// says, and a caller who wants a number has `--payload`.
///
/// A later pair wins over an earlier one with the same key, because the
/// alternative is silently sending one of two values the caller wrote.
pub fn attributes_from_pairs(pairs: &[String]) -> anyhow::Result<serde_json::Value> {
    let mut map = serde_json::Map::with_capacity(pairs.len());
    for pair in pairs {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("--attribute takes `key=value`, got \"{pair}\" with no `=`")
        })?;
        // `trim`, not `is_empty`: a key of one space is the same mistake as a
        // key of none, and it reaches the game as an entry nothing can index.
        // A key that merely *contains* a space is legitimate and survives.
        if key.trim().is_empty() {
            anyhow::bail!("--attribute needs a key before the `=`, got \"{pair}\"");
        }
        map.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    Ok(serde_json::Value::Object(map))
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
            attributes: None,
        })
        .unwrap();
        assert_eq!(body, "{}");
    }

    #[test]
    fn attributes_reach_roblox_under_the_name_it_documents() {
        let body = serde_json::to_string(&RestartRequest {
            bleed_off_duration_minutes: Some(30),
            attributes: Some(serde_json::json!({"reason": "hotfix"})),
        })
        .unwrap();
        assert_eq!(
            body,
            r#"{"bleedOffDurationMinutes":30,"attributes":{"reason":"hotfix"}}"#
        );
    }

    #[test]
    fn pairs_fold_into_one_object_of_strings() {
        let built = attributes_from_pairs(&[
            "reason=hotfix".to_string(),
            "message=Back in 5 minutes".to_string(),
        ])
        .unwrap();
        assert_eq!(
            built,
            serde_json::json!({"reason": "hotfix", "message": "Back in 5 minutes"})
        );
    }

    /// A value is allowed to contain `=`; only the first one separates.
    #[test]
    fn only_the_first_equals_separates() {
        let built = attributes_from_pairs(&["query=a=b".to_string()]).unwrap();
        assert_eq!(built, serde_json::json!({"query": "a=b"}));
    }

    #[test]
    fn an_empty_value_is_a_value_and_a_missing_equals_is_not() {
        assert_eq!(
            attributes_from_pairs(&["reason=".to_string()]).unwrap(),
            serde_json::json!({"reason": ""})
        );
        let err = attributes_from_pairs(&["reason".to_string()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("key=value"), "got: {err}");
        assert!(attributes_from_pairs(&["=hotfix".to_string()]).is_err());
    }

    /// A key of one space is the same mistake as a key of none, and it would
    /// otherwise reach a live restart as an entry nothing can index. A key that
    /// merely contains a space is legitimate and has to survive.
    #[test]
    fn a_key_of_only_whitespace_is_refused_but_one_containing_a_space_is_not() {
        for bad in [" =hotfix", "\t=hotfix", "  =hotfix"] {
            let err = attributes_from_pairs(&[bad.to_string()])
                .unwrap_err()
                .to_string();
            assert!(err.contains("needs a key before"), "got: {err}");
        }
        assert_eq!(
            attributes_from_pairs(&["show message=yes".to_string()]).unwrap(),
            serde_json::json!({"show message": "yes"})
        );
    }

    /// Sending one of two values the caller wrote is the one outcome with no
    /// defence, so the rule is stated and tested rather than left to `insert`.
    #[test]
    fn a_repeated_key_takes_the_last_value() {
        let built =
            attributes_from_pairs(&["reason=first".to_string(), "reason=second".to_string()])
                .unwrap();
        assert_eq!(built, serde_json::json!({"reason": "second"}));
    }

    #[test]
    fn a_payload_that_is_not_an_object_is_refused_by_name() {
        for (value, expected) in [
            (serde_json::json!("hotfix"), "a string"),
            (serde_json::json!([1, 2]), "an array"),
            (serde_json::json!(7), "a number"),
            (serde_json::json!(true), "a boolean"),
            (serde_json::json!(null), "null"),
        ] {
            let err = validate_attributes(&value).unwrap_err().to_string();
            assert!(err.contains("must be a JSON object"), "got: {err}");
            assert!(err.contains(expected), "got: {err}");
        }
        assert!(validate_attributes(&serde_json::json!({})).is_ok());
    }

    /// The bound is Roblox's, and it is checked here so a deploy fails before
    /// the restart rather than on a 400 that names no field.
    ///
    /// Also pins the returned text: the launch prints it rather than
    /// re-serialising, so it has to be the bytes that were measured.
    #[test]
    fn the_five_hundred_byte_ceiling_is_on_the_serialised_bytes() {
        // `{"m":"..."}` is 8 bytes of punctuation around the value.
        let fits = serde_json::json!({ "m": "x".repeat(MAX_ATTRIBUTE_BYTES - 8) });
        let text = validate_attributes(&fits).expect("500 bytes fits");
        assert_eq!(text, serde_json::to_string(&fits).unwrap());
        assert_eq!(text.len(), MAX_ATTRIBUTE_BYTES);

        let over = serde_json::json!({ "m": "x".repeat(MAX_ATTRIBUTE_BYTES - 7) });
        let err = validate_attributes(&over).unwrap_err().to_string();
        assert!(err.contains("501 bytes"), "got: {err}");
        assert!(err.contains("500-byte limit"), "got: {err}");

        // Bytes, not characters: serde_json never escapes non-ASCII, so a
        // multi-byte string costs on the wire what it costs here, which is what
        // Roblox measures. 250 two-byte chars plus 8 of punctuation is 508.
        let wide = serde_json::json!({ "m": "é".repeat(250) });
        assert_eq!(serde_json::to_string(&wide).unwrap().len(), 508);
        assert!(validate_attributes(&wide).is_err());
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
