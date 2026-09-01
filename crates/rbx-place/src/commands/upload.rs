use std::path::Path;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use colored::Colorize;

use crate::config::{Environment, PlacesConfig};
use crate::json::{MultiEnvWriteDocument, WriteCommand, WriteDocument};
use rbx_core::confirm::confirm_destructive;
use rbx_core::output::{self, OutputFormat};
use rbx_core::{EnvTarget, GlobalFlags};

use super::{cannot_ask, make_client, upload_and_classify, Landing};

/// One env's share of the run: its config, and the places the flags resolve to
/// inside it.
///
/// Built for every target env before the first byte goes out, which is what
/// lets the confirmation below name every env it is about to write to. It also
/// turns a `--place` name that the third env does not declare into a refusal,
/// rather than into a run that wrote two envs and then stopped.
struct EnvPlan<'a> {
    name: &'a str,
    config: &'a Environment,
    places: Vec<(String, u64)>,
}

impl EnvPlan<'_> {
    /// The place names, in the order they will be written.
    fn place_names(&self) -> Vec<&str> {
        self.places.iter().map(|(name, _)| name.as_str()).collect()
    }
}

// One more argument than clippy's threshold, and the same reasoning the other
// commands here carry: every one of these maps 1:1 onto a clap arg in lib.rs,
// so a struct would hide the CLI shape without making the call site clearer.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    envs: &[EnvTarget],
    place: Option<&str>,
    all_places: bool,
    file: &Path,
    published: bool,
    yes: bool,
    json: bool,
) -> Result<()> {
    let format = OutputFormat::from_json_flag(json);
    let config = PlacesConfig::load(&global.places)?;

    // Every env is resolved here, before the client exists and before a byte
    // is read, for the reason `rbx shop sync` resolves its list up front: a
    // fan-out that discovers a bad env halfway has already written to the good
    // ones, and there is nothing useful to do with that.
    let mut plan: Vec<EnvPlan<'_>> = Vec::with_capacity(envs.len());
    for target in envs {
        let env_config = config.get_env(&target.name)?;
        let places = if all_places {
            env_config.all_places_sorted()
        } else {
            vec![env_config.resolve_place(place)?]
        };
        plan.push(EnvPlan {
            name: &target.name,
            config: env_config,
            places,
        });
    }
    // Unreachable from the CLI, where `--env` is a required arg: this is for a
    // caller that resolved to nothing, so it gets a refusal rather than a run
    // that uploads nowhere and reports success.
    let Some(only) = plan.first() else {
        bail!("no env to upload to. Pass --env <name>, --env <group>, or --env all.")
    };
    // Drives both the `env:` headers and the shape of the `--json` document, so
    // a one-env run is indistinguishable from what it was before fan-out.
    let plural = plan.len() > 1;

    let client = make_client(global, base_url)?;

    // `Bytes`, not `Vec<u8>`: the file is read once and every place in every
    // env (and every retry inside each upload) shares that one buffer.
    let data = Bytes::from(
        std::fs::read(file).with_context(|| format!("Failed to read file: {}", file.display()))?,
    );
    let size_kb = data.len() as f64 / 1024.0;
    let version_type = if published { "published" } else { "saved" };

    // One env keeps the order it has always had: the details first, then a
    // question about what the reader has just seen. A fan-out cannot do that,
    // because its question covers envs whose details have not been printed yet,
    // so it names them in the prompt instead and prints each env's details as
    // it reaches it.
    if !plural {
        print_plan(format, file, size_kb, only, version_type);
    }

    // Asked once for the whole run, not per env, and gated on whether ANY
    // target env carries `confirm = true`. Per env, the prompt would be reached
    // mid-loop, after writes had already landed somewhere else, which is too
    // late for a "no" to mean anything. `rbx shop sync` records the same
    // reasoning.
    let requires_confirm = plan.iter().any(|env| env.config.confirm);

    // The confirmation is a question, so it has to be refused before it is
    // asked when nothing can answer it. Nothing has been written at this point,
    // so this failure leaves stdout empty rather than emitting a receipt for a
    // run that never happened.
    if requires_confirm && !yes && !format.may_prompt() {
        return Err(cannot_ask(format, "for confirmation", "--yes"));
    }
    confirm_destructive(&confirm_prompt(&plan, published), requires_confirm, yes)?;

    let mut receipts: Vec<WriteDocument> = Vec::with_capacity(plan.len());
    let mut failure: Option<anyhow::Error> = None;
    // What the closing line is allowed to claim. A run where every target
    // already held the file is not a failure, and it is not `Upload complete.`
    // either; but saying so requires having checked every one of them, so an
    // unreadable place keeps the run on the wording that claims nothing.
    let mut created_somewhere = false;
    let mut unverified_somewhere = false;

    'envs: for env in &plan {
        if plural {
            print_env_header(format, env.name);
            print_plan(format, file, size_kb, env, version_type);
        }

        let mut receipt = WriteDocument::new(
            WriteCommand::Upload,
            env.name,
            env.config.universe_id,
            published,
            !all_places,
        );

        for (place_name, place_id) in &env.places {
            if !format.is_json() {
                print!("  {} ({}) ... ", place_name.bold(), place_id);
            }
            match upload_and_classify(
                &client,
                env.config.universe_id,
                *place_id,
                data.clone(),
                published,
            )
            .await
            {
                Ok((version, landing)) => {
                    if !format.is_json() {
                        println!(
                            "{}{}",
                            format!("v{}", version).green(),
                            landing.note().unwrap_or_default()
                        );
                    }
                    match landing {
                        Landing::Created => created_somewhere = true,
                        Landing::Unknown => unverified_somewhere = true,
                        Landing::Unchanged => {}
                    }
                    receipt.landed(place_name, *place_id, version, landing.created());
                }
                Err(e) => {
                    // The places already uploaded to have new versions whatever
                    // happens to this one, and so does every env already
                    // walked. The receipts go out reporting them before the
                    // error propagates. The process still exits non-zero, and
                    // the document says `"ok": false`.
                    if !format.is_json() {
                        println!("{}", "failed".red());
                    }
                    receipts.push(receipt.failed(&e));
                    failure = Some(e);
                    break 'envs;
                }
            }
        }
        receipts.push(receipt);
    }

    if format.is_json() {
        emit_receipts(receipts, plural)?;
    }
    if let Some(error) = failure {
        return Err(error);
    }
    if !format.is_json() {
        println!();
        if created_somewhere || unverified_somewhere {
            println!("{}", "Upload complete.".green());
        } else {
            // Not an error: the places hold the file that was asked for, which
            // is the state the command exists to reach. Saying `Upload
            // complete.` here is the part that misleads, because it is what a
            // reader takes as proof their build went out.
            println!(
                "{}",
                "Nothing to upload: every place already holds this file.".yellow()
            );
        }
    }
    Ok(())
}

/// Which env the lines that follow belong to.
///
/// Printed only when there is more than one target, so a single-env run's
/// output is byte for byte what it was before this command could fan out.
fn print_env_header(format: OutputFormat, env: &str) {
    if format.is_json() {
        return;
    }
    println!();
    println!("{} {}", "env:".bold(), env.bold());
}

/// What is about to be written to one env.
fn print_plan(
    format: OutputFormat,
    file: &Path,
    size_kb: f64,
    env: &EnvPlan<'_>,
    version_type: &str,
) {
    if format.is_json() {
        return;
    }
    println!(
        "Uploading {} ({:.1} KB) → {} [{}]",
        file.display(),
        size_kb,
        env.place_names().join(", "),
        env.name.bold()
    );
    println!("  Universe: {}", env.config.universe_id);
    println!("  Version type: {}", version_type);
    println!();
}

/// The one question the run asks.
///
/// One env keeps the wording it has always had, naming its places. Several envs
/// name the envs instead: the places are per env, and a prompt listing every
/// place across every env is a sentence nobody finishes reading before typing
/// y.
fn confirm_prompt(plan: &[EnvPlan<'_>], published: bool) -> String {
    let (verb, state) = if published {
        ("publish", "live")
    } else {
        ("save as", "draft")
    };
    match plan {
        [one] => format!(
            "Upload to {} ({})? This will {} {}.",
            one.name,
            one.place_names().join(", "),
            verb,
            state
        ),
        several => format!(
            "Apply rbx place upload to env(s): [{}]? This will {} {}.",
            several
                .iter()
                .map(|env| env.name)
                .collect::<Vec<_>>()
                .join(", "),
            verb,
            state
        ),
    }
}

/// One JSON document on stdout, in the shape the invocation calls for.
///
/// A single env emits the bare receipt, unchanged; several emit the envelope
/// that holds one receipt per env. See [`MultiEnvWriteDocument`].
fn emit_receipts(receipts: Vec<WriteDocument>, plural: bool) -> Result<()> {
    if !plural {
        if let [one] = receipts.as_slice() {
            return output::emit(one);
        }
    }
    output::emit(&MultiEnvWriteDocument::new(WriteCommand::Upload, receipts))
}
