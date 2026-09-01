pub mod download;
pub mod fetch;
pub mod places;
pub mod promote;
pub mod rollback;
pub mod upload;
pub mod versions;

use anyhow::Result;
use bytes::Bytes;
use colored::Colorize;

use crate::api::RbxClient;
use rbx_core::output::OutputFormat;
use rbx_core::GlobalFlags;

/// What an upload did to the place, beyond the version number it answered with.
///
/// The upload endpoint returns a version number and nothing else, and it
/// returns the number the place is **already** at when the bytes match what it
/// already holds: Roblox creates no version for content it has. Measured
/// against a test place on 2026-09-01, three uploads one after another. The
/// same file twice answered the same number both times and left the version
/// list untouched; one byte changed, in the reserved bytes of the binary header
/// that the format ignores, produced a new version two minutes later. So the
/// deduplication is on the bytes sent, there is no cooldown involved, and the
/// number alone cannot tell a write from a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// Roblox made a new version out of these bytes.
    Created,
    /// The place already held these bytes. The version is real and is the one
    /// the place was already at; this run added nothing.
    Unchanged,
    /// The place's previous version could not be read, so the two above cannot
    /// be told apart. Reported the way it was before any of this existed.
    Unknown,
}

impl Landing {
    /// The parenthetical the human listing puts after the version number, if
    /// any. `Unknown` gets none: a run that could not check says nothing rather
    /// than hedging in the one place a reader is looking for a result.
    pub fn note(self) -> Option<String> {
        match self {
            Self::Created => None,
            Self::Unchanged => Some(" (unchanged)".yellow().to_string()),
            Self::Unknown => None,
        }
    }

    /// For the receipt: `None` where there is no answer rather than a false one.
    pub fn created(self) -> Option<bool> {
        match self {
            Self::Created => Some(true),
            Self::Unchanged => Some(false),
            Self::Unknown => None,
        }
    }
}

/// Upload bytes to a place, and say whether that actually created a version.
///
/// Shared by `upload` and `promote` because both send a place file through the
/// same endpoint and inherit the same ambiguity from it. See [`Landing`].
///
/// The extra read is best effort, and deliberately so: a key holding
/// `universe-places:write` and no read scope can upload and cannot list
/// versions, so a failure here means the answer is unknown, never that the
/// upload should not happen. A diagnostic must not be the reason a write fails.
pub async fn upload_and_classify(
    client: &RbxClient,
    universe_id: u64,
    place_id: u64,
    data: Bytes,
    published: bool,
) -> Result<(u64, Landing)> {
    // Before the write, so the comparison is against what the place held when
    // this run started rather than against what it holds afterwards.
    //
    // An empty list and a refused read are not the same answer and are kept
    // apart here: a place with no versions at all is one where anything that
    // lands is new, which is a fact, while a read that failed is the absence of
    // one.
    let before = client
        .list_versions(place_id, 1)
        .await
        .map(|versions| versions.first().map(|latest| latest.version_number))
        .ok();

    let version = client
        .upload_place(universe_id, place_id, data, published)
        .await?;

    Ok((version, classify(before, version)))
}

/// The rule [`upload_and_classify`] applies, separated from the two calls
/// around it so it can be checked without a server.
///
/// `before` is what the place's newest version was: the outer `None` is a read
/// that did not answer, the inner one a place that had no version at all.
fn classify(before: Option<Option<u64>>, version: u64) -> Landing {
    match before {
        None => Landing::Unknown,
        Some(None) => Landing::Created,
        // A number that did not move past what was already there is the number
        // of a version this run did not make.
        Some(Some(previous)) => {
            if version > previous {
                Landing::Created
            } else {
                Landing::Unchanged
            }
        }
    }
}

pub fn make_client(global: &GlobalFlags, base_url: Option<&str>) -> Result<RbxClient> {
    let key = global.api_key.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "--api-key or RBX_API_KEY env var is required for this operation.\n\
             Create a key at: https://create.roblox.com/dashboard/credentials"
        )
    })?;
    Ok(with_base(RbxClient::new(key), base_url))
}

/// Apply the hidden `--base-url` override, if one was given.
///
/// Separate from `make_client` because `place places` builds a keyless client:
/// the universe listing goes to the `develop` host, which takes no API key.
pub fn with_base(client: RbxClient, base_url: Option<&str>) -> RbxClient {
    match base_url {
        Some(url) => client.with_base_url(url),
        None => client,
    }
}

/// The error for a question this invocation is not allowed to ask.
///
/// Every write here has a point where it would stop and ask: a confirmation on
/// an env with `confirm = true`, or which version to roll back to. Under
/// `--json` stdout carries the document, so a prompt would corrupt it; with no
/// terminal there is nobody to answer one. `OutputFormat::may_prompt` decides
/// both at once, and this turns its refusal into a message naming the flag that
/// answers the question up front.
///
/// The two texts differ because the causes do: told "there is no terminal" when
/// there plainly is one, on a machine where `--json` was the only problem, is a
/// message that sends people to the wrong fix.
pub fn cannot_ask(format: OutputFormat, question: &str, flag: &str) -> anyhow::Error {
    if format.is_json() {
        anyhow::anyhow!("--json cannot ask {question}: stdout carries the document. Pass {flag}.")
    } else {
        anyhow::anyhow!("There is no terminal to ask {question} on. Pass {flag}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measurement in #4: the same file uploaded twice answered `v3` both
    /// times, and the second one created nothing.
    #[test]
    fn a_version_number_that_did_not_move_is_a_no_op() {
        assert_eq!(classify(Some(Some(3)), 3), Landing::Unchanged);
        assert_eq!(classify(Some(Some(3)), 4), Landing::Created);
    }

    /// A place with no version yet is one where anything that lands is new.
    /// That is a fact about the place, not an absence of one, which is why it
    /// does not share an answer with the case below.
    #[test]
    fn a_place_with_no_versions_gets_a_new_one() {
        assert_eq!(classify(Some(None), 1), Landing::Created);
    }

    /// A key that may upload and may not list versions is the reason this is
    /// three-valued. The run says nothing rather than guessing, and in
    /// particular does not report a write as unchanged.
    #[test]
    fn a_read_that_did_not_answer_leaves_the_question_open() {
        assert_eq!(classify(None, 4), Landing::Unknown);
        assert_eq!(Landing::Unknown.created(), None);
        assert!(Landing::Unknown.note().is_none());
    }

    /// The receipt's side of the same rule.
    #[test]
    fn only_an_unchanged_landing_reports_false() {
        assert_eq!(Landing::Created.created(), Some(true));
        assert_eq!(Landing::Unchanged.created(), Some(false));
    }
}
