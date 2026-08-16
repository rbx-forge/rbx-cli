//! The analytics client against recorded responses from a live experience.
//!
//! Re-record with `python scripts/capture_fixtures.py`.

use rbx_analytics::model::Operation;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}"))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

#[test]
fn a_real_completed_query_parses_into_a_series() {
    let operation: Operation = serde_json::from_str(&fixture("analytics_metrics.json")).unwrap();

    assert!(operation.done);
    assert!(operation.error.is_none());
    let response = operation.response.expect("a completed query has a result");
    let series = &response.values[0];

    // Recorded over four days of a live game, so the shape and the scale are
    // both real: a regression that drops data points fails here.
    assert_eq!(series.data_points.len(), 4);
    assert_eq!(series.label(), "total", "no breakdown was requested");
    assert!(
        series.data_points.iter().all(|p| p.value.unwrap() > 1000.0),
        "the recorded game has tens of thousands of daily players"
    );
    assert!(series.data_points.iter().all(|p| p.time.is_some()));
}

#[test]
fn a_rejected_query_parses_from_the_same_envelope() {
    // This body came back with HTTP 400. The failure still has to parse,
    // because the message inside it is the only thing that says what was wrong.
    let operation: Operation = serde_json::from_str(&fixture("analytics_error.json")).unwrap();

    assert!(operation.done, "a rejection is a finished operation");
    assert!(operation.response.is_none());
    let failure = operation.error.expect("a rejection carries an error");
    assert_eq!(failure.code, Some(2001));
    assert!(
        failure.message.unwrap().contains("was not found"),
        "the message names the bad metric"
    );
}

#[test]
fn an_unfinished_operation_is_recognised_rather_than_read_as_empty() {
    // Observed for real on a 365-day funnel query, and funnel metrics only
    // accept `granularity: None`, so wide ranges are their ordinary case.
    // Reading `done: false` as "no data" would print an empty table instead of
    // waiting for the answer.
    let operation: Operation =
        serde_json::from_str(r#"{"done":false,"path":"v1/universes/1/operations/metrics/abc"}"#)
            .unwrap();

    assert!(!operation.done);
    assert!(operation.response.is_none());
    assert!(operation.error.is_none());
    assert_eq!(
        operation.path.as_deref(),
        Some("v1/universes/1/operations/metrics/abc")
    );
}
