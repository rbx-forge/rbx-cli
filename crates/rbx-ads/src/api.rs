//! The ten operations `/ads-management/v1` exposes, and nothing else.
//!
//! There is no reporting call here because the API has none. Roblox says so
//! outright: reporting is deliberately not in v1 and will arrive through the
//! Analytics API, which `rbx-analytics` already speaks. Until then a campaign's
//! impressions and clicks are readable only in Ads Manager.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;

use rbx_core::api::{execute_with_retry, explain_missing_scope, ApiBase};

use crate::model::{
    BatchStatus, Campaign, CampaignOptions, CreateCampaign, Creative, ListAdvertisableUniverses,
    ListBillingAccounts, ListCampaigns, ListCreatives, UpdateCampaign,
};

/// Turn a response into `T`, or into the error Roblox described.
async fn read<T: DeserializeOwned>(response: reqwest::Response, what: &str) -> Result<T> {
    let status = response.status();
    let body = response
        .text()
        .await
        .with_context(|| format!("reading the response to {what}"))?;

    if !status.is_success() {
        return Err(explain_missing_scope(rbx_core::api::roblox_error(
            status, &body,
        )));
    }

    serde_json::from_str(&body).with_context(|| format!("parsing the response to {what}"))
}

/// A page cannot be the whole answer, and a caller that forgets to follow the
/// token gets a truncated list rather than an error. So no caller is given the
/// chance: these two functions follow the pages themselves.
///
/// `PAGE_LIMIT` is a stop, not a page size. A token that keeps pointing at a
/// next page, which a paging bug on either side can produce, would otherwise
/// loop forever against a paid API.
const PAGE_LIMIT: usize = 100;

pub async fn list_campaigns(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
) -> Result<Vec<Campaign>> {
    let mut all = Vec::new();
    let mut token = String::new();

    for _ in 0..PAGE_LIMIT {
        let mut url = base.join("/ads-management/v1/campaigns");
        if !token.is_empty() {
            url.push_str("?pageToken=");
            url.push_str(&rbx_core::api::encode_query_value(&token));
        }
        let response = execute_with_retry(|| {
            let request = client.get(&url).header("x-api-key", api_key);
            async move { Ok(request.send().await?) }
        })
        .await?;
        let page: ListCampaigns = read(response, "listing campaigns").await?;

        all.extend(page.campaigns);
        if page.next_page_token.is_empty() {
            return Ok(all);
        }
        token = page.next_page_token;
    }

    Ok(all)
}

pub async fn get_campaign(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    id: &str,
) -> Result<Campaign> {
    let url = base.join(&format!("/ads-management/v1/campaigns/{id}"));
    let response = execute_with_retry(|| {
        let request = client.get(&url).header("x-api-key", api_key);
        async move { Ok(request.send().await?) }
    })
    .await?;
    read(response, "reading a campaign").await
}

/// Create one campaign.
///
/// `idempotency_key` is required by Roblox and is the reason a retry cannot
/// cost money twice: `execute_with_retry` resends this request on a timeout or
/// a 429, and without a stable key each resend would be a new campaign with a
/// new budget. The caller derives the key from the campaign's own definition,
/// so a resend and a re-run of the same command both land on the first
/// campaign rather than a second.
pub async fn create_campaign(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    body: &CreateCampaign,
    idempotency_key: &str,
) -> Result<Campaign> {
    let url = base.join("/ads-management/v1/campaigns");
    let response = execute_with_retry(|| {
        let request = client
            .post(&url)
            .header("x-api-key", api_key)
            .header("x-idempotency-key", idempotency_key)
            .json(body);
        async move { Ok(request.send().await?) }
    })
    .await?;
    read(response, "creating a campaign").await
}

pub async fn update_campaign(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    id: &str,
    body: &UpdateCampaign,
) -> Result<Campaign> {
    let url = base.join(&format!("/ads-management/v1/campaigns/{id}"));
    let response = execute_with_retry(|| {
        let request = client.patch(&url).header("x-api-key", api_key).json(body);
        async move { Ok(request.send().await?) }
    })
    .await?;
    read(response, "updating a campaign").await
}

pub async fn batch_status(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    ids: &[String],
) -> Result<BatchStatus> {
    let url = base.join("/ads-management/v1/campaigns:batchGetStatus");
    let body = serde_json::json!({ "campaignIds": ids });
    let response = execute_with_retry(|| {
        let request = client.post(&url).header("x-api-key", api_key).json(&body);
        async move { Ok(request.send().await?) }
    })
    .await?;
    read(response, "reading campaign statuses").await
}

pub async fn list_creatives(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    archived: bool,
) -> Result<Vec<Creative>> {
    let mut all = Vec::new();
    let mut token = String::new();

    for _ in 0..PAGE_LIMIT {
        let mut url = base.join(&format!(
            "/ads-management/v1/creatives?isArchived={archived}"
        ));
        if !token.is_empty() {
            url.push_str("&pageToken=");
            url.push_str(&rbx_core::api::encode_query_value(&token));
        }
        let response = execute_with_retry(|| {
            let request = client.get(&url).header("x-api-key", api_key);
            async move { Ok(request.send().await?) }
        })
        .await?;
        let page: ListCreatives = read(response, "listing creatives").await?;

        all.extend(page.creatives);
        if page.next_page_token.is_empty() {
            return Ok(all);
        }
        token = page.next_page_token;
    }

    Ok(all)
}

pub async fn list_billing_accounts(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
) -> Result<ListBillingAccounts> {
    let url = base.join("/ads-management/v1/billing-accounts");
    let response = execute_with_retry(|| {
        let request = client.get(&url).header("x-api-key", api_key);
        async move { Ok(request.send().await?) }
    })
    .await?;
    read(response, "listing billing accounts").await
}

pub async fn list_advertisable_universes(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
) -> Result<ListAdvertisableUniverses> {
    let url = base.join("/ads-management/v1/advertisable-universes");
    let response = execute_with_retry(|| {
        let request = client.get(&url).header("x-api-key", api_key);
        async move { Ok(request.send().await?) }
    })
    .await?;
    read(response, "listing advertisable experiences").await
}

pub async fn campaign_options(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    universe_id: u64,
) -> Result<CampaignOptions> {
    let url = base.join(&format!(
        "/ads-management/v1/campaign-options?universeId={universe_id}"
    ));
    let response = execute_with_retry(|| {
        let request = client.get(&url).header("x-api-key", api_key);
        async move { Ok(request.send().await?) }
    })
    .await?;
    read(response, "reading campaign options").await
}
