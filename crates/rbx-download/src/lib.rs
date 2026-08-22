//! Download Roblox assets by id (`rbx download`).
//!
//! Two backends:
//! - `--source public` (default): the legacy `assetdelivery.roblox.com`
//!   endpoint with an optional `.ROBLOSECURITY` cookie (auto-detected from a
//!   local Studio install via `GlobalFlags::resolve_cookie`). Broad reach.
//! - `--source cloud`: the Open Cloud `asset-delivery-api` with `--api-key`.
//!   Supports `--version <n>` to pin a specific asset version.
//!
//! Asset type → file extension is resolved from `economy.roblox.com` metadata,
//! unless `--type` is given (a numeric AssetTypeId or an alias), in which case
//! no economy lookup happens at all.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;

use rbx_core::api::ApiBase;
use rbx_core::GlobalFlags;

#[derive(Args, Debug)]
pub struct DownloadCli {
    /// Asset ids to download.
    pub ids: Vec<u64>,

    /// Read additional asset ids from a file (whitespace/comma/newline
    /// separated; lines starting with `#` are ignored).
    #[arg(short = 'f', long)]
    pub file: Option<PathBuf>,

    /// Directory to write downloaded files into.
    #[arg(short = 'o', long, default_value = "downloads")]
    pub output: PathBuf,

    /// Force the public assetdelivery backend even when an API key is set.
    /// By default the Open Cloud backend is used whenever an --api-key /
    /// RBX_API_KEY is available, otherwise the public endpoint.
    #[arg(long, conflicts_with = "version")]
    pub public: bool,

    /// Skip the economy metadata lookup by giving the asset type yourself:
    /// a numeric AssetTypeId, or an alias (image, audio, mesh, lua, place,
    /// model, animation, video, font).
    #[arg(long = "type")]
    pub type_spec: Option<String>,

    /// Pin a specific asset version (Open Cloud only; selects the cloud
    /// backend and requires an API key + exactly one id).
    #[arg(long)]
    pub version: Option<u64>,

    /// Don't dereference Animation wrappers to their KeyframeSequence.
    #[arg(long)]
    pub raw: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Legacy assetdelivery endpoint (cookie auth, broad reach).
    Public,
    /// Open Cloud asset-delivery-api (api-key auth, supports --version).
    Cloud,
}

pub async fn run(cli: DownloadCli, global: &GlobalFlags) -> Result<()> {
    let mut ids = cli.ids.clone();
    if let Some(path) = &cli.file {
        ids.extend(read_ids_file(path)?);
    }
    if ids.is_empty() {
        bail!("No asset ids given. Pass ids positionally or via --file <path>.");
    }

    // Backend selection: --public forces public; otherwise an available API
    // key (or --version) means the caller wants Open Cloud.
    let source = if cli.public {
        Source::Public
    } else if cli.version.is_some() || global.api_key.is_some() {
        Source::Cloud
    } else {
        Source::Public
    };

    if cli.version.is_some() && ids.len() != 1 {
        bail!("--version targets a single asset; pass exactly one id.");
    }

    // Resolve the type override once (shared by every id).
    let forced = cli.type_spec.as_deref().map(resolve_type).transpose()?;

    // Auth per backend.
    let client = rbx_core::api::build_client();
    let cookie_header = match source {
        Source::Public => global
            .resolve_cookie()
            .map(|c| format!(".ROBLOSECURITY={c}")),
        Source::Cloud => None,
    };
    let api_key = match source {
        Source::Cloud => Some(
            global
                .api_key
                .clone()
                .context("--cloud needs an Open Cloud key (--api-key or RBX_API_KEY).")?,
        ),
        Source::Public => None,
    };

    std::fs::create_dir_all(&cli.output)
        .with_context(|| format!("creating output dir {}", cli.output.display()))?;

    let ctx = Ctx {
        client,
        hosts: Hosts::default(),
        cookie_header,
        api_key,
        source,
        output: cli.output,
        version: cli.version,
        raw: cli.raw,
        forced,
    };

    println!("Downloading {} asset(s) via {:?}...", ids.len(), ctx.source);
    let mut ok = 0usize;
    let mut failed = 0usize;
    for id in ids {
        match download_one(&ctx, id).await {
            Ok(name) => {
                println!("  {} {}", "✓".green(), name);
                ok += 1;
            }
            Err(e) => {
                println!("  {} asset {}: {}", "✗".red(), id, e);
                failed += 1;
            }
        }
    }

    println!("\n{} {} ok, {} failed", "Done:".bold(), ok, failed);
    if failed > 0 {
        bail!("{} download(s) failed", failed);
    }
    Ok(())
}

/// The three hosts this command reaches, one field each.
///
/// Injectable so the request shaping can run against a mock server; until
/// this existed the URLs were built inline and nothing here had been tested
/// over HTTP. Named `<field>` / `const <FIELD>_HOST` because `rbx-spec-drift`
/// resolves which host a `.join(...)` reaches by looking the const up from the
/// receiver's name.
struct Hosts {
    /// `apis.roblox.com`: Open Cloud asset delivery, needs an api key.
    cloud: ApiBase,
    /// `assetdelivery.roblox.com`: the public endpoint, cookie-optional.
    delivery: ApiBase,
    /// `economy.roblox.com`: asset name and type.
    economy: ApiBase,
}

impl Default for Hosts {
    fn default() -> Self {
        Self {
            cloud: ApiBase::default(),
            delivery: ApiBase::new(DELIVERY_HOST),
            economy: ApiBase::new(ECONOMY_HOST),
        }
    }
}

const DELIVERY_HOST: &str = "https://assetdelivery.roblox.com";
const ECONOMY_HOST: &str = "https://economy.roblox.com";

struct Ctx {
    client: reqwest::Client,
    hosts: Hosts,
    cookie_header: Option<String>,
    api_key: Option<String>,
    source: Source,
    output: PathBuf,
    version: Option<u64>,
    raw: bool,
    /// `(extension, type_id)` from --type, when given.
    forced: Option<(String, Option<i64>)>,
}

async fn download_one(ctx: &Ctx, id: u64) -> Result<String> {
    // Determine name + extension + type id, fetching economy metadata only if
    // --type wasn't supplied.
    let (name, ext, type_id) = match &ctx.forced {
        Some((ext, type_id)) => (None, ext.clone(), *type_id),
        None => {
            let (name, type_id) = fetch_details(ctx, id).await?;
            let ext = type_id.map(ext_for_type_id).unwrap_or("bin").to_string();
            (name, ext, type_id)
        }
    };

    let bytes = download_bytes(ctx, id, ctx.version).await?;

    // Animation wrapper → dereference to the referenced KeyframeSequence.
    if !ctx.raw && type_id.map(is_animation_type).unwrap_or(false) && is_animation_wrapper(&bytes) {
        if let Some(kf_id) = extract_rbxassetid(&bytes) {
            println!("    ↪ animation wrapper → KeyframeSequence {}", kf_id);
            // KeyframeSequences are not versioned the same way; fetch latest.
            let kf_bytes = download_bytes(ctx, kf_id, None).await?;
            return save(ctx, id, &name, "rbxm", &kf_bytes);
        }
    }

    save(ctx, id, &name, &ext, &bytes)
}

/// GET the asset bytes from the selected backend. Errors on HTTP failure or a
/// JSON error body (the public endpoint returns 200 + JSON on some failures).
async fn download_bytes(ctx: &Ctx, id: u64, version: Option<u64>) -> Result<Vec<u8>> {
    match ctx.source {
        Source::Cloud => {
            let key = ctx.api_key.as_deref().expect("cloud source has api key");
            let cloud = &ctx.hosts.cloud;
            let url = match version {
                Some(v) => cloud.join(&format!("/asset-delivery-api/v1/assetId/{id}/version/{v}")),
                None => cloud.join(&format!("/asset-delivery-api/v1/assetId/{id}")),
            };
            let resp = ctx.client.get(&url).header("x-api-key", key).send().await?;
            if !resp.status().is_success() {
                let s = resp.status();
                bail!(
                    "asset-delivery {}: {}",
                    s,
                    resp.text().await.unwrap_or_default()
                );
            }
            let meta: serde_json::Value = resp.json().await?;
            let location = meta
                .get("location")
                .and_then(|v| v.as_str())
                .context("no download location in asset-delivery response")?;
            let dl = ctx.client.get(location).send().await?;
            if !dl.status().is_success() {
                bail!("CDN returned {}", dl.status());
            }
            Ok(dl.bytes().await?.to_vec())
        }
        Source::Public => {
            let delivery = &ctx.hosts.delivery;
            let url = delivery.join(&format!("/v1/asset/?id={id}"));
            let mut req = ctx.client.get(&url);
            if let Some(ch) = &ctx.cookie_header {
                req = req.header(reqwest::header::COOKIE, ch);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let is_json = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("application/json"))
                .unwrap_or(false);
            let bytes = resp.bytes().await?;
            if is_json {
                // Body is an error payload, not the asset.
                let msg = serde_json::from_slice::<serde_json::Value>(&bytes)
                    .ok()
                    .and_then(|v| {
                        v.get("errors")
                            .and_then(|e| e.get(0))
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "asset not available (may be private or moderated)".into());
                bail!("{}", msg);
            }
            if !status.is_success() {
                bail!("assetdelivery returned {}", status);
            }
            Ok(bytes.to_vec())
        }
    }
}

/// Fetch `(Name, AssetTypeId)` from the economy details endpoint.
async fn fetch_details(ctx: &Ctx, id: u64) -> Result<(Option<String>, Option<i64>)> {
    let economy = &ctx.hosts.economy;
    let url = economy.join(&format!("/v2/assets/{id}/details"));
    let mut req = ctx.client.get(&url);
    if let Some(ch) = &ctx.cookie_header {
        req = req.header(reqwest::header::COOKIE, ch);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        bail!(
            "economy details {} (tip: pass --type to skip this lookup)",
            resp.status()
        );
    }
    let v: serde_json::Value = resp.json().await?;
    let name = v.get("Name").and_then(|n| n.as_str()).map(str::to_string);
    let type_id = v.get("AssetTypeId").and_then(|t| t.as_i64());
    Ok((name, type_id))
}

fn save(ctx: &Ctx, id: u64, name: &Option<String>, ext: &str, bytes: &[u8]) -> Result<String> {
    let filename = match name.as_deref().map(rbx_core::fs_name::safe_component) {
        Some(safe) if !safe.is_empty() => format!("{id}_{safe}.{ext}"),
        _ => format!("{id}.{ext}"),
    };
    let path = ctx.output.join(&filename);
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(filename)
}

fn read_ids_file(path: &PathBuf) -> Result<Vec<u64>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut ids = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',') {
        let t = token.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let id: u64 = t
            .parse()
            .with_context(|| format!("invalid asset id in file: '{t}'"))?;
        ids.push(id);
    }
    Ok(ids)
}

/// Resolve a `--type` spec to `(extension, optional AssetTypeId)`. The type id
/// is only needed so animation wrappers can still be dereferenced.
fn resolve_type(spec: &str) -> Result<(String, Option<i64>)> {
    if let Ok(id) = spec.parse::<i64>() {
        return Ok((ext_for_type_id(id).to_string(), Some(id)));
    }
    let (ext, type_id): (&str, Option<i64>) = match spec.to_lowercase().as_str() {
        "image" | "png" | "decal" => ("png", None),
        "audio" | "ogg" | "sound" => ("ogg", None),
        "mesh" => ("mesh", None),
        "lua" | "luau" | "script" => ("lua", None),
        "place" | "rbxl" => ("rbxl", None),
        "model" | "rbxm" => ("rbxm", None),
        "animation" | "anim" => ("rbxm", Some(24)),
        "video" | "mp4" => ("mp4", None),
        "font" | "ttf" => ("ttf", None),
        other => bail!(
            "unknown --type '{}'. Use a numeric AssetTypeId or one of: \
             image, audio, mesh, lua, place, model, animation, video, font",
            other
        ),
    };
    Ok((ext.to_string(), type_id))
}

/// AssetTypeId → file extension (matching the Roblox enum). Unknown → "bin".
fn ext_for_type_id(id: i64) -> &'static str {
    match id {
        1 | 2 | 11 | 12 | 13 | 18 | 21 | 34 => "png",
        3 => "ogg",
        4 => "mesh",
        5 => "lua",
        9 => "rbxl",
        62 => "mp4",
        73 => "ttf",
        8 | 10 | 17 | 19 | 24 | 27..=32 | 38 | 40..=58 | 61 | 64..=72 | 76..=79 | 88..=90 => "rbxm",
        _ => "bin",
    }
}

fn is_animation_type(id: i64) -> bool {
    matches!(id, 24 | 48..=56 | 61 | 78)
}

fn is_animation_wrapper(content: &[u8]) -> bool {
    contains(content, b"Animation")
        && contains(content, b"AnimationId")
        && !contains(content, b"KeyframeSequence")
}

/// Find the first `rbxassetid://<digits>` reference and return the id.
fn extract_rbxassetid(content: &[u8]) -> Option<u64> {
    let needle = b"rbxassetid://";
    let start = content.windows(needle.len()).position(|w| w == needle)? + needle.len();
    let digits: Vec<u8> = content[start..]
        .iter()
        .copied()
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Every host on one server, the way the other crates in this workspace
    /// do it: a test wants one place answering whatever the code asks for.
    fn ctx(server: &MockServer, source: Source, api_key: Option<&str>) -> Ctx {
        Ctx {
            client: reqwest::Client::new(),
            hosts: Hosts {
                cloud: ApiBase::new(server.uri()),
                delivery: ApiBase::new(server.uri()),
                economy: ApiBase::new(server.uri()),
            },
            cookie_header: None,
            api_key: api_key.map(str::to_string),
            source,
            output: PathBuf::from("."),
            version: None,
            raw: false,
            forced: None,
        }
    }

    /// The cloud path is two requests: the delivery API names a `location`,
    /// and the bytes come from there. Returning the first response's body
    /// would save JSON to disk under an `.rbxm` name.
    #[tokio::test]
    async fn the_cloud_source_follows_the_location_it_is_given() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset-delivery-api/v1/assetId/42"))
            .and(header("x-api-key", "k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "location": format!("{}/cdn/blob", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cdn/blob"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"bytes".to_vec()))
            .mount(&server)
            .await;

        let bytes = download_bytes(&ctx(&server, Source::Cloud, Some("k")), 42, None)
            .await
            .unwrap();
        assert_eq!(bytes, b"bytes");
    }

    /// A version is a different path, not a query parameter. Sent wrong, the
    /// API answers with the current version and the download silently gives
    /// you the wrong one.
    #[tokio::test]
    async fn a_version_goes_into_the_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset-delivery-api/v1/assetId/42/version/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "location": format!("{}/cdn/v3", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cdn/v3"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"v3".to_vec()))
            .mount(&server)
            .await;

        let bytes = download_bytes(&ctx(&server, Source::Cloud, Some("k")), 42, Some(3))
            .await
            .unwrap();
        assert_eq!(bytes, b"v3");
    }

    #[tokio::test]
    async fn a_refused_delivery_lookup_names_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset-delivery-api/v1/assetId/42"))
            .respond_with(ResponseTemplate::new(403).set_body_string("denied"))
            .mount(&server)
            .await;

        let error = download_bytes(&ctx(&server, Source::Cloud, Some("k")), 42, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("403"), "got: {error}");
    }

    /// The public endpoint takes the id as a query parameter, unlike the cloud
    /// one which puts it in the path.
    #[tokio::test]
    async fn the_public_source_asks_by_query_parameter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/asset/"))
            .and(query_param("id", "42"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"public".to_vec()))
            .mount(&server)
            .await;

        let bytes = download_bytes(&ctx(&server, Source::Public, None), 42, None)
            .await
            .unwrap();
        assert_eq!(bytes, b"public");
    }

    #[tokio::test]
    async fn details_come_from_the_economy_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/assets/42/details"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Name": "A Hat",
                "AssetTypeId": 8
            })))
            .mount(&server)
            .await;

        let (name, type_id) = fetch_details(&ctx(&server, Source::Public, None), 42)
            .await
            .unwrap();
        assert_eq!(name.as_deref(), Some("A Hat"));
        assert_eq!(type_id, Some(8));
    }
}
