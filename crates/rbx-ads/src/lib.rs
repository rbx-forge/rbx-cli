//! `rbx-ops ads` : launch and steer Roblox ad campaigns.
//!
//! This tool spends money. It sits in `rbx-ops` rather than `rbx` for the
//! reason `rbx-ops` exists at all: a campaign has no desired state to
//! reconcile from a file in the repo. Declaring campaigns in TOML would mean a
//! merge could relaunch one, or move a budget, with nobody's hand on it. So
//! every write here is a command somebody typed, dry-run by default, and needs
//! `--apply`.
//!
//! # What this cannot do
//!
//! Read results. `/ads-management/v1` has no reporting endpoint, which is
//! deliberate: Roblox says reporting is not in v1 and will arrive through the
//! Analytics API. Until then impressions, clicks and spend live in Ads
//! Manager, and this tool cannot fetch them.
//!
//! That shapes `launch`. Since the numbers are read by a human in a web page,
//! the campaign **name** is the only thread tying a row in that page back to
//! the creative it tested, so `launch` writes the asset id into every name it
//! creates.
//!
//! # Why one campaign per creative
//!
//! A campaign accepts up to ten creatives and Roblox distributes them evenly,
//! which is a fair experiment. But Ads Manager reports per campaign, not per
//! creative, so ten creatives in one campaign produce one number for the ten.
//! To learn which image won, each needs its own campaign. `launch` builds them
//! from a single set of flags, so the variants differ by their creative and
//! nothing else, which is the property the comparison rests on.
//!
//! The cost of that shape is real and worth knowing: identical campaigns chase
//! the same impressions, and internal competition makes each dollar buy a
//! little less. It inflates every variant equally, so the ranking survives
//! even when the absolute numbers suffer.
//!
//! # Status
//!
//! Roblox ships this API as an experiment and says request and response shapes
//! can change, and not to lean on it for production-critical automation. The
//! `rbx-spec-drift` test is the alarm for the day a path moves.

mod api;
pub mod json;
pub mod model;

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use colored::Colorize;

use rbx_core::api::{build_client, require_api_key, ApiBase};
use rbx_core::confirm::confirm_always;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use crate::json::{GetDocument, ListDocument, StatusDocument};
use crate::model::{
    micros_to_usd, usd_to_micros, Bid, Budget, CreateCampaign, Schedule, Targeting, UpdateBudget,
    UpdateCampaign,
};

/// The document allows exactly one of each today. Named constants rather than
/// flags: offering a choice the API refuses is a worse experience than not
/// offering it, and when Roblox adds a second value this is where it lands.
const OBJECTIVE: &str = "ENGAGEMENT";
const BID_STRATEGY: &str = "AUTOMATED";

#[derive(Args, Debug)]
pub struct AdsCli {
    #[command(subcommand)]
    command: Command,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

impl AdsCli {
    /// Point this invocation at a mock host. Tests only; `base_url` stays a
    /// hidden flag so nothing in production can set it by accident.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create one campaign per creative, identical in every other respect
    ///
    /// The shape of a thumbnail or icon test. Each campaign carries one image
    /// and the same budget, schedule and targeting as its siblings, so the
    /// only thing that differs is the thing under test.
    ///
    /// Results are not readable here. Read them in Ads Manager: every campaign
    /// this creates is named with its asset id so the rows can be told apart.
    Launch {
        /// Image asset id to test. Repeat it once per variant.
        ///
        /// Omit it on a terminal to pick from the account's creatives instead
        /// of copying ids out of `ads creatives` by hand.
        #[arg(long = "creative", value_name = "ASSET_ID")]
        creatives: Vec<String>,

        /// Base name. Each campaign gets it plus its asset id.
        #[arg(long)]
        name: String,

        /// Budget per campaign in dollars, e.g. `25` or `25.50`.
        ///
        /// Per campaign, not for the test: five variants at 25 is 125 dollars.
        #[arg(long)]
        budget: String,

        /// Whether the budget is spent per day or over the whole run.
        #[arg(long = "budget-type", value_parser = ["DAILY", "LIFETIME"], default_value = "DAILY")]
        budget_type: String,

        /// How many days to run.
        #[arg(long, default_value_t = 7)]
        days: u32,

        /// When to start, RFC 3339. Defaults to as soon as review clears.
        #[arg(long, value_name = "RFC3339")]
        start: Option<String>,

        /// How to pay. See `ads options` for what this account allows.
        #[arg(long, value_parser = ["CREDIT_CARD", "ADS_CREDIT", "INVOICE"], default_value = "CREDIT_CARD")]
        payment: String,

        /// Restrict to these countries, e.g. `--country US`. Repeatable.
        #[arg(long = "country", value_name = "ISO2")]
        countries: Vec<String>,

        /// Restrict to these age brackets. Repeatable.
        #[arg(long = "age", value_parser = ["AGE_13_17", "AGE_18_24", "AGE_25_PLUS"])]
        ages: Vec<String>,

        /// Restrict to these devices. Repeatable.
        #[arg(long = "device", value_parser = ["PHONE", "TABLET", "DESKTOP", "CONSOLE"])]
        devices: Vec<String>,

        /// Actually create the campaigns. Without it, nothing is sent.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },

    /// List the campaigns on this account
    List {
        /// Write the result to stdout as one JSON document instead of a table.
        ///
        /// Budgets carry both the exact micro-USD Roblox sent and the dollar
        /// figure the table prints, neither as a JSON number. stdout carries
        /// the document and nothing else; diagnostics stay on stderr. Field
        /// names are documented in docs/ops/ads.md.
        #[arg(long)]
        json: bool,
    },

    /// Show one campaign
    Get {
        /// Campaign id.
        id: String,

        /// Write the result to stdout as one JSON document.
        ///
        /// The campaign sits under `campaign`, the same object `list` puts in
        /// `campaigns`, so one filter reads either. Field names are documented
        /// in docs/ops/ads.md.
        #[arg(long)]
        json: bool,
    },

    /// Ask whether campaigns are serving, in review, or blocked
    Status {
        /// Campaign ids. Repeatable.
        #[arg(required = true)]
        ids: Vec<String>,

        /// Write the result to stdout as one JSON document.
        ///
        /// Answered ids and refused ones stay in separate arrays, so a script
        /// cannot read an id Roblox never answered for as a campaign that is
        /// not serving. Field names are documented in docs/ops/ads.md.
        #[arg(long)]
        json: bool,
    },

    /// Pause running campaigns
    ///
    /// With no id and a terminal to ask on, pick from the list. With `--name`,
    /// act on every campaign whose name starts with it, which is how a whole
    /// `launch` group is stopped at once.
    Pause {
        id: Option<String>,
        /// Act on every campaign whose name starts with this.
        #[arg(long, conflicts_with = "id")]
        name: Option<String>,
        #[arg(long)]
        apply: bool,
        #[arg(long, short)]
        yes: bool,
    },

    /// Resume paused campaigns
    Resume {
        id: Option<String>,
        /// Act on every campaign whose name starts with this.
        #[arg(long, conflicts_with = "id")]
        name: Option<String>,
        #[arg(long)]
        apply: bool,
        #[arg(long, short)]
        yes: bool,
    },

    /// Cancel campaigns for good
    ///
    /// Roblox does not document a way back from `CANCELLED`. Pause instead if
    /// they might run again.
    Cancel {
        id: Option<String>,
        /// Act on every campaign whose name starts with this.
        #[arg(long, conflicts_with = "id")]
        name: Option<String>,
        #[arg(long)]
        apply: bool,
        #[arg(long, short)]
        yes: bool,
    },

    /// Change a campaign's budget
    ///
    /// An increase takes effect at once. A decrease on a running campaign is
    /// applied at the next midnight in the account's time zone, and the
    /// campaign keeps spending the higher figure until then.
    Budget {
        id: Option<String>,
        /// New amount in dollars.
        #[arg(long)]
        amount: String,
        #[arg(long)]
        apply: bool,
        #[arg(long, short)]
        yes: bool,
    },

    /// List the images available as creatives
    Creatives {
        /// Show archived creatives instead of live ones.
        #[arg(long)]
        archived: bool,
    },

    /// List the experiences this account may advertise
    Universes,

    /// List the billing accounts this key can spend from
    Accounts,

    /// Rename a campaign
    ///
    /// The name is the only thread from a row in Ads Manager back to the
    /// creative it carried, so a typo made at launch would follow the whole
    /// test. Renaming does not touch delivery.
    Rename {
        id: String,
        /// The new name.
        #[arg(long)]
        name: String,
        #[arg(long)]
        apply: bool,
        #[arg(long, short)]
        yes: bool,
    },

    /// Show what a campaign for this experience may ask for
    ///
    /// Formats and their pixel sizes, objectives, payment types, the targeting
    /// dimensions on offer, and whether the experience is eligible at all.
    Options,
}

/// Describe a campaign the way a confirmation prompt needs to: enough to
/// recognise the wrong one before saying yes.
///
/// An id is opaque. `c_8f3a91` pasted from a list tells you nothing about what
/// you are pausing; the name, its state and its budget do.
fn describe(campaign: &model::Campaign) -> String {
    let budget = campaign
        .budget
        .as_ref()
        .map(|b| micros_to_usd(&b.amount_micros))
        .unwrap_or_else(|| "-".into());
    format!(
        "{} · {} · {} · {}",
        campaign.name, campaign.status, campaign.delivery_status, budget
    )
}

/// The format the commands that can ask a question run under.
///
/// Only writes prompt here (`launch` for its creatives, `pause`, `resume`,
/// `cancel` and `budget` for their target) and none of them has a `--json`:
/// this issue's flag covers reads, and a command that spends money is not one
/// to hand a pipeline. So the format is `Human` by construction.
///
/// Spelled out and routed through [`OutputFormat::may_prompt`] rather than
/// tested against a bare terminal check, because that is what makes it a fact
/// about today instead of a trap for tomorrow: the day one of these grows a
/// `--json`, the prompt is refused by the type rather than left to corrupt a
/// document. `may_prompt` also carries the terminal test itself, which this
/// crate used to keep a private copy of.
const WRITE_FORMAT: OutputFormat = OutputFormat::Human;

pub async fn run(cli: AdsCli, global: &GlobalFlags) -> Result<()> {
    let api_key = require_api_key(global.api_key.as_deref())?;
    let base = match &cli.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let client = build_client();

    match cli.command {
        Command::Launch {
            creatives,
            name,
            budget,
            budget_type,
            days,
            start,
            payment,
            countries,
            ages,
            devices,
            apply,
            yes,
        } => {
            let universe_id = global.single_universe()?;
            let amount_micros = usd_to_micros(&budget)?;

            let creatives = if creatives.is_empty() {
                pick_creatives(&client, &base, api_key).await?
            } else {
                creatives
            };

            let mut seen = creatives.clone();
            seen.sort();
            seen.dedup();
            if seen.len() != creatives.len() {
                bail!(
                    "the same creative is listed twice. Two identical campaigns would split the \
                     impressions of one and tell you nothing"
                );
            }

            let planned: Vec<CreateCampaign> = creatives
                .iter()
                .map(|asset| CreateCampaign {
                    name: format!("{name} [{asset}]"),
                    target_universe_id: universe_id.to_string(),
                    creative_asset_ids: vec![asset.clone()],
                    objective: OBJECTIVE,
                    payment_type: payment.clone(),
                    budget: Budget {
                        amount_micros: amount_micros.clone(),
                        budget_type: budget_type.clone(),
                    },
                    schedule: Schedule {
                        start_time: start.clone(),
                        duration_in_days: Some(days),
                    },
                    bid: Bid {
                        strategy: BID_STRATEGY,
                    },
                    targeting: Targeting {
                        age_groups: ages.clone(),
                        countries: countries.clone(),
                        devices: devices.clone(),
                    },
                })
                .collect();

            let each = micros_to_usd(&amount_micros);
            let total = micros_to_usd(
                &(amount_micros.parse::<u64>().unwrap_or(0) * planned.len() as u64).to_string(),
            );

            println!(
                "{} campaign(s) on universe {universe_id}, {each} each {}, {days} day(s)",
                planned.len(),
                budget_type.to_lowercase()
            );
            for campaign in &planned {
                println!("  {}", campaign.name);
            }
            println!(
                "{}",
                format!("total exposure {total} {}", budget_type.to_lowercase()).bold()
            );

            if !apply {
                println!(
                    "{}",
                    "Nothing sent. Re-run with --apply to create them.".yellow()
                );
                return Ok(());
            }

            confirm_always(
                &format!(
                    "Create {} campaign(s) and commit {total} of real spend?",
                    planned.len()
                ),
                yes,
            )?;

            for campaign in &planned {
                let key = idempotency_key(campaign);
                let created = api::create_campaign(&client, &base, api_key, campaign, &key).await?;
                println!(
                    "  {} {} {}",
                    "created".green(),
                    created.id,
                    created.delivery_status.dimmed()
                );
            }
            println!(
                "{}",
                "Campaigns are queued for ad-policy review. `ads status <id>` follows them."
                    .dimmed()
            );
            println!(
                "{}",
                "Impressions and clicks are read in Ads Manager; this API reports none.".dimmed()
            );
        }

        Command::List { json } => {
            let format = OutputFormat::from_json_flag(json);
            let campaigns = api::list_campaigns(&client, &base, api_key).await?;

            if campaigns.is_empty() {
                // An account with no campaigns is an empty document rather
                // than no document: a consumer reads `.campaigns | length`
                // instead of having to tell "none" from "the command printed
                // nothing". The line saying so still goes out, on stderr.
                format.note("No campaigns on this account.");
                if format.is_json() {
                    output::emit(&ListDocument::new(&campaigns))?;
                }
                return Ok(());
            }

            if format.is_json() {
                output::emit(&ListDocument::new(&campaigns))?;
                return Ok(());
            }

            for campaign in &campaigns {
                let budget = campaign
                    .budget
                    .as_ref()
                    .map(|b| micros_to_usd(&b.amount_micros))
                    .unwrap_or_else(|| "-".into());
                println!(
                    "{:<24} {:<10} {:<12} {:>10}  {}",
                    campaign.id, campaign.status, campaign.delivery_status, budget, campaign.name
                );
            }
        }

        Command::Get { id, json } => {
            let campaign = api::get_campaign(&client, &base, api_key, &id).await?;
            if OutputFormat::from_json_flag(json).is_json() {
                output::emit(&GetDocument::new(&campaign))?;
                return Ok(());
            }
            println!("{}", campaign.name.bold());
            println!("  id           {}", campaign.id);
            println!("  status       {}", campaign.status);
            println!("  delivery     {}", campaign.delivery_status);
            for reason in &campaign.delivery_status_reasons {
                println!("               {}", reason.yellow());
            }
            if let Some(budget) = &campaign.budget {
                println!(
                    "  budget       {} {}",
                    micros_to_usd(&budget.amount_micros),
                    budget.budget_type.to_lowercase()
                );
            }
            println!("  universe     {}", campaign.target_universe_id);
            println!("  creatives    {}", campaign.creative_asset_ids.join(", "));
        }

        Command::Status { ids, json } => {
            let batch = api::batch_status(&client, &base, api_key, &ids).await?;
            if OutputFormat::from_json_flag(json).is_json() {
                output::emit(&StatusDocument::new(ids.len(), &batch))?;
                return Ok(());
            }
            for status in &batch.statuses {
                println!(
                    "{:<24} {:<10} {}",
                    status.id, status.status, status.delivery_status
                );
                for reason in &status.delivery_status_reasons {
                    println!("{:<24} {}", "", reason.yellow());
                }
            }
            for failure in &batch.failures {
                println!("{:<24} {}", failure.id, failure.reason.red());
            }
        }

        Command::Pause {
            id,
            name,
            apply,
            yes,
        } => {
            let targets = resolve(&client, &base, api_key, id, name, "pause").await?;
            set_status(&client, &base, api_key, &targets, "PAUSED", apply, yes).await?;
        }
        Command::Resume {
            id,
            name,
            apply,
            yes,
        } => {
            let targets = resolve(&client, &base, api_key, id, name, "resume").await?;
            set_status(&client, &base, api_key, &targets, "ACTIVE", apply, yes).await?;
        }
        Command::Cancel {
            id,
            name,
            apply,
            yes,
        } => {
            let targets = resolve(&client, &base, api_key, id, name, "cancel").await?;
            set_status(&client, &base, api_key, &targets, "CANCELLED", apply, yes).await?;
        }

        Command::Budget {
            id,
            amount,
            apply,
            yes,
        } => {
            let amount_micros = usd_to_micros(&amount)?;
            let targets = resolve(&client, &base, api_key, id, None, "re-budget").await?;
            let campaign = &targets[0];

            println!(
                "{}  budget -> {}",
                describe(campaign),
                micros_to_usd(&amount_micros).bold()
            );
            if !apply {
                println!(
                    "{}",
                    "Nothing sent. Re-run with --apply to change it.".yellow()
                );
                return Ok(());
            }
            confirm_always(
                &format!(
                    "Set the budget of `{}` to {}?",
                    campaign.name,
                    micros_to_usd(&amount_micros)
                ),
                yes,
            )?;
            let update = UpdateCampaign {
                status: None,
                name: None,
                budget: Some(UpdateBudget { amount_micros }),
            };
            let updated =
                api::update_campaign(&client, &base, api_key, &campaign.id, &update).await?;
            println!("{} {}", "updated".green(), updated.id);
            println!(
                "{}",
                "A decrease lands at the next midnight in the account's time zone.".dimmed()
            );
        }

        Command::Rename {
            id,
            name,
            apply,
            yes,
        } => {
            let campaign = api::get_campaign(&client, &base, api_key, &id).await?;
            println!("{}  name -> {}", describe(&campaign), name.bold());
            if !apply {
                println!(
                    "{}",
                    "Nothing sent. Re-run with --apply to rename it.".yellow()
                );
                return Ok(());
            }
            confirm_always(&format!("Rename `{}` to `{name}`?", campaign.name), yes)?;
            let update = UpdateCampaign {
                status: None,
                name: Some(name),
                budget: None,
            };
            let updated = api::update_campaign(&client, &base, api_key, &id, &update).await?;
            println!("{} {}", "renamed".green(), updated.name);
        }

        Command::Creatives { archived } => {
            let creatives = api::list_creatives(&client, &base, api_key, archived).await?;
            if creatives.is_empty() {
                println!("No creatives.");
                return Ok(());
            }
            for creative in &creatives {
                println!(
                    "{:<24} {:<10} {:>5}x{:<5} {:<16} {}",
                    creative.asset_id,
                    if creative.is_archived {
                        "archived"
                    } else {
                        "live"
                    },
                    creative.width,
                    creative.height,
                    creative.moderation_status,
                    creative.asset_name
                );
            }
        }

        Command::Universes => {
            let list = api::list_advertisable_universes(&client, &base, api_key).await?;
            if list.advertisable_universes.is_empty() {
                println!("No experience on this account may be advertised.");
                return Ok(());
            }
            for universe in &list.advertisable_universes {
                println!("{}", universe.universe_id);
            }
        }

        Command::Accounts => {
            let list = api::list_billing_accounts(&client, &base, api_key).await?;
            if list.billing_accounts.is_empty() {
                println!("This key can spend from no billing account.");
                return Ok(());
            }
            for account in &list.billing_accounts {
                println!(
                    "{:<24} {:<10} {:<14} {}",
                    account.id, account.status, account.account_type, account.display_name
                );
            }
        }

        Command::Options => {
            let universe_id = global.single_universe()?;
            let options = api::campaign_options(&client, &base, api_key, universe_id).await?;

            if let Some(eligibility) = &options.eligibility {
                if eligibility.eligible {
                    println!("{} universe {universe_id}", "eligible".green());
                } else {
                    println!("{} universe {universe_id}", "not eligible".red());
                    for reason in &eligibility.reasons {
                        println!("  {}", reason.yellow());
                    }
                }
            }
            println!("formats");
            for format in &options.ad_formats {
                println!("  {:<20} {}x{}", format.format, format.width, format.height);
            }
            println!("objectives    {}", options.objectives.join(", "));
            println!("payment       {}", options.payment_types.join(", "));
            if let Some(dimensions) = &options.targeting_dimensions {
                println!("ages          {}", dimensions.age_groups.join(", "));
                println!("devices       {}", dimensions.devices.join(", "));
                println!("countries     {} available", dimensions.countries.len());
            }
        }
    }

    Ok(())
}

/// Work out which campaigns a command means, from an id, a name prefix, or a
/// question asked on the terminal.
///
/// Whichever route it took, the caller gets whole campaigns rather than ids, so
/// what it prints and asks about is the campaign's name and state. An id alone
/// cannot be checked by the person confirming it.
async fn resolve(
    client: &reqwest::Client,
    base: &ApiBase,
    api_key: &str,
    id: Option<String>,
    name: Option<String>,
    verb: &str,
) -> Result<Vec<model::Campaign>> {
    if let Some(id) = id {
        return Ok(vec![api::get_campaign(client, base, api_key, &id).await?]);
    }

    let campaigns = api::list_campaigns(client, base, api_key).await?;

    if let Some(prefix) = name {
        let matched: Vec<model::Campaign> = campaigns
            .into_iter()
            .filter(|c| c.name.starts_with(&prefix) && c.status != "CANCELLED")
            .collect();
        if matched.is_empty() {
            bail!("no campaign is named `{prefix}...`, or they are all cancelled already");
        }
        return Ok(matched);
    }

    if !WRITE_FORMAT.may_prompt() {
        bail!(
            "which campaign should I {verb}? Pass an id, or --name to take a whole launch group. \
             There is no terminal here to ask on"
        );
    }

    let choices: Vec<model::Campaign> = campaigns
        .into_iter()
        .filter(|c| c.status != "CANCELLED")
        .collect();
    if choices.is_empty() {
        bail!("no campaign on this account can be {verb}d");
    }

    let labels: Vec<String> = choices.iter().map(describe).collect();
    let chosen = dialoguer::Select::new()
        .with_prompt(format!("Which campaign should I {verb}?"))
        .items(&labels)
        .default(0)
        .interact()?;

    Ok(vec![choices.into_iter().nth(chosen).expect("in range")])
}

/// Ask which images to test, rather than making the caller copy asset ids out
/// of `ads creatives`.
///
/// Only reached when `--creative` was omitted, and only on a terminal: a
/// script that forgets the flag gets an error, not a prompt nobody answers.
async fn pick_creatives(
    client: &reqwest::Client,
    base: &ApiBase,
    api_key: &str,
) -> Result<Vec<String>> {
    if !WRITE_FORMAT.may_prompt() {
        bail!("no creative given. Pass --creative <ASSET_ID> once per variant");
    }

    let creatives = api::list_creatives(client, base, api_key, false).await?;
    if creatives.is_empty() {
        bail!("this account has no creative to test. Upload the images first");
    }

    let labels: Vec<String> = creatives
        .iter()
        .map(|c| {
            format!(
                "{} · {}x{} · {} · {}",
                c.asset_id, c.width, c.height, c.moderation_status, c.asset_name
            )
        })
        .collect();

    let chosen = dialoguer::MultiSelect::new()
        .with_prompt("Which images should this test compare? (space to pick, enter to confirm)")
        .items(&labels)
        .interact()?;

    if chosen.len() < 2 {
        bail!("a test compares at least two images; pick another, or pass --creative twice");
    }

    Ok(chosen
        .into_iter()
        .map(|i| creatives[i].asset_id.clone())
        .collect())
}

async fn set_status(
    client: &reqwest::Client,
    base: &ApiBase,
    api_key: &str,
    targets: &[model::Campaign],
    status: &str,
    apply: bool,
    yes: bool,
) -> Result<()> {
    for campaign in targets {
        println!("{}  -> {}", describe(campaign), status.bold());
    }
    if !apply {
        println!(
            "{}",
            "Nothing sent. Re-run with --apply to perform it.".yellow()
        );
        return Ok(());
    }

    let prompt = match targets {
        [one] => format!("Set `{}` to {status}?", one.name),
        many => format!("Set {} campaigns to {status}?", many.len()),
    };
    confirm_always(&prompt, yes)?;

    let update = UpdateCampaign {
        status: Some(status.to_owned()),
        name: None,
        budget: None,
    };
    for campaign in targets {
        let updated = api::update_campaign(client, base, api_key, &campaign.id, &update).await?;
        println!("{} {} {}", "updated".green(), updated.id, updated.status);
    }
    Ok(())
}

/// A key derived from the campaign itself, so a retry and a re-run of the same
/// command both resolve to the campaign that already exists.
///
/// Roblox requires `x-idempotency-key` on create, and `execute_with_retry`
/// resends on a 429 or a timeout. A random key would make each resend a fresh
/// campaign with a fresh budget, which is the one mistake here that costs real
/// money. Deriving it from the request body also means running `launch` twice
/// by accident is free.
///
/// FNV-1a rather than a hashing crate: it is eight lines, and its output has to
/// stay identical across releases for any of the above to hold, which
/// `DefaultHasher` does not promise.
fn idempotency_key(campaign: &CreateCampaign) -> String {
    let body = serde_json::to_string(campaign).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("rbx-ads-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campaign(name: &str, asset: &str) -> CreateCampaign {
        CreateCampaign {
            name: name.into(),
            target_universe_id: "1".into(),
            creative_asset_ids: vec![asset.into()],
            objective: OBJECTIVE,
            payment_type: "CREDIT_CARD".into(),
            budget: Budget {
                amount_micros: "25000000".into(),
                budget_type: "DAILY".into(),
            },
            schedule: Schedule {
                start_time: None,
                duration_in_days: Some(7),
            },
            bid: Bid {
                strategy: BID_STRATEGY,
            },
            targeting: Targeting::default(),
        }
    }

    #[test]
    fn the_same_campaign_always_gets_the_same_key() {
        assert_eq!(
            idempotency_key(&campaign("icon test [1]", "1")),
            idempotency_key(&campaign("icon test [1]", "1"))
        );
    }

    #[test]
    fn a_different_creative_gets_a_different_key() {
        assert_ne!(
            idempotency_key(&campaign("icon test [1]", "1")),
            idempotency_key(&campaign("icon test [2]", "2"))
        );
    }

    #[test]
    fn the_key_is_stable_across_releases() {
        // Pinned on purpose. If this value changes, a re-run of `launch` stops
        // matching the campaign it created before and buys a second one.
        assert_eq!(
            idempotency_key(&campaign("icon test [1]", "1")),
            "rbx-ads-e6f3ef68085d70f6"
        );
    }
}
