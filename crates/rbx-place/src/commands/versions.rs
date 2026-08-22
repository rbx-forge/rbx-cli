use anyhow::Result;
use colored::Colorize;

use crate::config::PlacesConfig;
use crate::json::VersionsDocument;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use super::make_client;

pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    env: &str,
    place: Option<&str>,
    count: usize,
    filter: &str,
    json: bool,
) -> Result<()> {
    let format = OutputFormat::from_json_flag(json);
    let client = make_client(global, base_url)?;

    // `--place-id` skips rbxplace.toml. See download.rs: the place has no name
    // then, so the id stands in for one in the header and in the document.
    let (place_name, place_id) = match global.place_id.as_slice() {
        [] => {
            let config = PlacesConfig::load(&global.places)?;
            let env_config = config.get_env(env)?;
            env_config.resolve_place(place)?
        }
        _ => {
            let id = global.single_place()?;
            (id.to_string(), id)
        }
    };

    // The header is half a line: the count that completes it is only known
    // after the call. Under `--json` neither half is printed, because the
    // document is what stdout carries.
    if !format.is_json() {
        print!(
            "Versions for {}/{} ({}) ... ",
            env.bold(),
            place_name.bold(),
            place_id
        );
    }
    let versions = match filter {
        "published" => client.list_versions_filtered(place_id, count, true).await?,
        "saved" => {
            client
                .list_versions_filtered(place_id, count, false)
                .await?
        }
        _ => client.list_versions(place_id, count).await?,
    };

    if format.is_json() {
        // Ahead of the "(no versions)" branch below: an empty list is a fact a
        // consumer reads a zero off, not a line to print.
        return output::emit(&VersionsDocument::new(
            env,
            &place_name,
            place_id,
            filter,
            count,
            &versions,
        ));
    }

    println!("{}", format!("{} found", versions.len()).green());
    println!();

    if versions.is_empty() {
        // Which filter was in force decides what the emptiness means, and the
        // difference matters: "this place has nothing" and "nothing is
        // published yet" send you to different places. A place with only
        // drafts answers empty under `published` and is not empty at all.
        match filter {
            "published" => {
                println!("  (no published versions: try --filter saved, or --filter all)")
            }
            "saved" => println!("  (no saved versions: try --filter published, or --filter all)"),
            _ => println!("  (no versions)"),
        }
        return Ok(());
    }

    for v in &versions {
        let tag = if v.published {
            format!("  {}", "[published]".cyan())
        } else {
            String::new()
        };
        println!(
            "  v{:<6}{}{}",
            v.version_number,
            tag,
            format!("  {}", v.display_time()).dimmed()
        );
    }

    Ok(())
}
