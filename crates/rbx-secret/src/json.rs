//! What `rbx secret` writes to stdout under `--json`.
//!
//! The envelope follows `rbx check --json`: `schema_version` first, then named
//! objects all the way down, optional fields omitted rather than emitted as
//! `null`. Field names are documented in `docs/secret.md` and are the
//! compatibility surface.
//!
//! **No document in this module can carry a secret value.** That is not a
//! convention to be careful about, it is a property of the types: there is no
//! field for one anywhere below. A listing does not have the content to begin
//! with — Roblox never sends it — and `set` reports what it wrote by name and
//! length, never by value. `--json` output is the form most likely to be
//! logged, redirected into a file, or pasted into an issue, so the safe shape
//! is the one that cannot be made unsafe by a future edit to a call site.
//!
//! The one deliberate exception is [`PublicKeyDocument`], which carries a
//! *public* key: publishing it is what it is for.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::model::Secret;

/// One `secret list` invocation.
#[derive(Debug, Serialize)]
pub struct ListDocument {
    pub schema_version: u32,
    pub universe_id: u64,
    /// The `--limit` in force for this run.
    pub limit: u32,
    /// True when the run stopped because it hit `--limit` rather than because
    /// the universe ran out of secrets. Raise `--limit` to see the rest.
    pub limit_reached: bool,
    /// Rows in `secrets`.
    pub count: usize,
    /// One object per secret, in the order Roblox returned them. Empty for a
    /// universe with no secrets, which is a document rather than an error.
    pub secrets: Vec<Row>,
}

/// One secret's metadata. Content is not a field, here or anywhere.
#[derive(Debug, Serialize)]
pub struct Row {
    /// The name `HttpService:GetSecret` takes. **Absent** in the case Roblox
    /// sends a row without one, which the human listing prints as `<no id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The domain the value may be sent to. **Absent** for a secret with no
    /// domain, which cannot leave the server at all — a meaningful state, and
    /// the reason an empty string is normalised away rather than reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Which public key the stored value was sealed under. Compare it against
    /// `rbx secret public-key --json | jq -r .key_id` to find secrets left
    /// behind by a key rotation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
}

impl Row {
    fn new(secret: &Secret) -> Self {
        Self {
            id: secret.id.clone(),
            domain: secret.effective_domain().map(str::to_string),
            key_id: secret.key_id.clone(),
            create_time: secret.create_time.clone(),
            update_time: secret.update_time.clone(),
        }
    }
}

impl ListDocument {
    pub fn new(universe_id: u64, limit: u32, secrets: &[Secret]) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            universe_id,
            limit,
            limit_reached: secrets.len() as u32 >= limit,
            count: secrets.len(),
            secrets: secrets.iter().map(Row::new).collect(),
        }
    }
}

/// One `secret set` invocation that actually wrote.
///
/// A dry run emits no document: `--apply` is what makes this a result, and a
/// consumer that got one either way would have to inspect a field to find out
/// whether anything happened.
#[derive(Debug, Serialize)]
pub struct SetDocument {
    pub schema_version: u32,
    pub universe_id: u64,
    pub id: String,
    /// `"created"` for a secret that did not exist, `"updated"` for one that
    /// replaced a stored value. Worth branching on: `updated` means a previous
    /// value is gone and unrecoverable.
    pub action: &'static str,
    /// Bytes of plaintext, before sealing. The nearest thing to a checksum
    /// that is safe to print — enough to catch the classic mistake of sending
    /// an empty file or a shell variable that never expanded, and not enough
    /// to be worth anything to somebody reading a build log.
    pub bytes: usize,
    /// **Absent** for a secret with no domain, which can never leave the
    /// server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// The key the value was sealed under, as reported by `public-key` at the
    /// moment of the write.
    pub key_id: String,
}

/// One `secret delete` invocation that actually deleted.
#[derive(Debug, Serialize)]
pub struct DeleteDocument {
    pub schema_version: u32,
    pub universe_id: u64,
    pub id: String,
}

/// One `secret public-key` invocation.
#[derive(Debug, Serialize)]
pub struct PublicKeyDocument {
    pub schema_version: u32,
    pub universe_id: u64,
    /// Base64 X25519 public key, exactly as Roblox sent it.
    pub public_key: String,
    /// Submit this alongside anything sealed under `public_key`, or Roblox
    /// stores a value it cannot decrypt.
    pub key_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIVERSE: u64 = 66778899001;

    fn parsed(document: &impl Serialize) -> serde_json::Value {
        let mut buf = Vec::new();
        rbx_core::output::write_json(&mut buf, document).expect("write");
        serde_json::from_slice(&buf).expect("the document must be valid JSON")
    }

    fn secret(json: &str) -> Secret {
        serde_json::from_str(json).expect("fixture")
    }

    #[test]
    fn a_listing_carries_the_columns_the_human_form_prints() {
        let secrets = vec![
            secret(
                r#"{"id":"discord","domain":"discord.com","key_id":"k1",
                    "create_time":"2026-08-01T10:00:00Z","update_time":"2026-08-02T10:00:00Z"}"#,
            ),
            secret(r#"{"id":"signing","domain":"","key_id":"k1"}"#),
        ];
        let doc = parsed(&ListDocument::new(UNIVERSE, 100, &secrets));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["universe_id"], UNIVERSE);
        assert_eq!(doc["count"], 2);
        assert_eq!(doc["limit_reached"], false);
        assert_eq!(doc["secrets"][0]["id"], "discord");
        assert_eq!(doc["secrets"][0]["domain"], "discord.com");
        assert_eq!(doc["secrets"][0]["update_time"], "2026-08-02T10:00:00Z");
        // An empty domain is a real state — "never leaves the server" — and it
        // reads as an absent key, not as `""`.
        assert!(doc["secrets"][1].get("domain").is_none(), "{doc}");
    }

    /// The property the whole module exists for, asserted rather than trusted:
    /// even handed a `Secret` with content in it, a listing cannot print it.
    #[test]
    fn no_listing_can_carry_a_secret_value() {
        let leaky = secret(r#"{"id":"discord","secret":"c3VwZXItc2VjcmV0","key_id":"k1"}"#);
        let doc = parsed(&ListDocument::new(UNIVERSE, 100, &[leaky]));

        let rendered = doc.to_string();
        assert!(!rendered.contains("c3VwZXItc2VjcmV0"), "{rendered}");
        assert!(doc["secrets"][0].get("secret").is_none(), "{rendered}");
    }

    #[test]
    fn a_write_reports_its_size_rather_than_its_content() {
        let doc = parsed(&SetDocument {
            schema_version: SCHEMA_VERSION,
            universe_id: UNIVERSE,
            id: "discord".into(),
            action: "updated",
            bytes: 7,
            domain: Some("discord.com".into()),
            key_id: "k1".into(),
        });

        assert_eq!(doc["action"], "updated");
        assert_eq!(doc["bytes"], 7);
        assert_eq!(doc["id"], "discord");
        assert_eq!(doc["key_id"], "k1");
    }

    /// An empty universe is an empty array and exit 0, not silence: `.count`
    /// answers either way.
    #[test]
    fn a_universe_with_no_secrets_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&ListDocument::new(UNIVERSE, 100, &[]));

        assert_eq!(doc["count"], 0);
        assert_eq!(doc["secrets"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn hitting_the_limit_is_reported_rather_than_left_to_be_inferred() {
        let rows = vec![secret(r#"{"id":"a"}"#), secret(r#"{"id":"b"}"#)];
        assert!(ListDocument::new(UNIVERSE, 2, &rows).limit_reached);
        assert!(!ListDocument::new(UNIVERSE, 3, &rows).limit_reached);
    }
}
