pub mod download;
pub mod fetch;
pub mod places;
pub mod promote;
pub mod rollback;
pub mod upload;
pub mod versions;

use anyhow::Result;

use crate::api::RbxClient;
use rbx_core::output::OutputFormat;
use rbx_core::GlobalFlags;

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
