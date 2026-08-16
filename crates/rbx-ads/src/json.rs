//! What `rbx ads list|get|status --json` write to stdout.
//!
//! Separate from `model` on purpose. `model` describes what Roblox sends and
//! what this crate posts back, in the API's own camelCase; this describes what
//! we promise, in snake_case, and the two are allowed to drift. Roblox ships
//! `/ads-management/v1` as an experiment and says the shapes can change, which
//! makes that separation load-bearing here rather than decorative: a field
//! renamed upstream is a parsing change in `model`, not a break in somebody's
//! `jq` filter.
//!
//! The envelope follows `rbx check --json`: a `schema_version` first, a
//! `totals` object, then named rows. Field names are documented in
//! `docs/ops/ads.md` and are the compatibility surface.
//!
//! # Money
//!
//! Every amount appears twice, and neither copy is a JSON number.
//! `amount_micros` is the string Roblox sent, exact to the millionth of a
//! dollar; `amount_usd` is the same value as a decimal string, the figure the
//! human output prints. A float would put a budget at 24.999999 dollars, and a
//! campaign's budget is real money.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::model::{self, MICROS_PER_USD};

/// One `ads list` invocation.
#[derive(Debug, Serialize)]
pub struct ListDocument {
    pub schema_version: u32,
    pub totals: ListTotals,
    /// One object per campaign, in the order Roblox returned them, every page
    /// followed.
    pub campaigns: Vec<Campaign>,
}

#[derive(Debug, Serialize)]
pub struct ListTotals {
    /// Rows in `campaigns`.
    pub returned: usize,
    /// How many of `returned` Roblox reports as `ACTIVE`. Not the same as
    /// serving: a campaign can be active and still in review, which is what
    /// `delivery_status` says.
    pub active: usize,
}

/// One `ads get` invocation.
///
/// The campaign sits under a named key rather than at the top level, so one
/// filter reads a campaign out of either document: `.campaign` here,
/// `.campaigns[]` from `list`.
#[derive(Debug, Serialize)]
pub struct GetDocument {
    pub schema_version: u32,
    pub campaign: Campaign,
}

/// One campaign.
#[derive(Debug, Serialize)]
pub struct Campaign {
    pub id: String,
    /// Free text, and load-bearing: `launch` writes the creative's asset id
    /// into it because Ads Manager reports per campaign, and the name is the
    /// only thread from a row there back to the image it carried.
    pub name: String,
    /// What the campaign was asked to do: `ACTIVE`, `PAUSED`, `CANCELLED`.
    pub status: String,
    /// What it is actually doing: `SERVING`, `IN_REVIEW`, `NOT_SERVING`,
    /// `REJECTED`. A campaign can be `ACTIVE` and not serving, so an alert
    /// that reads only `status` reports a test that never started as running.
    pub delivery_status: String,
    /// Why, when Roblox says. Always an array, empty when it said nothing:
    /// this is the one piece of feedback the API returns on a rejection.
    pub delivery_status_reasons: Vec<String>,
    /// **Absent** when Roblox reported no budget, which is not the same as a
    /// budget of zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    /// The experience being advertised. A string: universe ids exceed 2^53 and
    /// a JSON number would round in a JavaScript consumer.
    pub target_universe_id: String,
    /// The images this campaign carries. `launch` creates one campaign per
    /// creative, so this is normally one entry.
    pub creative_asset_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Budget {
    /// Micro-USD, exactly as Roblox sent it. A string, and the authoritative
    /// figure: `25000000` is $25.00.
    pub amount_micros: String,
    /// The same amount in dollars, as a decimal string truncated to the cent —
    /// the figure the human output prints. **Absent** when `amount_micros` is
    /// not a number, in which case the raw field is all there is to report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_usd: Option<String>,
    /// `DAILY` or `LIFETIME`. Named `type` because that is what the API calls
    /// it and what `--budget-type` sets.
    #[serde(rename = "type")]
    pub budget_type: String,
}

/// Render micro-USD as a decimal string, or `None` when Roblox sent something
/// that is not a number.
///
/// Digits throughout, never a float: `micros_to_usd` next door prints the same
/// figure for a person, and this is the same arithmetic without the `$`, which
/// a consumer would only have to strip before doing anything with it.
fn usd_decimal(micros: &str) -> Option<String> {
    micros.parse::<u64>().ok().map(|value| {
        format!(
            "{}.{:02}",
            value / MICROS_PER_USD,
            (value % MICROS_PER_USD) / 10_000
        )
    })
}

impl From<&model::Campaign> for Campaign {
    fn from(campaign: &model::Campaign) -> Self {
        Self {
            id: campaign.id.clone(),
            name: campaign.name.clone(),
            status: campaign.status.clone(),
            delivery_status: campaign.delivery_status.clone(),
            delivery_status_reasons: campaign.delivery_status_reasons.clone(),
            budget: campaign.budget.as_ref().map(|budget| Budget {
                amount_usd: usd_decimal(&budget.amount_micros),
                amount_micros: budget.amount_micros.clone(),
                budget_type: budget.budget_type.clone(),
            }),
            target_universe_id: campaign.target_universe_id.clone(),
            creative_asset_ids: campaign.creative_asset_ids.clone(),
        }
    }
}

impl ListDocument {
    /// Build the document from every campaign on the account.
    pub fn new(campaigns: &[model::Campaign]) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            totals: ListTotals {
                returned: campaigns.len(),
                active: campaigns.iter().filter(|c| c.status == "ACTIVE").count(),
            },
            campaigns: campaigns.iter().map(Campaign::from).collect(),
        }
    }
}

impl GetDocument {
    pub fn new(campaign: &model::Campaign) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            campaign: Campaign::from(campaign),
        }
    }
}

/// One `ads status` invocation.
///
/// `statuses` and `failures` are separate arrays because Roblox answers `200`
/// with both in one body: an id it could not read is not an error for the whole
/// call, and folding the two together would let a monitoring script read a
/// campaign it was never told about as absent rather than as unanswered.
#[derive(Debug, Serialize)]
pub struct StatusDocument {
    pub schema_version: u32,
    pub totals: StatusTotals,
    pub statuses: Vec<Status>,
    /// One object per id Roblox refused to answer for. Always an array, empty
    /// when every id came back.
    pub failures: Vec<Failure>,
}

#[derive(Debug, Serialize)]
pub struct StatusTotals {
    /// Ids given on the command line.
    pub requested: usize,
    /// Ids Roblox answered for: rows in `statuses`.
    pub returned: usize,
    /// Ids it refused: rows in `failures`. A run where this is not zero
    /// answered about fewer campaigns than it was asked about.
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub id: String,
    /// `ACTIVE`, `PAUSED`, `CANCELLED`.
    pub status: String,
    /// `SERVING`, `IN_REVIEW`, `NOT_SERVING`, `REJECTED`.
    pub delivery_status: String,
    /// Why, when Roblox says. Always an array, empty when it said nothing.
    pub delivery_status_reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub id: String,
    /// What Roblox said was wrong with the id.
    pub reason: String,
}

impl StatusDocument {
    /// Build the document from a batch answer. `requested` is the count from
    /// the command line rather than a sum of the two arrays, so an id Roblox
    /// dropped from both is still visible as a difference.
    pub fn new(requested: usize, batch: &model::BatchStatus) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            totals: StatusTotals {
                requested,
                returned: batch.statuses.len(),
                failed: batch.failures.len(),
            },
            statuses: batch
                .statuses
                .iter()
                .map(|status| Status {
                    id: status.id.clone(),
                    status: status.status.clone(),
                    delivery_status: status.delivery_status.clone(),
                    delivery_status_reasons: status.delivery_status_reasons.clone(),
                })
                .collect(),
            failures: batch
                .failures
                .iter()
                .map(|failure| Failure {
                    id: failure.id.clone(),
                    reason: failure.reason.clone(),
                })
                .collect(),
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

    fn campaigns(json: &str) -> Vec<model::Campaign> {
        let page: model::ListCampaigns = serde_json::from_str(json).expect("fixture");
        page.campaigns
    }

    const TWO: &str = r#"{"campaigns":[
        {"id":"c1","name":"icon test [18234567890]","status":"ACTIVE",
         "deliveryStatus":"SERVING","targetUniverseId":"5544332211",
         "creativeAssetIds":["18234567890"],
         "budget":{"amountMicros":"25500000","type":"DAILY"}},
        {"id":"c2","name":"icon test [18234567891]","status":"PAUSED",
         "deliveryStatus":"REJECTED","deliveryStatusReasons":["policy"],
         "targetUniverseId":"5544332211","creativeAssetIds":["18234567891"]}
    ]}"#;

    #[test]
    fn the_list_envelope_carries_the_documented_fields() {
        let doc = parsed(&ListDocument::new(&campaigns(TWO)));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["totals"]["returned"], 2);
        assert_eq!(doc["totals"]["active"], 1);
        assert_eq!(doc["campaigns"][0]["id"], "c1");
        assert_eq!(doc["campaigns"][0]["name"], "icon test [18234567890]");
        assert_eq!(doc["campaigns"][0]["status"], "ACTIVE");
        assert_eq!(doc["campaigns"][0]["delivery_status"], "SERVING");
        assert_eq!(doc["campaigns"][0]["target_universe_id"], "5544332211");
        assert_eq!(doc["campaigns"][0]["creative_asset_ids"][0], "18234567890");
        assert_eq!(doc["campaigns"][1]["delivery_status_reasons"][0], "policy");
    }

    /// The exact figure and the readable one, both as strings. A budget that
    /// went through an f64 is the defect nobody looks for.
    #[test]
    fn a_budget_carries_micros_and_dollars_and_neither_is_a_number() {
        let doc = parsed(&ListDocument::new(&campaigns(TWO)));
        let budget = &doc["campaigns"][0]["budget"];

        assert_eq!(budget["amount_micros"], "25500000");
        assert_eq!(budget["amount_usd"], "25.50");
        assert_eq!(budget["type"], "DAILY");
        assert!(budget["amount_micros"].is_string(), "{budget}");
        assert!(budget["amount_usd"].is_string(), "{budget}");
    }

    /// No budget is an absent key, not a zero: those are different facts and a
    /// consumer summing spend must not read one as the other.
    #[test]
    fn a_missing_budget_is_omitted_rather_than_invented() {
        let doc = parsed(&ListDocument::new(&campaigns(TWO)));

        assert!(doc["campaigns"][1].get("budget").is_none(), "{doc}");
    }

    /// Roblox sends money as a string, and this API is an experiment. A value
    /// that is not a number keeps its raw form rather than failing the run.
    #[test]
    fn an_unparsable_amount_keeps_the_raw_field_and_drops_the_rendered_one() {
        let rows = campaigns(
            r#"{"campaigns":[{"id":"c1","name":"n",
                "budget":{"amountMicros":"not-a-number","type":"LIFETIME"}}]}"#,
        );
        let doc = parsed(&ListDocument::new(&rows));

        assert_eq!(
            doc["campaigns"][0]["budget"]["amount_micros"],
            "not-a-number"
        );
        assert!(
            doc["campaigns"][0]["budget"].get("amount_usd").is_none(),
            "{doc}"
        );
    }

    #[test]
    fn an_account_with_no_campaigns_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&ListDocument::new(&[]));

        assert_eq!(doc["campaigns"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["totals"]["returned"], 0);
        assert_eq!(doc["totals"]["active"], 0);
    }

    /// One campaign lives under a named key, so `.campaign` and `.campaigns[]`
    /// hand a filter the same object.
    #[test]
    fn get_wraps_the_campaign_in_a_named_key() {
        let rows = campaigns(TWO);
        let doc = parsed(&GetDocument::new(&rows[0]));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["campaign"]["id"], "c1");
        assert_eq!(doc["campaign"]["budget"]["amount_usd"], "25.50");
    }

    #[test]
    fn status_keeps_answered_ids_and_refused_ones_apart() {
        let batch: model::BatchStatus = serde_json::from_str(
            r#"{"statuses":[{"id":"c1","status":"ACTIVE","deliveryStatus":"IN_REVIEW"},
                            {"id":"c2","status":"PAUSED","deliveryStatus":"REJECTED",
                             "deliveryStatusReasons":["policy"]}],
                "failures":[{"id":"c3","reason":"not found"}]}"#,
        )
        .expect("fixture");

        let doc = parsed(&StatusDocument::new(3, &batch));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["totals"]["requested"], 3);
        assert_eq!(doc["totals"]["returned"], 2);
        assert_eq!(doc["totals"]["failed"], 1);
        assert_eq!(doc["statuses"][0]["id"], "c1");
        assert_eq!(doc["statuses"][0]["delivery_status"], "IN_REVIEW");
        assert_eq!(doc["statuses"][1]["delivery_status_reasons"][0], "policy");
        assert_eq!(doc["failures"][0]["id"], "c3");
        assert_eq!(doc["failures"][0]["reason"], "not found");
    }

    /// A clean run still emits both arrays, so a filter does not have to test
    /// for the key before walking it.
    #[test]
    fn a_run_with_nothing_refused_still_emits_an_empty_failures_array() {
        let batch: model::BatchStatus =
            serde_json::from_str(r#"{"statuses":[{"id":"c1"}]}"#).expect("fixture");
        let doc = parsed(&StatusDocument::new(1, &batch));

        assert_eq!(doc["failures"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["totals"]["failed"], 0);
    }

    #[test]
    fn micros_render_as_a_decimal_string_without_a_currency_sign() {
        assert_eq!(usd_decimal("25000000").as_deref(), Some("25.00"));
        assert_eq!(usd_decimal("70000").as_deref(), Some("0.07"));
        assert_eq!(usd_decimal("0").as_deref(), Some("0.00"));
        assert_eq!(usd_decimal("not-a-number"), None);
    }
}
