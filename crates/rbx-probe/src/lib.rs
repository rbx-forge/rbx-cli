//! `rbx-ops probe` : send a raw authenticated request to any Open Cloud path
//! and print the response.
//!
//! Why this exists. Several endpoints we need are in beta and absent from the
//! Open Cloud reference (`server-management/v1` is the current example: the
//! announcement documents it, `llms.txt` does not list it). Writing a typed
//! client against a schema guessed from a forum post is how you ship a
//! deserializer that silently drops a field. `probe` gets the real response
//! first; the recorded body then becomes the fixture the typed client is
//! tested against.
//!
//! It stays useful afterwards, as the thing you reach for when an endpoint
//! starts returning something unexpected and you need to see the bytes.

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;
use reqwest::Method;

use rbx_core::api::{build_client, execute_with_retry, require_api_key, ApiBase};
use rbx_core::GlobalFlags;

#[derive(Args, Debug)]
pub struct ProbeCli {
    /// Open Cloud path, e.g. `/cloud/v2/universes/{universe}/user-restrictions`.
    ///
    /// `{universe}` is replaced with the universe id of the env resolved from
    /// `--env`. A path without that placeholder needs no env.
    ///
    /// The leading slash is optional, and dropping it is the fix when the
    /// shell mangles the argument: Git Bash on Windows rewrites a leading `/`
    /// into a Windows path, so `/cloud/v2/...` arrives as
    /// `C:/Program Files/Git/cloud/v2/...`. `cloud/v2/...` is unaffected, as
    /// is `MSYS_NO_PATHCONV=1`. PowerShell and real POSIX shells are fine
    /// either way.
    pub path: String,

    /// HTTP method. Anything other than GET also requires `--apply`.
    #[arg(short = 'X', long, default_value = "GET")]
    pub method: String,

    /// JSON request body. Parsed before sending so a malformed body fails
    /// here rather than as a confusing 400 from Roblox.
    #[arg(short = 'd', long)]
    pub data: Option<String>,

    /// Actually send a non-GET request.
    ///
    /// Without it, a write is described and not performed. Every write in
    /// `rbx-ops` follows this rule: the safe thing is what happens when you
    /// forget a flag.
    #[arg(long)]
    pub apply: bool,

    /// Override the API host. Intended for testing against a mock server.
    #[arg(long, hide = true)]
    pub base_url: Option<String>,
}

pub async fn run(cli: ProbeCli, global: &GlobalFlags) -> Result<()> {
    let method = Method::from_bytes(cli.method.to_uppercase().as_bytes())
        .with_context(|| format!("`{}` is not a valid HTTP method", cli.method))?;

    let body = match cli.data.as_deref() {
        Some(raw) => Some(
            serde_json::from_str::<serde_json::Value>(raw).context("--data must be valid JSON")?,
        ),
        None => None,
    };

    let base = match &cli.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let path = resolve_path(&cli.path, global)?;
    let url = base.join(&path);

    if method != Method::GET && !cli.apply {
        println!("{} {} {}", "would send".yellow().bold(), method, url);
        if let Some(body) = &body {
            println!("{}", serde_json::to_string_pretty(body)?);
        }
        println!("{}", "Nothing sent. Re-run with --apply.".dimmed());
        return Ok(());
    }

    let api_key = require_api_key(global.api_key.as_deref())?;
    let client = build_client();

    let response = execute_with_retry(|| {
        let mut request = client
            .request(method.clone(), &url)
            .header("x-api-key", api_key);
        if let Some(body) = &body {
            request = request.json(body);
        }
        async move { request.send().await.map_err(Into::into) }
    })
    .await?;

    let status = response.status();
    let text = response.text().await?;

    eprintln!("{} {}", status.as_u16().to_string().bold(), url.dimmed());

    // Pretty-print when the body parses as JSON, pass it through untouched
    // otherwise. An endpoint that answers with HTML (an auth redirect, an
    // outage page) is exactly when you most want to see the raw bytes.
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(json) => println!("{}", serde_json::to_string_pretty(&json)?),
        Err(_) => println!("{text}"),
    }

    Ok(())
}

/// Substitute `{universe}` from the resolved env.
///
/// Resolution is only attempted when the placeholder is present, so probing a
/// path that has no universe in it works in a directory with no
/// `rbxplace.toml`.
fn resolve_path(path: &str, global: &GlobalFlags) -> Result<String> {
    if !path.contains("{universe}") {
        return Ok(path.to_string());
    }

    // `--universe-id` first, so probing works in a directory with no
    // `rbxplace.toml` — which is most of what `probe` is for, since the
    // endpoints it exists to explore are usually ones no config knows about.
    if let Some(universe_id) = global.universe_id {
        return Ok(path.replace("{universe}", &universe_id.to_string()));
    }

    let targets = global.resolve_envs()?;
    match targets.len() {
        0 => bail!(
            "`{{universe}}` in the path needs a target. Pass --env <name>, \
             or --universe-id <id> to name one directly."
        ),
        1 => Ok(path.replace("{universe}", &targets[0].universe_id.to_string())),
        // `--env all` would mean several different requests. Refusing is
        // better than silently probing only the first env.
        _ => bail!(
            "`--env all` is ambiguous for probe. Name a single env; got {}.",
            targets
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags() -> GlobalFlags {
        GlobalFlags {
            api_key: Some("test-key".into()),
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: None,
            place: None,
            places: "rbxplace.toml".into(),
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    #[test]
    fn a_path_without_the_placeholder_is_returned_unchanged() {
        let path = resolve_path("/cloud/v2/users/1", &flags()).unwrap();
        assert_eq!(path, "/cloud/v2/users/1");
    }

    #[test]
    fn the_placeholder_without_an_env_is_an_error_naming_the_flag() {
        let err = resolve_path("/cloud/v2/universes/{universe}", &flags()).unwrap_err();
        assert!(err.to_string().contains("--env"), "got: {err}");
    }
}
