//! `rbx rtbf init`: write a commented `rbxrtbf.toml` to start from.

use anyhow::{bail, Result};
use colored::Colorize;

use super::RtbfCtx;
use crate::config;

pub fn run(ctx: &RtbfCtx<'_>) -> Result<()> {
    // Refused rather than merged or overwritten. A template file is the sort of
    // thing somebody runs `init` on twice by accident, and the second run would
    // otherwise replace the declarations they had just written.
    if ctx.config.exists() {
        bail!(
            "{} already exists. Delete it first, or --config to write elsewhere.",
            ctx.config.display()
        );
    }

    std::fs::write(&ctx.config, config::TEMPLATE)?;
    println!(
        "{} {}",
        "Wrote".green().bold(),
        ctx.config.display().to_string().cyan()
    );
    println!();
    println!("Every example is commented out, so this declares nothing yet.");
    println!("  1. uncomment and edit the templates that match your data stores");
    println!("  2. `rbx rtbf verify --env <name>` to check they match something real");
    println!("  3. `rbx rtbf sync --env <name>` to publish them");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_core::GlobalFlags;

    fn flags(dir: &std::path::Path) -> GlobalFlags {
        GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: None,
            place: None,
            places: dir.join("rbxplace.toml"),
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    #[test]
    fn init_writes_a_file_that_parses_and_declares_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(config::FILE);
        let global = flags(dir.path());
        let ctx = RtbfCtx {
            global: &global,
            config: path.clone(),
            base_url: None,
        };

        run(&ctx).expect("init");
        assert!(config::load(&path).expect("parses").templates.is_empty());
    }

    /// The second `init` must not eat the first one's work.
    #[test]
    fn init_refuses_to_overwrite_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(config::FILE);
        std::fs::write(&path, "[[key]]\nstore = \"A\"\npattern = \"U_{UserId}\"\n").unwrap();
        let global = flags(dir.path());
        let ctx = RtbfCtx {
            global: &global,
            config: path.clone(),
            base_url: None,
        };

        let err = run(&ctx).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        // And the declaration is still there.
        assert_eq!(config::load(&path).unwrap().templates.keys.len(), 1);
    }
}
