use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::PlacesConfig;
use rbx_core::GlobalFlags;

use super::make_client;

// See upload.rs: one argument per clap arg, deliberately.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    env: Option<&str>,
    place: Option<&str>,
    version: Option<u64>,
    published: bool,
    saved: bool,
    out: Option<&Path>,
) -> Result<()> {
    let client = make_client(global, base_url)?;

    // `--place-id` wins over `--env` / `--place` and skips rbxplace.toml
    // entirely, so a download works against a place this project has never
    // heard of. The place has no name then, and the progress lines say the id.
    let (place_name, place_id) = match global.place_id.as_slice() {
        [] => {
            // `--env all` (or a group) would resolve several places onto the
            // one path `--out` names, so every download but the last would be
            // overwritten by the next. Refused rather than silently kept, and
            // refused through `EnvSelector` so a group is turned away wherever
            // `all` is.
            if let Some(selector) = global.env_selector()? {
                selector
                    .single("places")
                    .context("`rbx place download` writes one file, and --out names one path")?;
            }
            // clap makes this unreachable: `--env` is required unless
            // `--place-id` is present, and this is the branch where it is not.
            let Some(env) = env else {
                anyhow::bail!(
                    "no target. Pass --env <name>, or --place-id <id> to name the place directly."
                )
            };
            let config = PlacesConfig::load(&global.places)?;
            let env_config = config.get_env(env)?;
            env_config.resolve_place(place)?
        }
        _ => {
            let id = global.single_place()?;
            (id.to_string(), id)
        }
    };

    let resolved_version = if version.is_some() {
        version
    } else if published {
        let v = client
            .find_version(place_id, true)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No published version found for place {}", place_id))?;
        Some(v.version_number)
    } else if saved {
        let v = client
            .find_version(place_id, false)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No saved version found for place {}", place_id))?;
        Some(v.version_number)
    } else {
        version
    };

    let version_label = resolved_version
        .map(|v| format!("v{}", v))
        .unwrap_or_else(|| "latest".to_string());

    let output_path: PathBuf = out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(format!("{}.rbxl", place_id)));

    // `env/place (id)` when an env resolved one. Under `--place-id` there is
    // no env and the name *is* the id, so the same shape would print the
    // number twice: the id alone says everything the pair would have.
    let target = match env {
        Some(name) => format!("{}/{} ({})", name.bold(), place_name.bold(), place_id),
        None => place_id.to_string().bold().to_string(),
    };

    println!("Downloading {target} @ {version_label}");

    print!("  Getting download URL ... ");
    let url = client.get_download_url(place_id, resolved_version).await?;
    println!("{}", "ok".green());

    print!("  Downloading ... ");
    let bytes = client.download_from_url(&url).await?;
    println!(
        "{}",
        format!("{:.1} KB", bytes.len() as f64 / 1024.0).green()
    );

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&output_path, &bytes)?;

    println!();
    println!("{} → {}", "Saved".green(), output_path.display());
    Ok(())
}
