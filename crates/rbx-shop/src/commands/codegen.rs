//! Regenerate — or verify — the codegen folder, offline.
//!
//! The generated modules are a pure function of `rbxshop.toml` and
//! `rbxshop.lock`, both of which are committed. Nothing here contacts Roblox,
//! which is what makes it usable from a pre-commit hook and from CD, and what
//! makes `--check` an answerable question: does the committed folder still
//! match the inputs it was derived from?
//!
//! `sync` runs the same generation at the end of a successful run. This
//! command exists so regenerating does not require credentials — otherwise a
//! `--check` failure would be unfixable by anyone without an API key, which
//! is the fastest way to get a hook disabled.

use std::path::Path;

use anyhow::{bail, Result};
use colored::Colorize;

use rbx_core::generated::CheckReport;

use crate::codegen;
use crate::config::Config;
use crate::ctx::ShopCtx;
use crate::lockfile::Lockfile;

pub fn run(ctx: &ShopCtx<'_>, check: bool) -> Result<()> {
    let config = Config::load_merged(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new("."));
    let lockfile_path = config_dir.join(crate::lockfile::LOCKFILE_NAME);

    if config.codegen.output.is_none() {
        bail!(
            "Code generation is disabled: [codegen].output is not set in {}. \
             Set it to the folder that should hold the generated modules, \
             e.g. output = \"src/shared/GameIds\".",
            ctx.config.display()
        );
    }

    if !lockfile_path.exists() {
        bail!(
            "No lockfile at {}. Code generation reads asset ids from it, so run \
             `rbx shop sync` once to create it.",
            lockfile_path.display()
        );
    }
    let lockfile = Lockfile::load(&lockfile_path)?;

    let Some(plan) = codegen::plan(&config, &lockfile, config_dir)? else {
        bail!(
            "{} has no envs yet, so there is nothing to generate. Run `rbx shop sync` first.",
            lockfile_path.display()
        );
    };

    if !check {
        return codegen::generate(&config, &lockfile, config_dir);
    }

    let mut report = CheckReport::new();
    for file in &plan.files {
        report.check(file)?;
    }
    // A leftover module from a deleted env is drift too: it still looks
    // generated, and game code can still require it.
    for path in &plan.stale {
        report.stale(path);
    }

    println!(
        "{} {} ({} env{})\n",
        "codegen:".bold(),
        plan.dir.display(),
        plan.env_count,
        if plan.env_count == 1 { "" } else { "s" }
    );

    report.finish(
        &format!("{} + {}", ctx.config.display(), lockfile_path.display()),
        "rbx shop codegen",
    )
}
