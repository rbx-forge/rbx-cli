//! Wire types for `/ads-management/v1`, and the money conversion.
//!
//! Only the fields this crate reads or sends. The document defines more on
//! `Campaign` (`createTime`, `updateTime`, `billingAccountId`); adding them
//! here would mean maintaining fields nothing prints.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Micro-USD, the unit every money field in this API uses, as a decimal string.
///
/// Roblox sends and accepts money as a string so a large value keeps full
/// precision in clients whose numbers are floats. Keeping it a string here for
/// the same reason: `5000000` is $5.00, and no float ever touches it.
pub const MICROS_PER_USD: u64 = 1_000_000;

/// Parse a dollar amount typed by a human into the micro-USD string the API
/// wants.
///
/// Deliberately not `f64`: `"0.07"` is not representable in binary floating
/// point, and a budget that arrives as 69999 micros instead of 70000 is the
/// kind of defect nobody looks for. The fractional part is read as digits.
pub fn usd_to_micros(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_start_matches('$');
    if trimmed.is_empty() {
        bail!("a budget is required, for example --budget 25 or --budget 25.50");
    }

    let (whole, fraction) = match trimmed.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (trimmed, ""),
    };

    if whole.is_empty() && fraction.is_empty() {
        bail!("`{input}` is not an amount");
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !fraction.chars().all(|c| c.is_ascii_digit()) {
        bail!("`{input}` is not an amount in dollars, for example 25 or 25.50");
    }
    if fraction.len() > 6 {
        bail!("`{input}` is finer than a millionth of a dollar, which the API cannot carry");
    }

    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| anyhow::anyhow!("`{input}` is larger than this tool can express"))?
    };

    // Right-pad to six digits: "5" is 500000 micros, not 5.
    let mut micros = fraction.to_owned();
    while micros.len() < 6 {
        micros.push('0');
    }
    let fraction: u64 = micros.parse().expect("six ascii digits");

    let total = whole
        .checked_mul(MICROS_PER_USD)
        .and_then(|w| w.checked_add(fraction))
        .ok_or_else(|| anyhow::anyhow!("`{input}` is larger than this tool can express"))?;

    Ok(total.to_string())
}

/// Render micro-USD back for display. Input comes from Roblox, so a value that
/// is not a number is shown as-is rather than hidden behind an error.
pub fn micros_to_usd(micros: &str) -> String {
    match micros.parse::<u64>() {
        Ok(value) => format!(
            "${}.{:02}",
            value / MICROS_PER_USD,
            (value % MICROS_PER_USD) / 10_000
        ),
        Err(_) => micros.to_owned(),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub amount_micros: String,
    /// `DAILY` or `LIFETIME`.
    #[serde(rename = "type")]
    pub budget_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_in_days: Option<u32>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Targeting {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub age_groups: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub countries: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Bid {
    /// The document allows one value. Sending it explicitly rather than
    /// omitting the field, so a second strategy arriving later is a compile
    /// error here and not a silent change of behaviour.
    pub strategy: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCampaign {
    pub name: String,
    pub target_universe_id: String,
    pub creative_asset_ids: Vec<String>,
    pub objective: &'static str,
    pub payment_type: String,
    pub budget: Budget,
    pub schedule: Schedule,
    pub bid: Bid,
    #[serde(skip_serializing_if = "targeting_is_empty")]
    pub targeting: Targeting,
}

fn targeting_is_empty(targeting: &Targeting) -> bool {
    targeting.age_groups.is_empty()
        && targeting.countries.is_empty()
        && targeting.devices.is_empty()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCampaign {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<UpdateBudget>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBudget {
    pub amount_micros: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Campaign {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub delivery_status: String,
    #[serde(default)]
    pub delivery_status_reasons: Vec<String>,
    #[serde(default)]
    pub creative_asset_ids: Vec<String>,
    #[serde(default)]
    pub target_universe_id: String,
    pub budget: Option<BudgetRead>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetRead {
    #[serde(default)]
    pub amount_micros: String,
    #[serde(rename = "type", default)]
    pub budget_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCampaigns {
    #[serde(default)]
    pub campaigns: Vec<Campaign>,
    #[serde(default)]
    pub next_page_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignStatus {
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub delivery_status: String,
    #[serde(default)]
    pub delivery_status_reasons: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignIdFailure {
    pub id: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchStatus {
    #[serde(default)]
    pub statuses: Vec<CampaignStatus>,
    #[serde(default)]
    pub failures: Vec<CampaignIdFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creative {
    pub id: String,
    #[serde(default)]
    pub asset_id: String,
    #[serde(default)]
    pub asset_name: String,
    #[serde(default)]
    pub moderation_status: String,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCreatives {
    #[serde(default)]
    pub creatives: Vec<Creative>,
    #[serde(default)]
    pub next_page_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingAccount {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub status: String,
    #[serde(rename = "type", default)]
    pub account_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListBillingAccounts {
    #[serde(default)]
    pub billing_accounts: Vec<BillingAccount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvertisableUniverse {
    #[serde(default)]
    pub universe_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAdvertisableUniverses {
    #[serde(default)]
    pub advertisable_universes: Vec<AdvertisableUniverse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdFormat {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniverseEligibility {
    #[serde(default)]
    pub eligible: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetingDimensions {
    #[serde(default)]
    pub age_groups: Vec<String>,
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub devices: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignOptions {
    #[serde(default)]
    pub ad_formats: Vec<AdFormat>,
    #[serde(default)]
    pub objectives: Vec<String>,
    #[serde(default)]
    pub payment_types: Vec<String>,
    pub targeting_dimensions: Option<TargetingDimensions>,
    pub eligibility: Option<UniverseEligibility>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_dollars_become_micros() {
        assert_eq!(usd_to_micros("25").unwrap(), "25000000");
        assert_eq!(usd_to_micros("$5").unwrap(), "5000000");
        assert_eq!(usd_to_micros(" 1 ").unwrap(), "1000000");
    }

    #[test]
    fn cents_are_read_as_digits_not_as_a_float() {
        // 0.07 has no exact binary representation. Through an f64 this lands
        // on 69999 micros about as often as on 70000.
        assert_eq!(usd_to_micros("0.07").unwrap(), "70000");
        assert_eq!(usd_to_micros("25.50").unwrap(), "25500000");
        assert_eq!(usd_to_micros("25.5").unwrap(), "25500000");
        assert_eq!(usd_to_micros(".5").unwrap(), "500000");
    }

    #[test]
    fn nonsense_is_refused_rather_than_rounded() {
        assert!(usd_to_micros("").is_err());
        assert!(usd_to_micros("abc").is_err());
        assert!(usd_to_micros("-5").is_err());
        assert!(usd_to_micros("1,50").is_err());
        assert!(usd_to_micros("1.2345678").is_err());
    }

    #[test]
    fn micros_render_back_as_dollars() {
        assert_eq!(micros_to_usd("25000000"), "$25.00");
        assert_eq!(micros_to_usd("70000"), "$0.07");
        assert_eq!(micros_to_usd("not-a-number"), "not-a-number");
    }
}
