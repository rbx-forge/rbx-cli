//! `rbx secret`: the universe secrets store `HttpService:GetSecret` reads from.
//!
//! A secret is a value the game needs and the repository must never contain —
//! a Discord webhook, a payment provider's key, the token for whatever service
//! a `DataStore` is being reconciled against. Roblox keeps it per universe,
//! encrypted, and hands it to the running game as a `Secret` userdata that Luau
//! can send in a request but cannot read, print or concatenate.
//!
//! Before this command existed the only way in was the Creator Dashboard, by
//! hand, per universe. That is exactly the shape of task that goes wrong
//! quietly: a staging universe silently keeps last quarter's key because
//! somebody updated production and stopped there.
//!
//! ## Writes are encrypted here, not in transit
//!
//! Roblox does not accept a secret in the clear even over TLS. The content is
//! sealed against the universe's own public key before it is sent, so what
//! crosses the wire — and what lands in any proxy log along the way — is
//! ciphertext only this universe can open. [`seal`] carries the details and
//! the reasoning; the consequence for this module is that every write is two
//! requests, because the public key has to be fetched first.
//!
//! ## Why there is no `rbx secret get`
//!
//! Not an omission, and not a scope this key is missing: Roblox never sends
//! stored content back. `list` returns names, domains, key ids and timestamps
//! — everything except the one field you might want — and that is the design
//! working. A secrets store you can read from is a secrets store one leaked
//! API key drains.
//!
//! So the model is write-only. To find out whether a value is right, replace
//! it; there is nothing to compare against, which is also why `set` cannot
//! report "no change" the way `rbx config sync` does.
//!
//! ## Why `set` restates the domain every time
//!
//! A secret's `domain` decides which hosts `HttpService` will attach it to,
//! and a secret with no domain cannot leave the server at all — it can be used
//! for signing in-process and nothing else. That is a useful state and a
//! terrible accident: the failure shows up in the game, at runtime, as a
//! request going out without its credential.
//!
//! `set` therefore demands `--domain <pattern>` or `--no-domain` on every
//! write, rather than defaulting to either. The alternative would be to carry
//! the stored domain forward on an update, which means reading it first — a
//! second scope, on a command otherwise usable with a write-only key — and
//! guessing at what Roblox does with an omitted field on a `PATCH`.
//!
//! ## Writes need `--apply`
//!
//! The same rule as `rbx data` and `rbx memorystore`, for a stronger reason
//! than either. Overwriting a secret destroys the previous value with nothing
//! anywhere that can recover it: not a backup file, not a version history, not
//! the API. If the old key is gone from the password manager too, it is gone.

pub mod json;
pub mod model;
pub mod seal;

use std::io::Read;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use reqwest::{Client, StatusCode};

use rbx_core::api::{
    build_client, encode_query_value, execute_json, execute_with_retry, explain_missing_scope,
    is_api_status, require_api_key, ApiBase,
};
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use crate::json::{DeleteDocument, ListDocument, PublicKeyDocument, SetDocument};
use crate::model::{validate_id, Secret, SecretList};
use crate::seal::UniverseKey;

/// Roblox caps a page at 500 secrets, and caps a universe at 500 secrets, so
/// one page is always enough in practice. The walk is written anyway: a cap
/// that holds today is not a cap worth hard-coding a correctness assumption
/// on, and paging costs nothing when there is only ever one page.
const MAX_PAGE_SIZE: u32 = 500;

#[derive(Args, Debug)]
pub struct SecretCli {
    #[command(subcommand)]
    command: Command,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

impl SecretCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List the secrets defined for the universe
    ///
    /// Names, domains and timestamps. Never values: Roblox does not send
    /// stored content back to anybody, which is the point of the store.
    List {
        /// Maximum secrets to fetch, across as many pages as it takes.
        #[arg(long, default_value_t = 500)]
        limit: u32,

        /// Write the listing to stdout as one JSON document.
        ///
        /// Field names are documented in docs/secret.md. No document this
        /// command emits has a field for secret content.
        #[arg(long)]
        json: bool,
    },

    /// Store a secret, creating it or replacing what is there
    ///
    /// The value is sealed against the universe's public key before it leaves
    /// this process, so it is never sent, logged or proxied in the clear.
    ///
    /// Replacing is destructive and irreversible: the previous value is not
    /// recoverable from Roblox, from this tool, or from anywhere else.
    Set {
        /// Secret name, as `HttpService:GetSecret` will ask for it.
        ///
        /// ASCII letters, digits and underscores, 1-64 characters, not
        /// starting with a digit.
        id: String,

        /// The value, inline.
        ///
        /// Convenient and the least private of the three: it travels as a
        /// command-line argument, so it is readable in the process list and
        /// recorded by command-line auditing — Windows event 4688, Sysmon, an
        /// EDR agent, auditd. Those logs persist and often leave the machine.
        /// Prefer --stdin outside a scratch terminal.
        #[arg(long, conflicts_with_all = ["file", "stdin"])]
        value: Option<String>,

        /// Read the value from a file, byte for byte.
        ///
        /// Nothing is trimmed: a PEM private key keeps its trailing newline,
        /// because a file's contents are taken to be deliberate.
        #[arg(long, conflicts_with = "stdin")]
        file: Option<PathBuf>,

        /// Read the value from standard input.
        ///
        /// One trailing newline is stripped, because a pipe adds one and a
        /// credential with `\n` glued to the end fails authentication in a way
        /// that takes an afternoon to find. Use --file to keep every byte.
        #[arg(long)]
        stdin: bool,

        /// Hosts HttpService may send this secret to, e.g. `api.example.com`,
        /// `*.example.com`, or `*` for anywhere.
        ///
        /// Required on every write, including an update: a `set` replaces the
        /// whole secret, so an unstated domain would be a silently cleared
        /// one.
        #[arg(long, conflicts_with = "no_domain")]
        domain: Option<String>,

        /// Store the value with no domain at all.
        ///
        /// The secret can then be used for signing inside the server and can
        /// never be attached to an outgoing request. Correct for a private
        /// key; a trap for an API token, which is why it has to be said.
        #[arg(long)]
        no_domain: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// Only under --apply: a dry run has no result to report. The document
        /// names the secret and its size, never its content.
        #[arg(long)]
        json: bool,
    },

    /// Delete a secret
    ///
    /// Irreversible, and the game stops being able to read it immediately.
    Delete {
        /// Secret name.
        id: String,

        /// Actually delete it.
        #[arg(long)]
        apply: bool,

        /// Write the result to stdout as one JSON document.
        #[arg(long)]
        json: bool,
    },

    /// Print the universe's public key, for sealing a secret elsewhere
    ///
    /// `set` fetches this on its own; this subcommand is for the case where
    /// the encryption has to happen somewhere else — a deployment system that
    /// holds the plaintext and will not hand it to a CLI, or a language
    /// binding doing the sealed box itself.
    ///
    /// The key is public. Publishing it is what it is for.
    PublicKey {
        /// Write the key to stdout as one JSON document.
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(cli: SecretCli, global: &GlobalFlags) -> Result<()> {
    let base = match &cli.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let api = Api {
        client: build_client(),
        base,
        api_key: require_api_key(global.api_key.as_deref())?.to_string(),
        universe_id: global.single_universe()?,
    };

    match cli.command {
        Command::List { limit, json } => {
            list(&api, limit, OutputFormat::from_json_flag(json)).await
        }

        Command::Set {
            id,
            value,
            file,
            stdin,
            domain,
            no_domain,
            apply,
            json,
        } => {
            let source = ValueSource::pick(value, file, stdin)?;
            let domain = Domain::pick(domain, no_domain)?;
            set(
                &api,
                &id,
                source,
                domain,
                apply,
                OutputFormat::from_json_flag(json),
            )
            .await
        }

        Command::Delete { id, apply, json } => {
            delete(&api, &id, apply, OutputFormat::from_json_flag(json)).await
        }

        Command::PublicKey { json } => public_key(&api, OutputFormat::from_json_flag(json)).await,
    }
}

async fn list(api: &Api, limit: u32, format: OutputFormat) -> Result<()> {
    let secrets = api.list(limit).await?;

    if secrets.is_empty() {
        format.note(
            format!("universe {} has no secrets", api.universe_id)
                .dimmed()
                .to_string(),
        );
    }
    if format.is_json() {
        return output::emit(&ListDocument::new(api.universe_id, limit, &secrets));
    }
    if secrets.is_empty() {
        return Ok(());
    }

    for secret in &secrets {
        let id = secret.id.as_deref().unwrap_or("<no id>");
        // Spelled out rather than left blank: a blank column reads as "we did
        // not look", and this is the field that decides whether the game can
        // use the secret in a request at all.
        let domain = match secret.effective_domain() {
            Some(domain) => format!("  {domain}"),
            None => "  (no domain: server-side use only)".to_string(),
        };
        let updated = secret
            .update_time
            .as_deref()
            .map(|time| format!("  updated {time}"))
            .unwrap_or_default();
        println!("{id}{}{}", domain.dimmed(), updated.dimmed());
    }
    println!("{}", format!("{} secret(s)", secrets.len()).dimmed());
    Ok(())
}

async fn set(
    api: &Api,
    id: &str,
    source: ValueSource,
    domain: Domain,
    apply: bool,
    format: OutputFormat,
) -> Result<()> {
    validate_id(id).map_err(|why| anyhow::anyhow!("{why}"))?;

    // Before reading the value, not after: a dry run that consumed stdin would
    // leave nothing for the `--apply` run a user pipes into next.
    if !apply {
        println!(
            "would store secret \"{id}\" in universe {} ({})",
            api.universe_id,
            domain.describe()
        );
        println!("{}", format!("  value from {}", source.describe()).dimmed());
        println!("{}", "Nothing sent. Re-run with --apply.".dimmed());
        return Ok(());
    }

    let plaintext = source.read()?;
    let key = api.public_key().await?;
    let sealed = key.seal(&plaintext)?;

    let body = Secret {
        secret: Some(sealed.content),
        key_id: Some(sealed.key_id.clone()),
        domain: Some(domain.as_wire().to_string()),
        ..Secret::default()
    };

    let action = api.upsert(id, &body).await?;

    if format.is_json() {
        return output::emit(&SetDocument {
            schema_version: output::SCHEMA_VERSION,
            universe_id: api.universe_id,
            id: id.to_string(),
            action: action.as_str(),
            bytes: plaintext.len(),
            domain: domain.as_option().map(str::to_string),
            key_id: sealed.key_id,
        });
    }

    println!(
        "{}",
        format!("✓ {} secret \"{id}\"", action.as_str()).green()
    );
    println!(
        "{}",
        format!("  {} bytes, {}", plaintext.len(), domain.describe()).dimmed()
    );
    println!(
        "{}",
        format!("  read it in Luau with HttpService:GetSecret(\"{id}\")").dimmed()
    );
    Ok(())
}

async fn delete(api: &Api, id: &str, apply: bool, format: OutputFormat) -> Result<()> {
    validate_id(id).map_err(|why| anyhow::anyhow!("{why}"))?;

    if !apply {
        println!(
            "would delete secret \"{id}\" from universe {}",
            api.universe_id
        );
        println!(
            "{}",
            "The value is not recoverable afterwards. Nothing sent. Re-run with --apply.".dimmed()
        );
        return Ok(());
    }

    api.delete(id).await?;

    if format.is_json() {
        return output::emit(&DeleteDocument {
            schema_version: output::SCHEMA_VERSION,
            universe_id: api.universe_id,
            id: id.to_string(),
        });
    }
    println!("{}", format!("✓ deleted secret \"{id}\"").green());
    Ok(())
}

async fn public_key(api: &Api, format: OutputFormat) -> Result<()> {
    let key = api.public_key().await?;

    if format.is_json() {
        return output::emit(&PublicKeyDocument {
            schema_version: output::SCHEMA_VERSION,
            universe_id: api.universe_id,
            public_key: key.encoded().to_string(),
            key_id: key.key_id().to_string(),
        });
    }
    println!("{}", key.encoded());
    eprintln!("{}", format!("key_id {}", key.key_id()).dimmed());
    Ok(())
}

/// Which of the three ways of naming a value was used.
#[derive(Debug)]
enum ValueSource {
    Inline(String),
    File(PathBuf),
    Stdin,
}

impl ValueSource {
    fn pick(value: Option<String>, file: Option<PathBuf>, stdin: bool) -> Result<Self> {
        match (value, file, stdin) {
            (Some(inline), _, _) => Ok(Self::Inline(inline)),
            (None, Some(path), _) => Ok(Self::File(path)),
            (None, None, true) => Ok(Self::Stdin),
            (None, None, false) => bail!(
                "`set` needs a value: --value <text>, --file <path>, or --stdin. \
                 --stdin is the one that keeps the value out of your shell history."
            ),
        }
    }

    /// Where the value comes from, for a dry run that has not read it yet.
    fn describe(&self) -> String {
        match self {
            Self::Inline(_) => "--value".to_string(),
            Self::File(path) => format!("{}", path.display()),
            Self::Stdin => "standard input".to_string(),
        }
    }

    fn read(&self) -> Result<Vec<u8>> {
        match self {
            Self::Inline(inline) => {
                // Said once, on stderr, in both output formats. It names the
                // command line rather than the shell history on purpose: the
                // history claim is the one people check and find false.
                // `--value $(pass show token)` stores the substitution in
                // history, not its result — true of PSReadLine, bash and zsh
                // alike — so leading with history invites the reader to
                // dismiss the whole warning. The command line is where the
                // expanded value really does land, in every shell, and it is
                // the worse exposure of the two anyway.
                eprintln!(
                    "{}",
                    "note: --value travels as a command-line argument, which the process list \
                     and any command-line auditing record. --stdin does not."
                        .dimmed()
                );
                Ok(inline.clone().into_bytes())
            }
            Self::File(path) => {
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))
            }
            Self::Stdin => {
                let mut buffer = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut buffer)
                    .context("reading the value from standard input")?;
                Ok(strip_one_trailing_newline(buffer))
            }
        }
    }
}

/// Drop a single trailing `\n`, and the `\r` before it if there is one.
///
/// `echo "$KEY" | rbx secret set ...` is the intended usage and `echo` adds a
/// newline; storing it produces an `Authorization` header with a line break in
/// it, which fails in a way that reads as a wrong key rather than a wrong
/// byte. Exactly one is removed, so a value that genuinely ends in two
/// newlines keeps one — and `--file` keeps every byte, for the case where that
/// is not good enough.
fn strip_one_trailing_newline(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    bytes
}

/// The domain decision, made explicit because both answers are dangerous by
/// accident and neither is a sensible default.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Domain {
    Restricted(String),
    None,
}

impl Domain {
    fn pick(domain: Option<String>, no_domain: bool) -> Result<Self> {
        match (domain, no_domain) {
            (Some(pattern), false) => {
                if pattern.trim().is_empty() {
                    bail!("--domain needs a host pattern; pass --no-domain to store no domain.");
                }
                Ok(Self::Restricted(pattern))
            }
            (None, true) => Ok(Self::None),
            (None, false) => bail!(
                "`set` needs --domain <pattern> or --no-domain. A secret's domain decides which \
                 hosts HttpService will send it to, a `set` replaces the whole secret, and a \
                 domain left unsaid would be a domain cleared: use --domain \"api.example.com\", \
                 --domain \"*\" for any host, or --no-domain for a value that must never leave \
                 the server."
            ),
            // clap's `conflicts_with` rejects this before it gets here; the arm
            // exists so the function is total rather than relying on that.
            (Some(_), true) => bail!("--domain and --no-domain cannot both be given."),
        }
    }

    /// What Roblox is sent. The empty string, not an absent field: `set`
    /// replaces the whole secret, so "no domain" has to be stated.
    fn as_wire(&self) -> &str {
        match self {
            Self::Restricted(pattern) => pattern,
            Self::None => "",
        }
    }

    fn as_option(&self) -> Option<&str> {
        match self {
            Self::Restricted(pattern) => Some(pattern),
            Self::None => None,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Restricted(pattern) => format!("domain {pattern}"),
            Self::None => "no domain: server-side use only".to_string(),
        }
    }
}

/// What a write turned out to be. Only known after the fact — see [`Api::upsert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Written {
    Created,
    Updated,
}

impl Written {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }
}

struct Api {
    client: Client,
    base: ApiBase,
    api_key: String,
    universe_id: u64,
}

impl std::fmt::Debug for Api {
    /// Hand-written so that no future `dbg!` or `{:?}` on this struct prints
    /// the API key. `#[derive(Debug)]` would.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base)
            .field("universe_id", &self.universe_id)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl Api {
    fn secrets_url(&self) -> String {
        self.base
            .join(&format!("/cloud/v2/universes/{}/secrets", self.universe_id))
    }

    fn secret_url(&self, id: &str) -> String {
        format!("{}/{}", self.secrets_url(), encode_query_value(id))
    }

    /// The universe's public key, and the id a write has to quote alongside it.
    async fn public_key(&self) -> Result<UniverseKey> {
        let url = format!("{}/public-key", self.secrets_url());
        let response: Secret = execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
        .context("fetching the universe's public key, which a secret has to be sealed against")?;

        UniverseKey::from_response(&response)
    }

    /// Create if absent, replace if present.
    ///
    /// `POST` first and fall back to `PATCH` on `409 Conflict`, rather than
    /// listing the secrets to find out which it is. Two reasons, and the
    /// second is the one that decided it:
    ///
    /// - **One scope.** Listing needs `universe.secret:read`. Doing it on
    ///   every write would mean a key that can only write cannot write, which
    ///   is precisely the key a deployment pipeline should be holding.
    /// - **No race.** Read-then-branch has a window: two pipelines writing the
    ///   same secret can both read "absent" and both `POST`, and one gets a
    ///   `409` it was not written to expect. Here the `409` *is* the branch.
    ///
    /// The `409` is the one thing here a mock cannot establish, since a mock
    /// answers whatever it was told to. Confirmed against live Open Cloud on
    /// 2026-08-22: two `set` calls on one name, the second returning `updated`
    /// through this path, with a changed `--domain` that the listing then read
    /// back. So the fallback is a measured behaviour rather than a reading of
    /// the specification.
    async fn upsert(&self, id: &str, body: &Secret) -> Result<Written> {
        let create = Secret {
            id: Some(id.to_string()),
            ..body.clone()
        };

        let url = self.secrets_url();
        let created: Result<Secret> = execute_json(|| {
            let request = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .json(&create);
            async move { request.send().await.map_err(Into::into) }
        })
        .await;

        match created {
            Ok(_) => Ok(Written::Created),
            // The typed status, not the rendered message: a message match
            // would fire on a body that merely mentions 409.
            Err(error) if is_api_status(&error, StatusCode::CONFLICT) => {
                let url = self.secret_url(id);
                // `id` is not repeated in the update body. It is in the path,
                // and the specification is explicit that it cannot be changed.
                let _: Secret = execute_json(|| {
                    let request = self
                        .client
                        .patch(&url)
                        .header("x-api-key", &self.api_key)
                        .json(body);
                    async move { request.send().await.map_err(Into::into) }
                })
                .await
                .map_err(explain_missing_scope)?;
                Ok(Written::Updated)
            }
            Err(error) => Err(explain_missing_scope(error)),
        }
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let url = self.secret_url(id);
        let result = execute_with_retry(|| {
            let request = self.client.delete(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await;

        match result {
            Ok(_) => Ok(()),
            // `execute_with_retry` turns every non-success status into an
            // error, so this is the branch a 404 arrives through — not a
            // status check on a returned response. Matched on the typed status
            // rather than the rendered message, which embeds the body.
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => bail!(
                "no secret \"{id}\" in universe {}. `rbx secret list` shows what is there.",
                self.universe_id
            ),
            Err(error) => Err(explain_missing_scope(error)),
        }
    }

    /// Up to `limit` secrets, following cursors as far as needed.
    async fn list(&self, limit: u32) -> Result<Vec<Secret>> {
        let mut collected: Vec<Secret> = Vec::new();
        let mut cursor: Option<String> = None;

        // Fixed for the whole walk rather than shrunk as the remainder falls:
        // a cursor is issued against the parameters of the call that produced
        // it, and the surplus on the last page is discarded by the truncate
        // below anyway.
        let page_size = 1u32.max(limit.min(MAX_PAGE_SIZE));

        while (collected.len() as u32) < limit {
            let mut url = format!("{}?limit={}", self.secrets_url(), page_size);
            if let Some(token) = &cursor {
                url.push_str("&cursor=");
                url.push_str(&encode_query_value(token));
            }

            let page: SecretList = execute_json(|| {
                let request = self.client.get(&url).header("x-api-key", &self.api_key);
                async move { request.send().await.map_err(Into::into) }
            })
            .await
            .map_err(explain_missing_scope)?;

            let empty = page.secrets.is_empty();
            collected.extend(page.secrets.iter().cloned());

            match page.next_page() {
                // An empty page with a cursor would otherwise spin forever.
                Some(_) if empty => break,
                Some(next) => cursor = Some(next.to_string()),
                None => break,
            }
        }

        collected.truncate(limit as usize);
        Ok(collected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_adding_a_newline_does_not_add_it_to_the_secret() {
        assert_eq!(strip_one_trailing_newline(b"key".to_vec()), b"key");
        assert_eq!(strip_one_trailing_newline(b"key\n".to_vec()), b"key");
        assert_eq!(strip_one_trailing_newline(b"key\r\n".to_vec()), b"key");
        // Exactly one. A value that really ends in a blank line keeps it.
        assert_eq!(strip_one_trailing_newline(b"key\n\n".to_vec()), b"key\n");
        // Interior newlines are content: a PEM key piped in stays a PEM key.
        assert_eq!(
            strip_one_trailing_newline(b"line\nline\n".to_vec()),
            b"line\nline"
        );
        assert_eq!(strip_one_trailing_newline(Vec::new()), Vec::<u8>::new());
    }

    #[test]
    fn a_domain_has_to_be_decided_rather_than_defaulted() {
        assert!(Domain::pick(None, false).is_err());
        assert_eq!(Domain::pick(None, true).expect("none"), Domain::None);
        assert_eq!(
            Domain::pick(Some("api.example.com".into()), false).expect("restricted"),
            Domain::Restricted("api.example.com".into())
        );
        // An empty --domain is the mistake that silently means --no-domain.
        assert!(Domain::pick(Some("   ".into()), false).is_err());
    }

    /// "No domain" has to travel as an empty string. Omitting the field on a
    /// `PATCH` is the ambiguous case this design exists to avoid.
    #[test]
    fn no_domain_is_sent_explicitly_rather_than_left_out() {
        assert_eq!(Domain::None.as_wire(), "");
        assert_eq!(Domain::None.as_option(), None);
        assert_eq!(Domain::Restricted("*".into()).as_wire(), "*");
    }

    #[test]
    fn a_value_has_to_come_from_somewhere() {
        assert!(ValueSource::pick(None, None, false).is_err());
        assert!(matches!(
            ValueSource::pick(Some("v".into()), None, false),
            Ok(ValueSource::Inline(_))
        ));
        assert!(matches!(
            ValueSource::pick(None, Some(PathBuf::from("k.pem")), false),
            Ok(ValueSource::File(_))
        ));
        assert!(matches!(
            ValueSource::pick(None, None, true),
            Ok(ValueSource::Stdin)
        ));
    }

    /// The API key must not reach a debug rendering. `#[derive(Debug)]` on
    /// `Api` would put it there, and a `{:?}` in an error path is how it would
    /// escape.
    #[test]
    fn the_api_key_is_not_in_the_debug_output() {
        let api = Api {
            client: build_client(),
            base: ApiBase::new("https://example.invalid"),
            api_key: "SECRET-KEY-CANARY".to_string(),
            universe_id: 1,
        };

        let rendered = format!("{api:?}");
        assert!(!rendered.contains("SECRET-KEY-CANARY"), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
    }
}
