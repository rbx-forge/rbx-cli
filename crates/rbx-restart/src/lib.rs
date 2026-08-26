//! `rbx-ops restart` : push a published version out to servers still running
//! the old one, without waiting for them to cycle on their own.
//!
//! Publishing does not restart anything. Servers already up keep running the
//! code they started with until they empty out, which on a busy experience can
//! take hours. That is fine for a feature and not fine for a fix.
//!
//! The dry run here is unusual and worth knowing about: it is not a simulation.
//! Roblox has a forecast endpoint that answers how many real players would be
//! kicked right now, so `restart launch` without `--apply` shows that number
//! and stops. You decide against a fact.

pub mod model;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use reqwest::Client;

use rbx_core::api::{
    build_client, execute_json, execute_with_retry, explain_missing_scope, require_api_key, ApiBase,
};
use rbx_core::confirm::confirm_always;
use rbx_core::GlobalFlags;

use crate::model::{
    attributes_from_pairs, validate_attributes, validate_bleed_off, Forecast, RestartLaunched,
    RestartRequest, RestartStatuses,
};

/// Roblox's own default is unstated, so one is chosen here rather than left to
/// chance. Thirty minutes empties most servers on a fast-cycling experience
/// without making a hotfix feel indefinite.
const DEFAULT_BLEED_OFF_MINUTES: i32 = 30;

#[derive(Args, Debug)]
pub struct RestartCli {
    #[command(subcommand)]
    command: Command,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

impl RestartCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show what a restart would cost right now
    ///
    /// Read-only. Roblox counts the players and instances that would actually
    /// be closed, which is not the same as everyone currently playing: a server
    /// already on the newest version is left alone.
    Forecast,

    /// Show restarts already in flight
    Status,

    /// Restart servers running an older version
    Launch {
        /// Minutes before servers begin closing.
        ///
        /// During this window players stop being matchmade to the servers due
        /// for restart, so most leave on their own and are never kicked. Longer
        /// is gentler. Roblox accepts 1 to 240.
        #[arg(long, default_value_t = DEFAULT_BLEED_OFF_MINUTES)]
        bleed_off: i32,

        /// Actually launch it. Without this you get the forecast and nothing
        /// else happens.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,

        /// One attribute handed to the game servers, `key=value`. Repeatable.
        ///
        /// Servers scheduled to close fire
        /// `game.ServerRestartScheduled(restartTime, source, attributes)`, and
        /// these are its third argument: a reason, an urgency, a line to show
        /// players. Without any, that table arrives empty.
        ///
        /// Values are sent as strings. For a number, a boolean or nesting, use
        /// `--payload`.
        #[arg(
            long = "attribute",
            value_name = "KEY=VALUE",
            conflicts_with = "payload"
        )]
        attributes: Vec<String>,

        /// The whole attributes object as JSON.
        ///
        /// Parsed here, the way `rbx message publish --payload` is, so a
        /// malformed body fails locally rather than as a 400 from inside a
        /// deploy. Must be a JSON object, at most 500 bytes serialised.
        #[arg(long)]
        payload: Option<String>,
    },
}

struct Api {
    client: Client,
    base: ApiBase,
    api_key: String,
    universe_id: u64,
}

impl Api {
    async fn forecast(&self) -> Result<Forecast> {
        let url = self.base.join(&format!(
            "/server-management/v1/universes/{}/restarts:forecast",
            self.universe_id
        ));
        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    async fn statuses(&self) -> Result<RestartStatuses> {
        let url = self.base.join(&format!(
            "/server-management/v1/universes/{}/restarts",
            self.universe_id
        ));
        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    async fn launch(&self, body: &RestartRequest) -> Result<RestartLaunched> {
        let url = self.base.join(&format!(
            "/server-management/v1/universes/{}/restarts",
            self.universe_id
        ));
        let response = execute_with_retry(|| {
            let request = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .json(body);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;

        let text = response.text().await?;
        serde_json::from_str(&text).with_context(|| format!("parsing the restart response: {text}"))
    }
}

pub async fn run(cli: RestartCli, global: &GlobalFlags) -> Result<()> {
    if global.env.as_deref() == Some("all") {
        bail!(
            "`--env all` is refused here. Restarting every environment at once would close \
             production servers because a glob matched. Name one env."
        );
    }

    let universe_id = global.single_universe()?;

    let api = Api {
        client: build_client(),
        base: match &cli.base_url {
            Some(url) => ApiBase::new(url.clone()),
            None => ApiBase::default(),
        },
        api_key: require_api_key(global.api_key.as_deref())?.to_string(),
        universe_id,
    };

    match cli.command {
        Command::Forecast => {
            print_forecast(&api.forecast().await?);
            Ok(())
        }

        Command::Status => {
            let statuses = api.statuses().await?;
            if statuses.restart_statuses.is_empty() {
                println!("{}", "No restart in flight.".dimmed());
                return Ok(());
            }
            for (id, status) in &statuses.restart_statuses {
                println!("{}", format!("restart {id}").bold());
                if let Some(scheduled) = &status.scheduled_time {
                    println!("  asked for      {scheduled}");
                }
                if let Some(start) = &status.start_time {
                    println!("  closing from   {start}");
                }
                for (place, place_status) in &status.place_restart_statuses {
                    let state = place_status.state.as_deref().unwrap_or("?");
                    let coloured = match state {
                        // The bleed-off: nothing has been closed yet.
                        "DELAYING" => state.yellow(),
                        "RESTARTING" => state.red().bold(),
                        "SUCCEEDED" => state.green(),
                        _ => state.normal(),
                    };
                    println!("  place {place:<18} {coloured}");
                }
                println!();
            }
            Ok(())
        }

        Command::Launch {
            bleed_off,
            apply,
            yes,
            attributes,
            payload,
        } => {
            validate_bleed_off(bleed_off)?;

            // Resolved before a single request goes out. A malformed payload is
            // the caller's typo, and finding out should not cost a round trip
            // (nor, on the `--apply` path, happen after the confirmation).
            // `validate_attributes` hands back the exact text it measured, so
            // the dry-run line below prints the bytes that were checked rather
            // than a second serialisation of the same value.
            let attributes = match (&payload, attributes.is_empty()) {
                (Some(raw), _) => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(raw).context("--payload must be valid JSON")?;
                    let text = validate_attributes(&parsed)?;
                    Some((parsed, text))
                }
                (None, false) => {
                    let built = attributes_from_pairs(&attributes)?;
                    let text = validate_attributes(&built)?;
                    Some((built, text))
                }
                (None, true) => None,
            };

            // Always fetched, whether or not this run will do anything. The
            // forecast is the whole basis for deciding, and it is Roblox's
            // number rather than one computed here.
            let forecast = api.forecast().await?;
            print_forecast(&forecast);
            println!();

            if forecast.is_noop() {
                println!(
                    "{}",
                    "Every server is already on the newest version. Nothing to restart.".green()
                );
                return Ok(());
            }

            // Printed on both paths. The forecast says how much a restart
            // costs and nothing about what the servers will be told, and the
            // dry run is where somebody checks the second half.
            if let Some((_, text)) = &attributes {
                println!("attributes: {text}");
                println!();
            }

            if !apply {
                println!(
                    "{}",
                    format!(
                        "Nothing sent. `--apply` would close {} instance(s) and disconnect \
                         up to {} player(s), after a {bleed_off} minute bleed-off during which \
                         most of them leave on their own.",
                        forecast.total_instances_impacted(),
                        forecast.total_players_impacted()
                    )
                    .yellow()
                );
                return Ok(());
            }

            confirm_always(
                &format!(
                    "Restart universe {universe_id}? Up to {} player(s) disconnected after a \
                     {bleed_off} minute bleed-off.",
                    forecast.total_players_impacted()
                ),
                yes,
            )?;

            let launched = api
                .launch(&RestartRequest {
                    bleed_off_duration_minutes: Some(bleed_off),
                    attributes: attributes.map(|(value, _text)| value),
                })
                .await?;

            println!(
                "{} restart {} scheduled: {} instance(s), {} player(s) impacted",
                "done".green().bold(),
                launched.id.as_deref().unwrap_or("(no id returned)"),
                launched.instances_impacted,
                launched.players_impacted
            );
            println!(
                "{}",
                "Servers begin closing when the bleed-off ends. Watch it with \
                 `rbx-ops restart status`."
                    .dimmed()
            );
            Ok(())
        }
    }
}

fn print_forecast(forecast: &Forecast) {
    if forecast.place_forecasts.is_empty() {
        println!("{}", "Nothing running. No servers to restart.".dimmed());
        return;
    }

    println!(
        "{:<20}  {:>16}  {:>18}  {}",
        "PLACE".bold(),
        "PLAYERS HIT".bold(),
        "INSTANCES HIT".bold(),
        "NEWEST VERSION".bold()
    );
    for (place, p) in &forecast.place_forecasts {
        println!(
            "{place:<20}  {:>16}  {:>18}  {}",
            format!("{}/{}", p.players_impacted, p.total_players),
            format!("{}/{}", p.instances_impacted, p.total_instances),
            p.latest_place_version.as_deref().unwrap_or("-")
        );
    }
    println!();
    println!(
        "{}",
        format!(
            "{} player(s) would be disconnected, {} instance(s) closed.",
            forecast.total_players_impacted(),
            forecast.total_instances_impacted()
        )
        .bold()
    );
    println!(
        "{}",
        "Hit is not total: a server already on the newest version is left alone.".dimmed()
    );
}
