pub mod check;
pub mod get;
pub mod init;
pub mod list;
pub mod pull;
pub mod rollback;
pub mod sync;
pub mod versions;

use anyhow::Result;

use crate::api::RbxConfigClient;
use crate::ctx::ConfigCtx;

pub fn make_client(ctx: &ConfigCtx) -> Result<RbxConfigClient> {
    let key = ctx.api_key.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "--api-key or RBX_API_KEY env var is required.\n\
             Create a key at: https://create.roblox.com/dashboard/credentials"
        )
    })?;
    let client = RbxConfigClient::new(key);
    #[cfg(test)]
    if let Some(url) = &ctx.base_url {
        return Ok(client.with_base_url(url.clone()));
    }
    Ok(client)
}

/// Resolve the publish message: an explicit flag, `--no-message`, `--yes`, or
/// a prompt.
///
/// `--yes` answers this one too. It reads as "do not ask me anything", and a
/// flag that skipped the confirmation and then stopped on a second question
/// was the difference between a pipeline that runs and one that fails on
/// `not a terminal`: a message naming the wrong thing entirely, since the
/// terminal was never the problem.
///
/// Off a terminal without any of the three, the refusal names the flags rather
/// than the stream. `dialoguer` fails there with an IO error about a terminal,
/// which tells somebody reading CI logs nothing they can act on.
pub fn resolve_message(message: Option<&str>, no_message: bool, yes: bool) -> Result<String> {
    if no_message || yes {
        return Ok(String::new());
    }
    if let Some(m) = message {
        return Ok(m.to_string());
    }
    if !rbx_core::output::is_interactive() {
        anyhow::bail!(
            "a publish message is needed and there is nobody to ask. On `config sync`, pass --message \"...\", or --no-message to publish without one, or --yes, which answers this as well as the confirmation. `config rollback` composes its own message and needs a terminal."
        );
    }
    let msg = dialoguer::Input::<String>::new()
        .with_prompt("Publish message (Enter to skip)")
        .allow_empty(true)
        .interact_text()
        .map_err(|e| anyhow::anyhow!("Prompt error: {}", e))?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this exists for: `config sync --yes` skipped the confirmation
    /// and then stopped on a second question, failing CI with "not a terminal":
    /// a message about the stream, when the stream was never the problem.
    #[test]
    fn yes_answers_the_message_prompt_too() {
        assert_eq!(resolve_message(None, false, true).unwrap(), "");
    }

    #[test]
    fn no_message_still_wins_and_an_explicit_message_survives() {
        assert_eq!(
            resolve_message(Some("ship it"), false, false).unwrap(),
            "ship it"
        );
        assert_eq!(resolve_message(Some("ship it"), true, false).unwrap(), "");
    }

    /// A test binary's streams are not terminals, which is the branch CI takes,
    /// so this asserts the refusal names flags rather than the stream.
    #[test]
    fn with_nobody_to_ask_the_refusal_names_the_flags() {
        let err = resolve_message(None, false, false).unwrap_err().to_string();
        assert!(err.contains("--message"), "got: {err}");
        assert!(err.contains("--no-message"), "got: {err}");
        assert!(err.contains("--yes"), "got: {err}");
        assert!(!err.contains("not a terminal"), "got: {err}");
    }
}
