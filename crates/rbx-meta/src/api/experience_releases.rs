//! Roblox "Experience Releases" endpoint: the one Creator Hub hits for the
//! `Enable Beta mode` toggle. Cookie-based, requires CSRF.

use anyhow::Result;
use rbx_core::api::roblox_error;
use reqwest::header;
use serde::Deserialize;
use serde_json::json;

use super::RbxClient;

const PATH: &str = "/experience-releases/v1beta1/experience_releases_api";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseStatusResponse {
    release_status: Option<String>,
}

impl RbxClient {
    /// Read the current "release status" for the universe. Returns `true` when
    /// Beta mode is enabled (`RELEASE_STATUS_BETA`), `false` for anything else
    /// (`RELEASE_STATUS_NONE` / unset / unknown).
    pub async fn get_beta_mode(&self) -> Result<Option<bool>> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.api_url(&format!("{}/release_status/{}", PATH, self.universe_id));

        let response = self
            .client
            .get(&url)
            .header(header::COOKIE, format!(".ROBLOSECURITY={}", cookie))
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(roblox_error(status, &body).context("reading the release status"));
        }
        let parsed: ReleaseStatusResponse = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse release_status response: {}\nBody: {}",
                e,
                body
            )
        })?;
        Ok(parsed.release_status.map(|s| s == "RELEASE_STATUS_BETA"))
    }

    /// Set the universe's Beta mode. `true` → RELEASE_STATUS_BETA, `false` →
    /// RELEASE_STATUS_NONE. Handles CSRF transparently.
    pub async fn set_beta_mode(&self, enabled: bool) -> Result<()> {
        let cookie = self.cookie_header()?.to_string();
        let url = self.api_url(&format!("{PATH}/release_status"));
        let cookie_header = format!(".ROBLOSECURITY={}", cookie);
        let release_status = if enabled {
            "RELEASE_STATUS_BETA"
        } else {
            "RELEASE_STATUS_NONE"
        };
        let body = json!({
            "universeId": self.universe_id,
            "releaseStatus": release_status,
        });

        let build = || {
            self.client
                .post(&url)
                .header(header::COOKIE, &cookie_header)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://create.roblox.com")
                .header(header::REFERER, "https://create.roblox.com/")
                .json(&body)
        };

        // The dance is `rbx_core::api::send_with_csrf`; what is here is the
        // sentence a refusal turns into. The hint has to be read off the raw
        // body, which is why the shared helper hands back a `Refusal` rather
        // than a formatted error: building one first would have eaten the body
        // this reads.
        match rbx_core::api::send_with_csrf(&self.csrf_token, build).await {
            Ok(_) => Ok(()),
            Err(rbx_core::api::CsrfError::Transport(e)) => Err(e),
            Err(rbx_core::api::CsrfError::Refused(r)) => {
                let hint = beta_mode_hint(&r.body);
                let retried = if r.retried {
                    " (retried with a refreshed CSRF token)"
                } else {
                    ""
                };
                Err(roblox_error(r.status, &r.body).context(format!(
                    "setting beta_mode on universe {}{}{}",
                    self.universe_id, retried, hint
                )))
            }
        }
    }
}

/// Build a human-friendly hint for known Roblox error patterns on the
/// experience-releases endpoint. Returns an empty string when nothing matches.
fn beta_mode_hint(body: &str) -> String {
    if body.contains("too recently") {
        let remaining = parse_remaining_minutes(body);
        match remaining {
            Some(mins) => format!(
                "\n  hint: Roblox enforces a ~10 min cooldown after DISABLING Beta mode before \
                 it can be re-enabled.\n         Try again in ~{} min.",
                mins.max(1)
            ),
            None => {
                "\n  hint: Roblox enforces a ~10 min cooldown after DISABLING Beta mode before \
                     it can be re-enabled. Try again in a few minutes."
                    .to_string()
            }
        }
    } else {
        String::new()
    }
}

/// Parse `"(<elapsed> < <required> days)"` out of the error body and return
/// the remaining wait time in minutes (ceil). Returns None if the body
/// doesn't match the expected shape.
fn parse_remaining_minutes(body: &str) -> Option<u64> {
    let days_idx = body.find(" days)")?;
    let before_close = &body[..days_idx];
    let open_idx = before_close.rfind('(')?;
    let nums = &before_close[open_idx + 1..];
    let (elapsed_str, required_str) = nums.split_once(" < ")?;
    let elapsed: f64 = elapsed_str.trim().parse().ok()?;
    let required: f64 = required_str.trim().parse().ok()?;
    let remaining_days = (required - elapsed).max(0.0);
    let remaining_mins = (remaining_days * 24.0 * 60.0).ceil() as u64;
    Some(remaining_mins)
}
