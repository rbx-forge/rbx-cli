//! Reading the shape `introspect` answers with.
//!
//! Roblox does not echo back what it was sent. A scope goes out as
//! `{scopeType, targetParts, operations}` and comes back from
//! `/cloud-authentication/v1/apiKey/introspect` as
//! `{name, operations, universeIds}`: a different name for the type, a
//! different name for the target, and targets typed as universe ids rather
//! than the `U<id>` / `G<id>` / `*` strings a creator-targeted scope is sent
//! with.
//!
//! ## How this was found
//!
//! Not by a test. Every test in this crate feeds the parser something this
//! crate wrote, so a parser that only understands what we send passes all of
//! them. It was found by creating a key against real Open Cloud and reading
//! the answer, on 2026-08-16.
//!
//! What made it invisible for longer is that both readers failed *quietly*.
//! `diagnostics::introspect_scopes` collected with `filter_map(... .ok())`,
//! so entries that did not deserialise were dropped one by one until nothing
//! was left, and an empty scope list reads as "this key grants nothing"
//! rather than as "this was not understood". `create`'s post-create check
//! counted the raw JSON array instead of the parsed one, so it compared 5
//! against 5 and reported a match it had not verified.

use serde::Deserialize;

use crate::scope_builder::ScopeDef;

/// One scope as `introspect` returns it.
///
/// Deliberately its own type rather than serde aliases on [`ScopeDef`]: that
/// type is the **request** shape, it is serialised onto the wire, and giving
/// it aliases would let a future edit send `name` where Roblox expects
/// `scopeType`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IntrospectScope {
    name: String,
    #[serde(default)]
    operations: Vec<String>,
    /// Present on universe-targeted scopes. Absent on the ones targeting a
    /// creator or nothing at all, which is why it defaults rather than
    /// failing the whole entry.
    #[serde(default)]
    universe_ids: Vec<String>,
}

/// Parse the `scopes` array of an introspect response into the request shape,
/// so a caller can compare what came back against what it sent.
///
/// Returns `None` when the document has no `scopes` array at all: the one
/// case that means "this is not an introspect response" rather than "this key
/// has no scopes". An entry that cannot be read is an error rather than a
/// silent drop: a key reported as granting nothing, because its scopes were
/// unreadable, is the failure this module exists to have stopped.
pub fn scopes_from_response(resp: &serde_json::Value) -> Option<Result<Vec<ScopeDef>, String>> {
    let arr = resp.get("scopes")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        match serde_json::from_value::<IntrospectScope>(entry.clone()) {
            Ok(s) => out.push(ScopeDef {
                scope_type: s.name,
                // `*` is what a scope with no universe targets is sent as, and
                // it is what keeps the comparison in `create` honest for the
                // badge and creator scopes.
                target_parts: if s.universe_ids.is_empty() {
                    vec!["*".to_string()]
                } else {
                    s.universe_ids
                },
                operations: s.operations,
            }),
            Err(e) => return Some(Err(format!("unreadable scope entry: {e}"))),
        }
    }
    Some(Ok(out))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    /// The shape Roblox actually returned on 2026-08-16, copied from a live
    /// response. This is the fixture the parser exists for, and the one no
    /// test had before, because every other fixture in this crate was written
    /// by this crate.
    fn live_response() -> serde_json::Value {
        json!({
            "authorizedUserId": 1234567890,
            "enabled": true,
            "name": "opsdev_shoptest",
            "scopes": [
                {
                    "name": "developer-product",
                    "operations": ["read", "write"],
                    "universeIds": ["99887766554"]
                },
                {
                    "name": "legacy-universe.badge",
                    "operations": ["write", "manage-and-spend-robux"]
                }
            ]
        })
    }

    #[test]
    fn the_live_shape_parses_into_the_request_shape() {
        let scopes = scopes_from_response(&live_response()).unwrap().unwrap();
        assert_eq!(scopes.len(), 2);

        assert_eq!(scopes[0].scope_type, "developer-product");
        assert_eq!(scopes[0].target_parts, vec!["99887766554"]);
        assert_eq!(scopes[0].operations, vec!["read", "write"]);
    }

    /// A scope with no universe list is not a scope with no target: it is the
    /// `*` a badge or creator scope is sent with, and comparing it as an empty
    /// list would report every one of them as missing.
    #[test]
    fn a_scope_without_universes_targets_everything() {
        let scopes = scopes_from_response(&live_response()).unwrap().unwrap();
        assert_eq!(scopes[1].scope_type, "legacy-universe.badge");
        assert_eq!(scopes[1].target_parts, vec!["*"]);
    }

    /// The request shape must **not** parse as the response shape. If it did,
    /// this parser would accept what we send and the mismatch would be
    /// invisible again.
    #[test]
    fn the_request_shape_is_not_silently_accepted() {
        let sent = json!({
            "scopes": [{
                "scopeType": "universe",
                "targetParts": ["1"],
                "operations": ["read"]
            }]
        });
        assert!(
            scopes_from_response(&sent).unwrap().is_err(),
            "a document in the request shape must be reported, not read as empty"
        );
    }

    /// No `scopes` key at all means this is not an introspect response, which
    /// is a different thing from a key that grants nothing.
    #[test]
    fn a_document_without_scopes_is_not_an_empty_key() {
        assert!(scopes_from_response(&json!({"enabled": true})).is_none());
    }
}
