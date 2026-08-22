//! Types for the `cloud/v2` universe secrets store.
//!
//! Two things about the wire format are worth stating, because both differ
//! from every other `cloud/v2` surface in this workspace and neither is
//! guessable:
//!
//! - **The resource is `snake_case`.** `key_id`, `create_time`, `update_time`,
//!   where the rest of `cloud/v2` sends `keyId` and `createTime`. So these
//!   structs carry no `rename_all` and the field names are the wire names.
//! - **The list envelope is not.** `secrets` and `nextPageCursor` in the same
//!   document as the `snake_case` items, and a *cursor* rather than the
//!   `pageToken`/`nextPageToken` pair used everywhere else.
//!
//! Neither is a mistake in the transcription. Both are what the vendored
//! `spec/openapi.json` declares, and `rbx-spec-drift` is what notices if that
//! stops being true.

use serde::{Deserialize, Serialize};

/// One secret, in both directions.
///
/// Roblox uses this same shape for four different things (a listing row, a
/// create body, an update body, and the public key) which is why nearly every
/// field is optional. What each call actually fills in:
///
/// | call | `id` | `secret` | `key_id` | `domain` | timestamps |
/// |------|------|----------|----------|----------|------------|
/// | list | yes | never | yes | yes | yes |
/// | public-key | `"public-key"` | the key | yes | no | no |
/// | create | required | required | required | optional | ignored |
/// | update | ignored (path) | required | required | optional | ignored |
///
/// The one guarantee that matters for a CLI: **`secret` never comes back with
/// a stored value in it.** A listing carries metadata only, so there is no
/// `rbx secret get`, and nothing here can print a secret it once wrote.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Secret {
    /// The name the game passes to `HttpService:GetSecret`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Base64. Sealed ciphertext on the way in, the universe's public key on
    /// the way out of `public-key`, and absent everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// Which key the content was sealed under. Roblox rejects a write whose
    /// `key_id` it does not recognise, which is what makes a rotated public
    /// key a clean failure rather than a secret nothing can decrypt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    /// The domain wildcard the secret may be sent to, e.g. `api.example.com`
    /// or `*`. Absent or empty means the value can never leave the server:
    /// usable for signing, never as a header or a URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
}

impl Secret {
    /// The domain as the human and JSON forms report it.
    ///
    /// Roblox returns absent and empty for the same state (a secret no
    /// request may carry) so both normalise to `None` here rather than
    /// leaving each call site to remember that `Some("")` is not a domain.
    pub fn effective_domain(&self) -> Option<&str> {
        self.domain.as_deref().filter(|d| !d.is_empty())
    }
}

/// One page of secrets.
///
/// `previousPageCursor` is deserialized and then ignored: this walks forward
/// only, and a field parsed but unused is cheaper than one that turns up in a
/// body and has nowhere to go.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SecretList {
    #[serde(default)]
    pub secrets: Vec<Secret>,
    #[serde(default, rename = "nextPageCursor")]
    pub next_page_cursor: Option<String>,
    #[serde(default, rename = "previousPageCursor")]
    pub previous_page_cursor: Option<String>,
}

impl SecretList {
    /// Empty string and `null` both mean "no more pages". Roblox has been seen
    /// sending either, and a `?cursor=` with an empty value is a request that
    /// pages forever.
    pub fn next_page(&self) -> Option<&str> {
        self.next_page_cursor.as_deref().filter(|c| !c.is_empty())
    }
}

/// Reject a name Roblox would reject, before spending a request on it.
///
/// The rule is in the specification: "alphanumeric or underscore, 1-64
/// characters, not starting with a number", and the failure without this
/// check is a `400` whose body does not say which of the four constraints was
/// broken. It is also the shape `HttpService:GetSecret("name")` takes in Luau,
/// so a name that fails here would never have been readable from the game.
pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.chars().count() > 64 {
        return Err(format!(
            "a secret name is 1 to 64 characters; \"{id}\" is {}",
            id.chars().count()
        ));
    }
    if id.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!("a secret name cannot start with a digit: \"{id}\""));
    }
    if let Some(bad) = id.chars().find(|c| !c.is_ascii_alphanumeric() && *c != '_') {
        return Err(format!(
            "a secret name is ASCII letters, digits and underscores only; \"{id}\" contains {bad:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_names_are_snake_case_unlike_the_rest_of_cloud_v2() {
        let parsed: Secret = serde_json::from_str(
            r#"{"id":"discord","key_id":"k1","domain":"discord.com",
                "create_time":"2026-08-01T10:00:00Z","update_time":"2026-08-02T10:00:00Z"}"#,
        )
        .expect("fixture");

        assert_eq!(parsed.id.as_deref(), Some("discord"));
        assert_eq!(parsed.key_id.as_deref(), Some("k1"));
        assert_eq!(parsed.create_time.as_deref(), Some("2026-08-01T10:00:00Z"));
        // And a listing never carries content, which is why there is no `get`.
        assert_eq!(parsed.secret, None);
    }

    /// A write body must not carry keys Roblox did not ask for: `create_time`
    /// on a `POST` is at best ignored and at worst a `400`.
    #[test]
    fn a_write_body_omits_every_field_it_has_nothing_to_say_about() {
        let body = Secret {
            id: Some("discord".into()),
            secret: Some("c2VhbGVk".into()),
            key_id: Some("k1".into()),
            ..Secret::default()
        };

        let rendered = serde_json::to_string(&body).expect("serialize");
        assert_eq!(
            rendered,
            r#"{"id":"discord","secret":"c2VhbGVk","key_id":"k1"}"#
        );
    }

    #[test]
    fn an_empty_domain_is_no_domain() {
        let private = Secret {
            domain: Some(String::new()),
            ..Secret::default()
        };
        assert_eq!(private.effective_domain(), None);
        assert_eq!(Secret::default().effective_domain(), None);

        let public = Secret {
            domain: Some("*".into()),
            ..Secret::default()
        };
        assert_eq!(public.effective_domain(), Some("*"));
    }

    #[test]
    fn an_empty_cursor_ends_the_walk_the_same_as_a_missing_one() {
        let list: SecretList =
            serde_json::from_str(r#"{"secrets":[],"nextPageCursor":""}"#).expect("fixture");
        assert_eq!(list.next_page(), None);

        let more: SecretList =
            serde_json::from_str(r#"{"secrets":[],"nextPageCursor":"abc"}"#).expect("fixture");
        assert_eq!(more.next_page(), Some("abc"));
    }

    #[test]
    fn a_name_the_api_would_refuse_is_refused_here_first() {
        assert!(validate_id("discord").is_ok());
        assert!(validate_id("_internal").is_ok());
        assert!(validate_id("aws_access_key_2").is_ok());

        assert!(validate_id("").is_err());
        assert!(validate_id(&"a".repeat(65)).is_err());
        assert!(validate_id("2fa_token").is_err());
        // The two that bite: a hyphen reads as legal, and so does a dot.
        assert!(validate_id("api-key").is_err());
        assert!(validate_id("api.key").is_err());
    }
}
