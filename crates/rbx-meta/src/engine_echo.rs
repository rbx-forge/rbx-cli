//! The one place Roblox says anything about the *inside* of
//! `engineAvatarSettings`.
//!
//! # Why this exists
//!
//! `rbx meta` sends that document through without modelling it, and
//! `schemas/rbxavatar.schema.json` is guidance rather than validation: both
//! deliberate, both documented where they are decided. The consequence is a
//! real blind spot: a key misspelled in the file is a key Roblox ignores, and
//! nothing anywhere reports it. The avatar setting simply does not take, and
//! the developer goes looking in Studio.
//!
//! Except that Roblox does answer. `PATCH /v2/universes/{id}/configuration`
//! responds with `UniverseSettingsResponseV2`, and that carries
//! `engineAvatarSettings`: the document as Roblox understood it. Every other
//! endpoint treats the field as an opaque string; this response is the only
//! request in the whole API whose answer looks inside.
//!
//! So the echo is compared against what was sent, and the differences are
//! reported. It costs nothing: the response was already coming back and was
//! being thrown away.
//!
//! # What a difference means
//!
//! **A key sent and not echoed was dropped.** That is the one worth acting on:
//! a typo, a key from a Roblox version that no longer exists, or a key this
//! project invented. The setting did not apply.
//!
//! **A key echoed and not sent was filled in by Roblox.** Normal: a partial
//! document is a normal thing to write, and Roblox completes it. Reported at a
//! lower volume because it is how somebody discovers what the full document
//! looks like without guessing.
//!
//! # Why it warns rather than fails
//!
//! By the time there is an echo to read, the write has already landed. Failing
//! afterwards would report an error for something that succeeded, and would
//! leave the lockfile disagreeing with Roblox over a spelling. Roblox is also
//! entitled to normalise a document it accepted, and treating every
//! normalisation as an error would train everyone to ignore the output: the
//! same reasoning that keeps the schema's `additionalProperties` open.

use std::collections::BTreeSet;

use serde_json::Value;

/// What the echo said about the document that was sent.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Echo {
    /// Keys sent that Roblox did not send back: they did not apply.
    pub dropped: Vec<String>,
    /// Keys Roblox added that were not sent: defaults it filled in.
    pub added: Vec<String>,
}

impl Echo {
    pub fn is_clean(&self) -> bool {
        self.dropped.is_empty() && self.added.is_empty()
    }
}

/// Every leaf path in a JSON document, dotted.
///
/// Arrays are leaves rather than being walked into. The documents here use
/// them for fixed-length vectors (`SingleColliderSize`, `LimitBounds`,
/// `CustomHeight`) where an index is a coordinate and not a key anybody
/// misspells. Walking them would turn one changed number into three reported
/// paths.
fn leaf_paths(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaf_paths(child, &path, out);
            }
        }
        // An empty object is itself a leaf: `{}` is a meaningful document here,
        // and losing it would make "cleared the settings" look like "sent
        // nothing".
        _ => {
            if !prefix.is_empty() {
                out.insert(prefix.to_string());
            }
        }
    }
}

/// Compare the document that was sent against the one Roblox echoed.
///
/// `None` when there is nothing to compare: an empty body, a response that is
/// not JSON, or one carrying no `engineAvatarSettings`. All three are ordinary
/// (a mock server in a test, an endpoint that changed its response shape, a
/// patch that touched something else) and none of them are failures of the
/// write that already happened.
pub fn compare(sent: &str, response_body: &str) -> Option<Echo> {
    let response: Value = serde_json::from_str(response_body).ok()?;
    let echoed = response.get("engineAvatarSettings")?.as_str()?;

    let sent: Value = serde_json::from_str(sent).ok()?;
    let echoed: Value = serde_json::from_str(echoed).ok()?;

    let mut sent_paths = BTreeSet::new();
    leaf_paths(&sent, "", &mut sent_paths);
    let mut echoed_paths = BTreeSet::new();
    leaf_paths(&echoed, "", &mut echoed_paths);

    Some(Echo {
        dropped: sent_paths.difference(&echoed_paths).cloned().collect(),
        added: echoed_paths.difference(&sent_paths).cloned().collect(),
    })
}

#[cfg(test)]
mod tests;
