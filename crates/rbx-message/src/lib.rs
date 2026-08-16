//! `rbx message` : send one MessagingService message to every running server.
//!
//! The push half of the pair `memorystore` is the pull half of. A memory store
//! item is read when a server's own code decides to look, so a value written
//! from outside appears on whatever polling interval the experience already
//! has. This is what tells the servers to look now.
//!
//! ## Why this is here at all
//!
//! It was declined once, on the grounds that publishing is server-to-server
//! IPC and belongs to game code rather than to an ops CLI. That reasoning
//! assumed the publisher is a game server. It is not, in the case this exists
//! for: the publisher is a VPS, a cron job, a deploy step — something outside
//! Roblox that has just changed a value and wants the running servers to
//! notice. There is no in-experience way to originate that.
//!
//! ## The message is a string, not JSON
//!
//! Roblox types `message` as a string, so a structured payload is a string
//! containing JSON, decoded in-experience with `HttpService:JSONDecode`. That
//! is worth knowing before designing a topic: passing `--payload` here
//! serialises the value and sends the text, which is the same thing the game
//! will have to undo.
//!
//! ## What it cannot tell you
//!
//! Whether anybody heard. The call answers `200` once Roblox has accepted the
//! message for delivery, with no count of servers reached and no delivery
//! receipt — an experience with no running servers accepts a publish exactly
//! like a busy one. Anything needing confirmation needs the servers to write
//! back somewhere, which is what a memory store map is for.

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;
use reqwest::Client;
use serde::Serialize;

use rbx_core::api::{
    build_client, execute_with_retry, explain_missing_scope, require_api_key, ApiBase,
};
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

pub mod json;

use json::PublishDocument;

/// Roblox's actual ceiling for one message, and its floor.
///
/// Measured against the live API on 2026-08-13, not read off the
/// documentation, which says 1 KB. 1024 was this crate's first guess and it was
/// wrong in the direction that matters: it refused messages between 1025 and
/// 1114 bytes that Roblox accepts. The service states the real bounds itself
/// when you cross them —
/// `The length of published message must be between 1 and 1114.` — and 1114
/// answers 200 while 1115 does not.
///
/// The floor is not decoration either: an empty message is a 400, so a caller
/// that publishes `""` as a bare signal gets a failure at the far end of a
/// deploy rather than here.
const MAX_MESSAGE_BYTES: usize = 1114;

#[derive(Args, Debug)]
pub struct MessageCli {
    /// Topic name, as the game passes to `SubscribeAsync`.
    #[arg(long)]
    topic: String,

    /// Message body, sent as-is.
    #[arg(long, conflicts_with = "payload")]
    message: Option<String>,

    /// Message body as JSON, serialised to the string Roblox expects.
    ///
    /// The same bytes `--message` would send, minus the shell quoting: the
    /// value is parsed here so a malformed payload fails before the publish
    /// rather than inside `JSONDecode` on a live server.
    ///
    /// Spelled `--payload` since 0.12.0. It was `--json`, which now means what
    /// it means on every other command: write the result as a document.
    #[arg(long)]
    payload: Option<String>,

    /// Actually send it.
    #[arg(long)]
    apply: bool,

    /// Write the result to stdout as one JSON document.
    ///
    /// The receipt of a publish that cannot be recalled: the topic, the
    /// universe, the byte count, and whether it was sent or only planned.
    /// Field names are documented in docs/ops/message.md.
    #[arg(long)]
    json: bool,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true)]
    base_url: Option<String>,
}

impl MessageCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Debug, Serialize)]
struct PublishRequest<'a> {
    topic: &'a str,
    message: &'a str,
}

pub async fn run(cli: MessageCli, global: &GlobalFlags) -> Result<()> {
    let format = OutputFormat::from_json_flag(cli.json);
    let message = match (&cli.message, &cli.payload) {
        (Some(text), _) => text.clone(),
        (None, Some(raw)) => {
            let parsed: serde_json::Value =
                serde_json::from_str(raw).context("--payload must be valid JSON")?;
            serde_json::to_string(&parsed)?
        }
        (None, None) => bail!("`publish` needs --message <text> or --payload <json>."),
    };

    // Checked before the request rather than after a 400: the limit is on the
    // encoded bytes, and a message of emoji is longer than its character count
    // suggests.
    let size = message.len();
    if size == 0 {
        bail!(
            "the message is empty, which Roblox refuses. Send something, even a single \
             character, if the point is only to signal that a value changed."
        );
    }
    if size > MAX_MESSAGE_BYTES {
        bail!(
            "the message is {size} bytes, over Roblox's {MAX_MESSAGE_BYTES}-byte limit. \
             Publish a reference instead of a payload: put the value in a memory store map \
             and send the key."
        );
    }

    let universe_id = global.single_universe()?;
    let base = match &cli.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let url = base.join(&format!("/cloud/v2/universes/{universe_id}:publishMessage"));

    if !cli.apply {
        if format.is_json() {
            return output::emit(&PublishDocument::planned(&cli.topic, universe_id, &message));
        }
        println!(
            "would publish to topic \"{}\" in universe {universe_id}",
            cli.topic
        );
        println!("{}", message.dimmed());
        println!("{}", format!("{size} bytes").dimmed());
        println!("{}", "Nothing sent. Re-run with --apply.".dimmed());
        return Ok(());
    }

    let api_key = require_api_key(global.api_key.as_deref())?.to_string();
    let client: Client = build_client();
    let body = PublishRequest {
        topic: &cli.topic,
        message: &message,
    };

    let response = execute_with_retry(|| {
        let request = client.post(&url).header("x-api-key", &api_key).json(&body);
        async move { request.send().await.map_err(Into::into) }
    })
    .await
    .map_err(explain_missing_scope)?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        bail!("publish failed: {status} {text}");
    }

    if format.is_json() {
        return output::emit(&PublishDocument::sent(&cli.topic, universe_id, &message));
    }

    println!(
        "{}",
        format!("✓ published {size} bytes to \"{}\"", cli.topic).green()
    );
    // Said every time, because the success line above is the only feedback and
    // it is easy to read as "the servers got it".
    println!(
        "{}",
        "  Roblox accepted it for delivery. It reports no count of servers reached, and an \
         experience with none running accepts a publish just the same."
            .dimmed()
    );
    Ok(())
}
