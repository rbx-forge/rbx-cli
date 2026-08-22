//! `rbx-ops ban` : look at and change who is allowed into an experience.
//!
//! Everything that writes here is dry-run first and prompts second, because
//! the input is a name typed by a person under pressure, usually from a Discord
//! report, and the output is a real player locked out of a real game. The
//! resolved account is printed with its profile link before anything is sent,
//! so the wrong `Builderman` is caught by a human rather than by an appeal.

pub mod json;
pub mod model;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use reqwest::{Client, StatusCode};

use rbx_core::api::{
    build_client, encode_query_value, execute_json, execute_with_retry_policy,
    explain_missing_scope, is_api_status, require_api_key, ApiBase, RetryPolicy,
};
use rbx_core::confirm::confirm_always;
use rbx_core::output::{emit, OutputFormat};
use rbx_core::users::{self, User, UserRef};
use rbx_core::GlobalFlags;

use crate::json::{ListDocument, StatusDocument};
use crate::model::{
    parse_duration, GameJoinRestrictionUpdate, LogPage, RestrictionPage, RestrictionUpdate,
    UserRestriction, MAX_DISPLAY_REASON, MAX_PAGE_SIZE, MAX_PRIVATE_REASON,
};

#[derive(Args, Debug)]
pub struct BanCli {
    #[command(subcommand)]
    command: Command,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,

    /// Override the users.roblox.com host. For testing.
    #[arg(long, hide = true, global = true)]
    users_url: Option<String>,
}

impl BanCli {
    /// Point this invocation at mock hosts instead of Roblox.
    ///
    /// Tests only. Both hosts are hidden flags rather than public fields so
    /// that nothing in production can set them by accident, and this is the
    /// one door into them.
    #[doc(hidden)]
    pub fn with_hosts(mut self, api: String, users: String) -> Self {
        self.base_url = Some(api);
        self.users_url = Some(users);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show whether specific players are restricted
    ///
    /// Accepts a user id, a username, `@username`, or a pasted profile link.
    Status {
        /// Players to look up.
        #[arg(required = true)]
        users: Vec<String>,

        /// Write the result to stdout as one JSON document.
        ///
        /// One object per player asked about, with the account, whether they
        /// are restricted, and (when they are) the length and both reasons,
        /// the same facts the human form prints. stdout carries the document
        /// and nothing else; notes and warnings stay on stderr. Field names are
        /// documented in docs/ops/ban.md.
        #[arg(long)]
        json: bool,
    },

    /// List every restricted player in the experience
    List {
        /// Maximum rows to fetch.
        #[arg(long, default_value_t = 100)]
        limit: u32,

        /// Include entries that exist but are not currently active.
        #[arg(long)]
        include_inactive: bool,

        /// Write the result to stdout as one JSON document.
        ///
        /// One object per restricted player: the id, whether the entry is
        /// active, whether it is permanent, and the private note. No names:
        /// this endpoint does not send them. stdout carries the document and
        /// nothing else; notes and warnings stay on stderr. Field names are
        /// documented in docs/ops/ban.md.
        #[arg(long)]
        json: bool,
    },

    /// Show the restriction audit trail
    Logs {
        /// Maximum entries to fetch.
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },

    /// Restrict players from joining
    Add {
        /// Players to restrict.
        #[arg(required = true)]
        users: Vec<String>,

        /// Why, for your records. Never shown to the player. Required, because
        /// a ban nobody can explain in six months is a ban nobody can defend.
        #[arg(long)]
        reason: String,

        /// What the player is told. Shown to them on their next join attempt.
        #[arg(long)]
        display_reason: Option<String>,

        /// How long: `30m`, `12h`, `7d`, `2w`.
        ///
        /// Omit it for a permanent restriction. There is no `--permanent`
        /// flag on purpose: the harshest outcome should come from leaving
        /// something out deliberately, not from a word you could mistype.
        #[arg(long)]
        duration: Option<String>,

        /// Let alt accounts through instead of restricting them too.
        ///
        /// Roblox propagates a restriction to linked alt accounts by default,
        /// which is what you want for an exploiter. This turns that off.
        ///
        /// Named for what it does rather than after Roblox's field: theirs is
        /// `excludeAltAccounts`, where `true` means "do not propagate". A flag
        /// called `--exclude-alts` reads as "include the alts in the ban" to
        /// every person who has ever typed it, which is the opposite.
        #[arg(long)]
        allow_alts: bool,

        /// Actually send it. Without this, nothing is written.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt. For scripts.
        #[arg(long)]
        yes: bool,
    },

    /// Lift a restriction
    Remove {
        /// Players to unrestrict.
        #[arg(required = true)]
        users: Vec<String>,

        /// Actually send it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

struct Context_ {
    client: Client,
    base: ApiBase,
    users_host: String,
    api_key: String,
    universe_id: u64,
    /// The env that named `universe_id`, for the documents to say which leg of
    /// a matrix job produced them. `None` under a bare `--universe-id`, which
    /// wins over `--env`: naming the env there would credit a file that was
    /// never consulted.
    env: Option<String>,
}

pub async fn run(cli: BanCli, global: &GlobalFlags) -> Result<()> {
    if global.env.as_deref() == Some("all") {
        bail!(
            "`--env all` is refused here. Restricting a player across every environment at once \
             is never what someone means, and it is not undoable in one step either."
        );
    }

    let universe_id = global.single_universe()?;

    let ctx = Context_ {
        client: build_client(),
        base: match &cli.base_url {
            Some(url) => ApiBase::new(url.clone()),
            None => ApiBase::default(),
        },
        users_host: cli
            .users_url
            .clone()
            .unwrap_or_else(|| "https://users.roblox.com".to_string()),
        api_key: require_api_key(global.api_key.as_deref())?.to_string(),
        universe_id,
        env: match global.universe_id {
            Some(_) => None,
            None => global.env.clone(),
        },
    };

    match cli.command {
        Command::Status { users, json } => {
            status(&ctx, &users, OutputFormat::from_json_flag(json)).await
        }
        Command::List {
            limit,
            include_inactive,
            json,
        } => {
            list(
                &ctx,
                limit,
                include_inactive,
                OutputFormat::from_json_flag(json),
            )
            .await
        }
        Command::Logs { limit } => logs(&ctx, limit).await,
        Command::Add {
            users,
            reason,
            display_reason,
            duration,
            allow_alts,
            apply,
            yes,
        } => {
            if reason.chars().count() > MAX_PRIVATE_REASON {
                bail!(
                    "--reason is {} characters; Roblox allows {MAX_PRIVATE_REASON}",
                    reason.chars().count()
                );
            }
            if let Some(shown) = &display_reason {
                if shown.chars().count() > MAX_DISPLAY_REASON {
                    bail!(
                        "--display-reason is {} characters; Roblox allows {MAX_DISPLAY_REASON}",
                        shown.chars().count()
                    );
                }
            }
            let duration = duration.as_deref().map(parse_duration).transpose()?;
            let update = RestrictionUpdate {
                game_join_restriction: GameJoinRestrictionUpdate {
                    active: true,
                    duration,
                    private_reason: Some(reason),
                    display_reason,
                    // Sent only when turning propagation off; leaving it
                    // out keeps Roblox's default, which is to propagate.
                    exclude_alt_accounts: allow_alts.then_some(true),
                },
            };
            write(&ctx, &users, update, apply, yes, "restrict").await
        }
        Command::Remove { users, apply, yes } => {
            let update = RestrictionUpdate {
                game_join_restriction: GameJoinRestrictionUpdate {
                    active: false,
                    duration: None,
                    private_reason: None,
                    display_reason: None,
                    exclude_alt_accounts: None,
                },
            };
            write(&ctx, &users, update, apply, yes, "unrestrict").await
        }
    }
}

async fn resolve(ctx: &Context_, inputs: &[String]) -> Result<Vec<User>> {
    let refs = inputs
        .iter()
        .map(|input| UserRef::parse(input))
        .collect::<Result<Vec<_>>>()?;
    users::resolve_with_host(&ctx.client, &refs, &ctx.users_host).await
}

async fn status(ctx: &Context_, inputs: &[String], format: OutputFormat) -> Result<()> {
    let mut document = StatusDocument::new(ctx.env.as_deref(), ctx.universe_id);

    for user in resolve(ctx, inputs).await? {
        let url = ctx.base.join(&format!(
            "/cloud/v2/universes/{}/user-restrictions/{}",
            ctx.universe_id, user.id
        ));
        let restriction: UserRestriction = execute_json(|| {
            let request = ctx.client.get(&url).header("x-api-key", &ctx.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;

        // The lookups stay a loop under both formats: a run that fails on the
        // third player has already answered for the first two, and the human
        // form has already printed them.
        if format.is_json() {
            document.push(&user, &restriction);
            continue;
        }

        println!("{}", user.label().bold());
        println!("  {}", user.profile_url().dimmed());
        if restriction.is_active() {
            let join = restriction.game_join_restriction.as_ref();
            println!(
                "  {}  for {}",
                "RESTRICTED".red().bold(),
                restriction.duration_label()
            );
            if let Some(since) = join.and_then(|r| r.start_time.as_deref()) {
                println!("  since        {since}");
            }
            if let Some(reason) = join.and_then(|r| r.private_reason.as_deref()) {
                println!("  your note    {reason}");
            }
            if let Some(shown) = join.and_then(|r| r.display_reason.as_deref()) {
                println!("  player sees  {shown}");
            }
        } else {
            println!("  {}", "not restricted".green());
        }
        println!();
    }

    if format.is_json() {
        emit(&document)?;
    }
    Ok(())
}

/// Follow the restriction listing until `limit` rows are held or Roblox runs
/// out of pages.
///
/// Public and named rather than inlined into `list`, because the three
/// properties that make a paged walk correct cannot be asserted from outside
/// `run`, and that is precisely how the missing truncate reached production in
/// `servers` and `memorystore`: every test passed with the bug in place.
pub async fn walk_restrictions(
    client: &Client,
    base: &ApiBase,
    api_key: &str,
    universe_id: u64,
    limit: u32,
    include_inactive: bool,
) -> Result<Vec<UserRestriction>> {
    let mut rows: Vec<UserRestriction> = Vec::new();
    let mut token: Option<String> = None;

    // Decided once for the whole walk, not recomputed per page. `cloud/v2`
    // requires every paged call to repeat the parameters of the call that
    // issued its token, so shrinking this as the remaining count falls would
    // send page two with a value Roblox is entitled to reject. Overshooting on
    // the last page costs nothing, because the truncate below discards the
    // surplus, and the two only work paired: a fixed page size without the
    // truncate is what made `--limit 5` answer with 100 rows.
    let page_size = 1u32.max(limit.min(MAX_PAGE_SIZE));

    while (rows.len() as u32) < limit {
        let mut url = base.join(&format!(
            "/cloud/v2/universes/{universe_id}/user-restrictions?maxPageSize={page_size}"
        ));
        if let Some(page_token) = &token {
            url.push_str("&pageToken=");
            url.push_str(&encode_query_value(page_token));
        }
        let page: RestrictionPage = execute_json(|| {
            let request = client.get(&url).header("x-api-key", api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;

        // Read the token before consuming the rows: `next_token` borrows the
        // page, and the loop below moves part of it.
        let next = page.next_token().map(str::to_string);

        // Emptiness is judged on what Roblox returned, never on what survived
        // `include_inactive`. A page holding nothing but lifted restrictions is
        // a page to step over, not a reason to stop: measuring it after the
        // filter would end the walk early and answer with less than was asked.
        let empty = page.user_restrictions.is_empty();

        for restriction in page.user_restrictions {
            if include_inactive || restriction.is_active() {
                rows.push(restriction);
            }
        }
        match next {
            // An empty page carrying a token would otherwise spin for ever:
            // the row count never grows, so the loop condition never fails.
            Some(_) if empty => break,
            Some(value) => token = Some(value),
            None => break,
        }
    }

    rows.truncate(limit as usize);
    Ok(rows)
}

async fn list(
    ctx: &Context_,
    limit: u32,
    include_inactive: bool,
    format: OutputFormat,
) -> Result<()> {
    let rows = walk_restrictions(
        &ctx.client,
        &ctx.base,
        &ctx.api_key,
        ctx.universe_id,
        limit,
        include_inactive,
    )
    .await?;

    if format.is_json() {
        return emit(&ListDocument::new(
            ctx.env.as_deref(),
            ctx.universe_id,
            limit,
            include_inactive,
            &rows,
        ));
    }

    if rows.is_empty() {
        println!("{}", "Nobody is restricted in this experience.".green());
        return Ok(());
    }

    println!(
        "{:<14}  {:<12}  {}",
        "USER".bold(),
        "FOR".bold(),
        "REASON".bold()
    );
    for restriction in &rows {
        let id = restriction
            .user_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "?".into());
        let reason = restriction
            .game_join_restriction
            .as_ref()
            .and_then(|r| r.private_reason.as_deref())
            .unwrap_or("-");
        println!("{id:<14}  {:<12}  {reason}", restriction.duration_label());
    }
    println!();
    println!("{} restricted", rows.len());
    println!(
        "{}",
        "Names are not returned by this endpoint; `ban status <id>` resolves one.".dimmed()
    );
    Ok(())
}

async fn logs(ctx: &Context_, limit: u32) -> Result<()> {
    let url = ctx.base.join(&format!(
        "/cloud/v2/universes/{}/user-restrictions:listLogs?maxPageSize={}",
        ctx.universe_id,
        limit.min(100)
    ));
    let page: LogPage = execute_json(|| {
        let request = ctx.client.get(&url).header("x-api-key", &ctx.api_key);
        async move { request.send().await.map_err(Into::into) }
    })
    .await
    .map_err(explain_missing_scope)?;

    if page.logs.is_empty() {
        println!("{}", "No restriction activity recorded.".dimmed());
        return Ok(());
    }

    println!(
        "{:<20}  {:<10}  {:<12}  {:<10}  {}",
        "WHEN".bold(),
        "ACTION".bold(),
        "USER".bold(),
        "FOR".bold(),
        "BY / REASON".bold()
    );
    for entry in &page.logs {
        let when = entry
            .create_time
            .as_deref()
            .map(|stamp| {
                stamp
                    .replace('T', " ")
                    .split('.')
                    .next()
                    .unwrap_or(stamp)
                    .to_string()
            })
            .unwrap_or_else(|| "-".into());
        let action = if entry.active {
            entry.action().red().bold()
        } else {
            entry.action().green()
        };
        let user = entry
            .user_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "?".into());
        let by = entry
            .moderator_id()
            .map(|id| id.to_string())
            // No moderator means Roblox acted rather than a person.
            .unwrap_or_else(|| "roblox".into());
        let reason = entry.private_reason.as_deref().unwrap_or("");
        let pad = 10usize.saturating_sub(entry.action().len());
        println!(
            "{when:<20}  {action}{:pad$}  {user:<12}  {:<10}  {by} {}",
            "",
            entry.duration_label(),
            reason.dimmed()
        );
    }
    println!();
    println!(
        "{}",
        "An unban is an entry too, so this is a history, not a current state. \
         Use `ban list` for who is restricted right now."
            .dimmed()
    );
    Ok(())
}

async fn write(
    ctx: &Context_,
    inputs: &[String],
    update: RestrictionUpdate,
    apply: bool,
    yes: bool,
    verb: &str,
) -> Result<()> {
    let targets = resolve(ctx, inputs).await?;

    // Always shown, whether or not this run will send anything. Seeing the
    // account you are about to act on, with a link to check it, is the step
    // that catches the wrong Builderman.
    println!("{} {} player(s):", verb, targets.len());
    for user in &targets {
        println!("  {}", user.label().bold());
        println!("    {}", user.profile_url().dimmed());
    }
    println!();
    println!("{}", serde_json::to_string_pretty(&update)?);
    println!();

    if !apply {
        println!(
            "{}",
            "Nothing sent. Re-run with --apply to perform it.".yellow()
        );
        return Ok(());
    }

    confirm_always(
        &format!(
            "{} {} player(s) in universe {}?",
            verb,
            targets.len(),
            ctx.universe_id
        ),
        yes,
    )?;

    for user in &targets {
        // An idempotency key makes a retry after a timeout safe: Roblox
        // recognises the repeat instead of applying it twice.
        let idempotency = format!("rbx-ops-{}-{}-{}", ctx.universe_id, user.id, verb);
        let url = ctx.base.join(&format!(
            "/cloud/v2/universes/{}/user-restrictions/{}?updateMask=gameJoinRestriction\
             &idempotencyKey.key={}",
            ctx.universe_id, user.id, idempotency
        ));

        // Roblox rate-limits restriction writes *per user per universe*, and
        // the cooldown outlasts the shared default policy (3 tries, ~7s
        // total). Confirmed by banning and immediately unbanning one account:
        // the second call answered 429 RESOURCE_EXHAUSTED and exhausted every
        // retry. Banning then correcting yourself is a normal thing to do, so
        // the wait is stretched to roughly a minute here rather than left to
        // fail in the caller's face.
        let policy = RetryPolicy {
            max_retries: 5,
            base_backoff_secs: 2,
        };

        execute_with_retry_policy(
            || {
                let request = ctx
                    .client
                    .patch(&url)
                    .header("x-api-key", &ctx.api_key)
                    .json(&update);
                async move { request.send().await.map_err(Into::into) }
            },
            &policy,
        )
        .await
        .map_err(|error| {
            let error = explain_missing_scope(error);
            if is_api_status(&error, StatusCode::TOO_MANY_REQUESTS) {
                error.context(format!(
                    "Roblox is still rate-limiting writes for {} in this universe. \
                     The limit is per player, so waiting a minute and re-running the same \
                     command is the fix; nothing was applied.",
                    user.id
                ))
            } else {
                error
            }
        })
        .with_context(|| format!("{verb} {}", user.label()))?;

        println!("{} {}", "done".green().bold(), user.label());
    }
    Ok(())
}

/// Where `--json` is allowed to appear, and why that is a rule rather than an
/// arrangement.
#[cfg(test)]
mod json_flag_tests {
    use super::*;

    #[derive(clap::Parser)]
    struct Wrapper {
        #[command(flatten)]
        ban: BanCli,
    }

    fn parses(args: &[&str]) -> bool {
        let mut argv = vec!["ban"];
        argv.extend_from_slice(args);
        <Wrapper as clap::Parser>::try_parse_from(argv).is_ok()
    }

    /// A format that owns stdout may not stop to ask a question: that is
    /// `OutputFormat::may_prompt`, and it is false for `Json` whatever the
    /// terminal looks like. Both writing subcommands here ask one, through
    /// `confirm_always`, so neither carries the flag: the guarantee is
    /// structural rather than a check somebody has to remember to write.
    ///
    /// This pins it. Adding `--json` to `add` would fail here, before it could
    /// draw a y/N prompt into somebody's pipe on the way to banning a player.
    #[test]
    fn json_is_confined_to_the_subcommands_that_never_prompt() {
        assert!(!OutputFormat::Json.may_prompt());

        for reading in [
            vec!["status", "builderman", "--json"],
            vec!["list", "--json"],
            vec!["list", "--include-inactive", "--json"],
        ] {
            assert!(parses(&reading), "{reading:?} should take --json");
        }

        for writing in [
            vec!["add", "builderman", "--reason", "x", "--json"],
            vec!["remove", "builderman", "--json"],
        ] {
            assert!(!parses(&writing), "{writing:?} must not take --json");
        }
    }

    /// `logs` is out of this lot rather than forbidden: the audit trail has a
    /// document's worth of structure and nobody has asked for it yet. The flag
    /// is absent, not refused, and adding it later is a normal change.
    #[test]
    fn logs_has_no_document_yet() {
        assert!(parses(&["logs"]));
        assert!(!parses(&["logs", "--json"]));
    }
}
