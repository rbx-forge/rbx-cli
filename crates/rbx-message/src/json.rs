//! The document `rbx message --json` writes.
//!
//! A publish cannot be recalled and Roblox reports nothing about who received
//! it, so the receipt is the only record that the call went out. It carries
//! what the human form prints and nothing more.
//!
//! **Emitted only when nothing failed.** A run that stops before the request:
//! a malformed payload, an oversized message, an env that resolves to no
//! universe: writes nothing to stdout, and neither does a publish Roblox
//! refuses. An empty stdout next to a non-zero exit says "this did not happen"
//! without a consumer having to read a field to find out.
//!
//! **`message` follows the invocation, not the data.** A dry run prints the
//! body it would send, so the document carries it; `--apply` prints only that
//! it went, so the document does not. Echoing a published payload back into
//! stdout would put it in whatever captured the log, for a line the command
//! itself decided not to print.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

/// One `rbx message` invocation.
#[derive(Debug, Serialize)]
pub struct PublishDocument {
    pub schema_version: u32,
    /// The topic as the experience passes it to `SubscribeAsync`.
    pub topic: String,
    /// The universe the message went to, as a string: universe ids exceed
    /// 2^53 and a consumer parsing them as JSON numbers would round them.
    pub universe_id: String,
    /// Encoded length of the message, which is what Roblox's limit applies to.
    pub bytes: usize,
    /// True when the message was sent. False on a dry run, which is the
    /// default: `--apply` is what sends.
    pub applied: bool,
    /// The body that would be sent. **Absent** once it has been, see the
    /// module docs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PublishDocument {
    /// The dry run: what would go out, including the body.
    pub fn planned(topic: &str, universe_id: u64, message: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            topic: topic.to_string(),
            universe_id: universe_id.to_string(),
            bytes: message.len(),
            applied: false,
            message: Some(message.to_string()),
        }
    }

    /// The receipt of a publish Roblox accepted.
    pub fn sent(topic: &str, universe_id: u64, message: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            topic: topic.to_string(),
            universe_id: universe_id.to_string(),
            bytes: message.len(),
            applied: true,
            message: None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn value(doc: &PublishDocument) -> serde_json::Value {
        serde_json::to_value(doc).unwrap()
    }

    /// The id is a string on the way out, whatever it is in memory: a universe
    /// id past 2^53 read back as a JSON number is a different universe.
    #[test]
    fn the_universe_id_is_a_string() {
        let doc = value(&PublishDocument::sent("cache", 9007199254740993, "x"));
        assert_eq!(doc["universe_id"], "9007199254740993");
    }

    /// The byte count is the encoded length, not the character count, because
    /// that is the number Roblox's limit is expressed in.
    #[test]
    fn the_byte_count_counts_bytes_rather_than_characters() {
        let doc = value(&PublishDocument::planned("cache", 1, "héllo"));
        assert_eq!(doc["bytes"], 6);
    }

    /// A dry run says what it would send; a publish that went says only that
    /// it went. See the module docs for why the shape follows the invocation.
    #[test]
    fn the_body_travels_on_a_dry_run_and_not_after_apply() {
        let planned = value(&PublishDocument::planned("cache", 1, "reload"));
        assert_eq!(planned["message"], "reload");
        assert_eq!(planned["applied"], false);

        let sent = value(&PublishDocument::sent("cache", 1, "reload"));
        assert!(
            sent.get("message").is_none(),
            "a sent publish must not echo its body: {sent}"
        );
        assert_eq!(sent["applied"], true);
        assert_eq!(
            sent["bytes"], 6,
            "the count survives the body being dropped"
        );
    }
}
