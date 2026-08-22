//! Writes `schemas/*.json` from the serde models that read the TOML config
//! files, so editors can validate and autocomplete them.
//!
//! # Why a separate binary
//!
//! The config files are the product surface of an IaC tool, and until now they
//! got no editor support at all. A JSON Schema per file, matched by taplo or
//! Even Better TOML, gives every adopter inline validation the moment they
//! open one.
//!
//! This is a dev tool, not a subcommand of `rbx`. `schemars` and its derive
//! machinery would otherwise ride along in a binary that ships to users and
//! never calls them, on a project whose release profile is tuned for size. So
//! the models carry `JsonSchema` behind an off-by-default `schema` feature,
//! and this crate (built from source, never released) is the only thing that
//! turns it on.
//!
//! # Why derive rather than hand-write
//!
//! A hand-written schema is a second description of the same file format, and
//! the two drift the moment somebody adds a field. Deriving means the structs
//! are the single source of truth, and their doc comments become the hover
//! text for free.
//!
//! # The rule that matters
//!
//! **The schema must not be stricter than the tool.** Unknown keys are warned
//! about and ignored, never rejected (see `docs/env.md`), so no table here may
//! close `additionalProperties`. An editor painting a valid file red is worse
//! than no schema at all: it teaches people to ignore the squiggles, and then
//! the real errors go unread too. `schemas.rs` has the test that holds this.

mod engine_avatar;
mod schemas;

use anyhow::{Context, Result};
use clap::Parser;

use rbx_core::generated::{CheckReport, GeneratedFile};

#[derive(Parser, Debug)]
#[command(
    name = "rbx-schema",
    about = "Generate the JSON Schemas for the rbx TOML config files",
    long_about = "Writes one JSON Schema per config file into schemas/.\n\n\
                  Run with --check in CI to fail when the committed schemas no \
                  longer match the models. Exits 2 on drift, matching \
                  `rbx env gen-module --check`."
)]
struct Cli {
    /// Compare against the committed schemas instead of writing them.
    ///
    /// Exits 2 if any file is missing or stale, so a CI job can tell
    /// "regenerate and commit" from "the command blew up".
    #[arg(long)]
    check: bool,

    /// Directory the schemas live in, relative to the working directory.
    #[arg(long, default_value = "schemas")]
    out_dir: std::path::PathBuf,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:?}");
            if err
                .chain()
                .any(|cause| cause.is::<rbx_core::generated::Drift>())
            {
                std::process::ExitCode::from(rbx_core::generated::DRIFT_EXIT_CODE)
            } else {
                std::process::ExitCode::FAILURE
            }
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    let files: Vec<GeneratedFile> = schemas::all()
        .into_iter()
        .map(|schema| {
            let json = serde_json::to_string_pretty(&schema.body)
                .with_context(|| format!("serialising the schema for {}", schema.config_file))?;
            // Trailing newline: every other generated file in this repo has
            // one, and a file without it shows up as "\ No newline at end of
            // file" in every diff that ever touches it.
            Ok(GeneratedFile::new(
                cli.out_dir.join(schema.file_name),
                format!("{json}\n"),
            ))
        })
        .collect::<Result<_>>()?;

    if cli.check {
        let mut report = CheckReport::new();
        for file in &files {
            report.check(file)?;
        }
        report.finish("the serde config models", "cargo run -p rbx-schema")
    } else {
        for file in &files {
            file.write()?;
            println!("{}", file.path.display());
        }
        Ok(())
    }
}
