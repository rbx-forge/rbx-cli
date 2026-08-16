//! Differential pull: writes minimal toml deltas given the base `[game]` /
//! `[media]` and the targeted env. Algorithm (per field):
//!   * if base is unset → write remote to base, remove any env override
//!   * if remote == base → remove any env override
//!   * else → write remote as env override
//!
//! Toml is rewritten with `toml_edit` so user comments are preserved.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;
use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::api::media::RemoteMedia;
use crate::api::models::ApiSocialLink;
use crate::api::RbxClient;
use crate::config::{
    Config, Devices, EnvOverlay, MediaConfig, MediaOverlay, PrivateServer, ServerFill, SocialLink,
    SocialLinks, Visibility,
};
use crate::ctx::MetaCtx;
use crate::lockfile::{Lockfile, MediaLock, DEFAULT_ENV, LOCKFILE_NAME, LOCKFILE_VERSION};
use rbx_core::confirm::confirm_always;
use rbx_core::image::{hash_bytes, process_bytes};

pub async fn run(
    ctx: &MetaCtx<'_>,
    dry_run: bool,
    accept_remote: bool,
    accept_local: bool,
    yes: bool,
) -> Result<()> {
    let mut config = Config::load(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new(".")).to_path_buf();
    let lockfile_path = config_dir.join(LOCKFILE_NAME);
    let mut lockfile = Lockfile::load(&lockfile_path)?;

    let (env_name, universe_id, place_id) = ctx.resolve_target(&config)?;

    // The place half of the guard `sync` applies. The differential pull
    // compares remote against this section, so a section describing a
    // different place makes it write overlays for what are really two
    // different places. `Repoint::Universe` because re-pointing an env at a
    // new universe and pulling is how you adopt it.
    super::ensure_not_repointed(
        &env_name,
        &lockfile.env_view(&env_name),
        universe_id,
        place_id,
        super::Repoint::Universe,
    )?;

    let cookie = ctx.resolve_cookie();
    let client = RbxClient::new(
        ctx.api_key(),
        cookie,
        universe_id,
        place_id,
        config.media.bleed,
        config.media.language_code.clone(),
    );

    // -----------------------------------------------------------------------
    // Fetch remote
    // -----------------------------------------------------------------------
    println!(
        "Pulling env '{}' (universe {} / place {})...",
        env_name.cyan(),
        universe_id,
        place_id
    );
    let universe = client.get_universe().await?;
    let place = client.get_place().await?;

    let (remote_server_fill, remote_allow_copying) = if client.has_cookie() {
        match client.get_place_legacy().await {
            Ok(legacy) => (
                ServerFill::from_legacy(
                    legacy.social_slot_type.as_deref(),
                    legacy.custom_social_slots_count,
                ),
                legacy.copying_allowed,
            ),
            Err(e) => {
                eprintln!(
                    "warning: legacy place fetch failed ({}). Skipping cookie-only place fields.",
                    e
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    let remote_studio_access = match client.get_universe_config_legacy().await {
        Ok(cfg) => cfg.studio_access_to_apis_allowed,
        Err(e) => {
            eprintln!(
                "warning: universe config fetch failed ({}). Skipping studio_access_to_apis_allowed.",
                e
            );
            None
        }
    };

    let remote_beta_mode = if client.has_cookie() {
        match client.get_beta_mode().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "warning: experience-releases fetch failed ({}). Skipping beta_mode.",
                    e
                );
                lockfile.env_view(&env_name).game.beta_mode
            }
        }
    } else {
        lockfile.env_view(&env_name).game.beta_mode
    };

    let remote_visibility = universe
        .visibility
        .as_deref()
        .and_then(Visibility::from_open_cloud);

    // -----------------------------------------------------------------------
    // Differential apply on the in-memory config
    // -----------------------------------------------------------------------
    let mut changes: Vec<String> = Vec::new();
    let overlay = config.envs.entry(env_name.clone()).or_default();

    // Scalars
    diff_apply_opt(
        &mut config.game.name,
        &mut overlay.name,
        place.display_name.clone().or(universe.display_name.clone()),
        "name",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.description,
        &mut overlay.description,
        place.description.clone().or(universe.description.clone()),
        "description",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.server_size,
        &mut overlay.server_size,
        place.server_size,
        "server_size",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.voice_chat,
        &mut overlay.voice_chat,
        universe.voice_chat_enabled,
        "voice_chat",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.allow_copying,
        &mut overlay.allow_copying,
        remote_allow_copying,
        "allow_copying",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.visibility,
        &mut overlay.visibility,
        remote_visibility,
        "visibility",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.studio_access_to_apis_allowed,
        &mut overlay.studio_access_to_apis_allowed,
        remote_studio_access,
        "studio_access_to_apis_allowed",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.beta_mode,
        &mut overlay.beta_mode,
        remote_beta_mode,
        "beta_mode",
        &mut changes,
    );

    // private_server (atomic)
    let remote_private_server = universe
        .private_server_price_robux
        .map(|price| PrivateServer { price });
    diff_apply_opt(
        &mut config.game.private_server,
        &mut overlay.private_server,
        remote_private_server,
        "private_server",
        &mut changes,
    );

    // server_fill (atomic)
    diff_apply_opt(
        &mut config.game.server_fill,
        &mut overlay.server_fill,
        remote_server_fill,
        "server_fill",
        &mut changes,
    );

    // devices (per-field)
    diff_apply_opt(
        &mut config.game.devices.desktop,
        &mut overlay.devices.desktop,
        universe.desktop_enabled,
        "devices.desktop",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.mobile,
        &mut overlay.devices.mobile,
        universe.mobile_enabled,
        "devices.mobile",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.tablet,
        &mut overlay.devices.tablet,
        universe.tablet_enabled,
        "devices.tablet",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.console,
        &mut overlay.devices.console,
        universe.console_enabled,
        "devices.console",
        &mut changes,
    );
    diff_apply_opt(
        &mut config.game.devices.vr,
        &mut overlay.devices.vr,
        universe.vr_enabled,
        "devices.vr",
        &mut changes,
    );

    // social links (per platform, atomic)
    diff_apply_social(
        &mut config.game.social_links.facebook,
        &mut overlay.social_links.facebook,
        universe.facebook_social_link.as_ref().map(to_social),
        "facebook",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.twitter,
        &mut overlay.social_links.twitter,
        universe.twitter_social_link.as_ref().map(to_social),
        "twitter",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.youtube,
        &mut overlay.social_links.youtube,
        universe.youtube_social_link.as_ref().map(to_social),
        "youtube",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.twitch,
        &mut overlay.social_links.twitch,
        universe.twitch_social_link.as_ref().map(to_social),
        "twitch",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.discord,
        &mut overlay.social_links.discord,
        universe.discord_social_link.as_ref().map(to_social),
        "discord",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.roblox_group,
        &mut overlay.social_links.roblox_group,
        universe.roblox_group_social_link.as_ref().map(to_social),
        "roblox_group",
        &mut changes,
    );
    diff_apply_social(
        &mut config.game.social_links.guilded,
        &mut overlay.social_links.guilded,
        universe.guilded_social_link.as_ref().map(to_social),
        "guilded",
        &mut changes,
    );

    // Print scalar diff
    if changes.is_empty() {
        println!("\n{}", "✓ Scalar fields already match remote.".green());
    } else {
        println!("\n{}", "Config updates:".bold());
        for c in &changes {
            println!("  • {}", c);
        }
    }

    // -----------------------------------------------------------------------
    // Media handling
    // -----------------------------------------------------------------------
    // For dry-run, prepare/save nothing on disk. Otherwise we ensure the env
    // entry exists in the lockfile up front so download_media can persist after
    // each successful operation (crash-safe).
    let had_media_before = {
        let el = lockfile.env_view(&env_name);
        el.media.icon.is_some() || !el.media.thumbnails.is_empty()
    };
    if !dry_run {
        // Confirm BEFORE any local file mutation. The pull rewrites
        // rbxmeta.toml + lockfile (and may overwrite media/) so this guards
        // against accidental clobbering of in-progress local edits.
        confirm_always(
            &format!(
                "Overwrite local rbxmeta.toml and lockfile for env '{}'?",
                env_name
            ),
            yes,
        )?;

        lockfile.version = LOCKFILE_VERSION;
        let el = lockfile.env_mut(&env_name);
        el.universe_id = universe_id;
        el.place_id = place_id;
        lockfile.save(&lockfile_path)?;
    }

    if accept_remote && !dry_run {
        download_media(
            &client,
            &mut config,
            &mut lockfile,
            &env_name,
            &config_dir,
            &lockfile_path,
        )
        .await?;
    } else if accept_local && !dry_run {
        println!(
            "\n{} clearing media hashes for env '{}' (next sync will re-upload).",
            "--accept-local:".yellow(),
            env_name
        );
        let el = lockfile.env_mut(&env_name);
        el.media.icon = None;
        el.media.thumbnails = Vec::new();
        lockfile.save(&lockfile_path)?;
    } else if !accept_remote && !accept_local && had_media_before {
        println!(
            "\n{} media not pulled. Use --accept-remote to download from Roblox or \
             --accept-local to re-upload local on next sync.",
            "note:".dimmed()
        );
    }

    // -----------------------------------------------------------------------
    // Persist the resolved game state for this env in the lockfile.
    // -----------------------------------------------------------------------
    let (resolved_game, _resolved_media) = config.resolve_env(Some(&env_name));
    let new_game_lock = crate::diff::config_to_lock(&resolved_game);

    if dry_run {
        println!("\n{}", "(dry-run: nothing written)".dimmed());
        return Ok(());
    }

    lockfile.env_mut(&env_name).game = new_game_lock;
    lockfile.save(&lockfile_path)?;
    println!(
        "\n{} {}",
        "Updated".green().bold(),
        lockfile_path.display().to_string().cyan()
    );

    // -----------------------------------------------------------------------
    // Save config back to the toml (preserves comments via toml_edit)
    // -----------------------------------------------------------------------
    write_config_toml(&ctx.config, &config)?;
    println!(
        "{} {}",
        "Updated".green().bold(),
        ctx.config.display().to_string().cyan()
    );

    Ok(())
}

fn to_social(link: &ApiSocialLink) -> SocialLink {
    SocialLink {
        title: link.title.clone(),
        url: link.uri.clone(),
    }
}

// ---------------------------------------------------------------------------
// Differential algorithm
// ---------------------------------------------------------------------------

/// Apply differential algorithm for an `Option<T>` field.
///   - remote=None: do nothing (we don't pull "absence")
///   - base=None && remote=Some: promote to base, clear overlay
///   - remote==base: clear overlay
///   - else: set overlay = remote
fn diff_apply_opt<T: Clone + PartialEq + std::fmt::Debug>(
    base: &mut Option<T>,
    overlay: &mut Option<T>,
    remote: Option<T>,
    label: &str,
    changes: &mut Vec<String>,
) {
    let Some(r) = remote else { return };
    match base {
        None => {
            *base = Some(r.clone());
            if overlay.is_some() {
                *overlay = None;
            }
            changes.push(format!("{}: base ← {:?}", label, r));
        }
        Some(b) if *b == r => {
            if overlay.take().is_some() {
                changes.push(format!("{}: cleared override (matches base)", label));
            }
        }
        Some(_) => {
            let was = overlay.replace(r.clone());
            if was.as_ref() != Some(&r) {
                changes.push(format!("{}: override ← {:?}", label, r));
            }
        }
    }
}

fn diff_apply_social(
    base: &mut Option<SocialLink>,
    overlay: &mut Option<SocialLink>,
    remote: Option<SocialLink>,
    platform: &str,
    changes: &mut Vec<String>,
) {
    let Some(r) = remote else { return };
    match base {
        None => {
            *base = Some(r.clone());
            if overlay.is_some() {
                *overlay = None;
            }
            changes.push(format!("social.{}: base ← '{}'", platform, r.title));
        }
        Some(b) if *b == r => {
            if overlay.take().is_some() {
                changes.push(format!("social.{}: cleared override", platform));
            }
        }
        Some(_) => {
            let was = overlay.replace(r.clone());
            if was.as_ref() != Some(&r) {
                changes.push(format!("social.{}: override ← '{}'", platform, r.title));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Media download (--accept-remote)
// ---------------------------------------------------------------------------

async fn download_media(
    client: &RbxClient,
    config: &mut Config,
    lockfile: &mut Lockfile,
    env: &str,
    config_dir: &Path,
    lockfile_path: &Path,
) -> Result<()> {
    let media_for_dir = effective_media(config, env);
    let dir_rel = media_for_dir
        .dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("assets"));
    // For named envs we sub-namespace the default media directory so two envs
    // never collide on the same default file name (e.g. assets/icon.png).
    let env_specific = env != DEFAULT_ENV;
    let env_dir_rel = if env_specific {
        dir_rel.join(env)
    } else {
        dir_rel.clone()
    };

    // Snapshot the lockfile media state BEFORE we mutate it. Used for cached
    // checks (image_id match → skip download). Mutations happen on
    // `lockfile.env_mut(env).media` and are persisted after each operation.
    let prior_media = lockfile.env_view(env).media.clone();

    // Icon.
    match client.fetch_icon().await? {
        Some(RemoteMedia {
            target_id,
            image_url,
        }) => {
            let effective = effective_media(config, env);
            // Prefer the env's explicit overlay path; fall back to base only in
            // single-env mode; otherwise default to assets/<env>/icon.png so
            // sibling envs do not overwrite each other.
            let overlay_path = config.envs.get(env).and_then(|o| o.media.icon.clone());
            let icon_rel: PathBuf = if let Some(p) = overlay_path {
                p
            } else if env_specific {
                env_dir_rel.join("icon.png")
            } else {
                effective
                    .icon
                    .clone()
                    .unwrap_or_else(|| dir_rel.join("icon.png"))
            };
            let icon_full = config_dir.join(&icon_rel);

            let cached = prior_media
                .icon
                .as_ref()
                .map(|l| l.image_id == Some(target_id))
                .unwrap_or(false);
            let local_present = icon_full.exists();

            if cached && local_present {
                println!(
                    "\n{} icon unchanged (image_id {}, {})",
                    "·".dimmed(),
                    target_id,
                    icon_full.display()
                );
            } else {
                println!(
                    "\n{} icon (image_id {})...",
                    "Downloading".cyan(),
                    target_id
                );
                let bytes = client.download_bytes(&image_url).await?;
                if let Some(p) = icon_full.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&icon_full, &bytes)
                    .with_context(|| format!("Failed to write {}", icon_full.display()))?;
                let processed = process_bytes(&bytes, effective.bleed)?;
                let hash = hash_bytes(&processed);
                lockfile.env_mut(env).media.icon = Some(MediaLock {
                    hash,
                    image_id: Some(target_id),
                });
                lockfile.save(lockfile_path)?;

                // Named envs always write the path as an overlay so they never
                // hijack the base. Standalone (DEFAULT_ENV) uses the
                // promote-to-base differential.
                if env_specific {
                    config.envs.entry(env.to_string()).or_default().media.icon = Some(icon_rel);
                } else {
                    apply_media_path(
                        &mut config.media.icon,
                        &mut config.envs.entry(env.to_string()).or_default().media.icon,
                        Some(icon_rel),
                    );
                }
                println!("  {} saved to {}", "✓".green(), icon_full.display());
            }
        }
        None => {
            println!("\n{} no icon on Roblox", "✗".dimmed());
            lockfile.env_mut(env).media.icon = None;
            lockfile.save(lockfile_path)?;
        }
    }

    // Thumbnails. Per-index check: reuse if image_id matches and local file exists.
    // After each download (or skip), the lockfile is updated at index i and
    // saved so a crash mid-loop leaves the lockfile consistent with what's been
    // downloaded to disk.
    let remote_thumbs = client.fetch_thumbnails().await?;
    if remote_thumbs.is_empty() {
        lockfile.env_mut(env).media.thumbnails = Vec::new();
        lockfile.save(lockfile_path)?;
    } else {
        println!(
            "\n{} {} thumbnail(s)...",
            "Syncing".cyan(),
            remote_thumbs.len()
        );
        let mut new_paths: Vec<PathBuf> = Vec::with_capacity(remote_thumbs.len());
        let effective = effective_media(config, env);
        let overlay_thumbs = config
            .envs
            .get(env)
            .and_then(|o| o.media.thumbnails.clone());
        for (
            i,
            RemoteMedia {
                target_id,
                image_url,
            },
        ) in remote_thumbs.iter().enumerate()
        {
            // Pick the destination path with the same fallback rules as for
            // the icon: overlay (env) > effective base (single-env) > env-namespaced default.
            let overlay_path = overlay_thumbs.as_ref().and_then(|v| v.get(i)).cloned();
            let path_rel: PathBuf = if let Some(p) = overlay_path {
                p
            } else if env_specific {
                env_dir_rel.join(format!("thumbnail_{:02}.png", i + 1))
            } else {
                effective
                    .thumbnails
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| dir_rel.join(format!("thumbnail_{:02}.png", i + 1)))
            };
            let path_full = config_dir.join(&path_rel);

            let cached = prior_media
                .thumbnails
                .get(i)
                .map(|l| l.image_id == Some(*target_id))
                .unwrap_or(false);
            let local_present = path_full.exists();

            let new_lock = if cached && local_present {
                println!(
                    "  {} {} unchanged (image_id {})",
                    "·".dimmed(),
                    path_full.display(),
                    target_id
                );
                prior_media.thumbnails[i].clone()
            } else {
                let bytes = client.download_bytes(image_url).await?;
                if let Some(p) = path_full.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&path_full, &bytes)
                    .with_context(|| format!("Failed to write {}", path_full.display()))?;
                let processed = process_bytes(&bytes, effective.bleed)?;
                let hash = hash_bytes(&processed);
                println!(
                    "  {} {} (image_id {})",
                    "✓".green(),
                    path_full.display(),
                    target_id
                );
                MediaLock {
                    hash,
                    image_id: Some(*target_id),
                }
            };

            // Insert/replace at index i and persist.
            {
                let media = &mut lockfile.env_mut(env).media;
                if media.thumbnails.len() > i {
                    media.thumbnails[i] = new_lock;
                } else {
                    media.thumbnails.push(new_lock);
                }
            }
            lockfile.save(lockfile_path)?;
            new_paths.push(path_rel);
        }

        // Drop trailing entries that no longer correspond to a remote thumbnail.
        {
            let media = &mut lockfile.env_mut(env).media;
            if media.thumbnails.len() > remote_thumbs.len() {
                media.thumbnails.truncate(remote_thumbs.len());
            }
        }
        lockfile.save(lockfile_path)?;

        // Named envs always write the paths as an overlay so they never hijack
        // the base. Standalone (DEFAULT_ENV) uses the promote-to-base diff.
        if env_specific {
            config
                .envs
                .entry(env.to_string())
                .or_default()
                .media
                .thumbnails = Some(new_paths);
        } else {
            apply_media_thumbnails(
                &mut config.media.thumbnails,
                &mut config
                    .envs
                    .entry(env.to_string())
                    .or_default()
                    .media
                    .thumbnails,
                new_paths,
            );
        }
    }

    Ok(())
}

/// Resolved (base + env overlay) MediaConfig for the current pull state.
fn effective_media(config: &Config, env: &str) -> MediaConfig {
    let mut media = config.media.clone();
    if let Some(overlay) = config.envs.get(env) {
        media.apply_overlay(&overlay.media);
    }
    media
}

fn apply_media_path(
    base: &mut Option<PathBuf>,
    overlay: &mut Option<PathBuf>,
    remote: Option<PathBuf>,
) {
    let Some(r) = remote else { return };
    match base {
        None => {
            *base = Some(r);
            *overlay = None;
        }
        Some(b) if *b == r => {
            *overlay = None;
        }
        Some(_) => {
            *overlay = Some(r);
        }
    }
}

fn apply_media_thumbnails(
    base: &mut Vec<PathBuf>,
    overlay: &mut Option<Vec<PathBuf>>,
    remote: Vec<PathBuf>,
) {
    if base.is_empty() {
        *base = remote;
        *overlay = None;
    } else if *base == remote {
        *overlay = None;
    } else {
        *overlay = Some(remote);
    }
}

// ---------------------------------------------------------------------------
// TOML write-back (preserves user comments via toml_edit)
// ---------------------------------------------------------------------------

fn write_config_toml(path: &Path, config: &Config) -> Result<()> {
    let original = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut doc: DocumentMut = original
        .parse()
        .with_context(|| format!("Failed to parse {} as TOML", path.display()))?;

    // [experience] (if present in config)
    if let Some(exp) = &config.experience {
        let t = ensure_table(&mut doc, "experience");
        set_value(t, "universe_id", value(exp.universe_id as i64));
        set_value(t, "place_id", value(exp.place_id as i64));
    } else if doc.contains_key("experience") {
        doc.remove("experience");
    }

    // [game] base
    write_game_block(&mut doc, "game", config);

    // [media] base
    write_media_block(&mut doc, "media", &config.media);

    // [envs.<name>] overlays
    sync_envs_tables(&mut doc, config);

    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

fn write_game_block(doc: &mut DocumentMut, key: &str, config: &Config) {
    let game = &config.game;
    let t = ensure_table(doc, key);
    set_opt_str(t, "name", game.name.as_deref());
    set_opt_str(t, "description", game.description.as_deref());
    set_opt_int(t, "server_size", game.server_size.map(|v| v as i64));
    set_opt_bool(t, "voice_chat", game.voice_chat);
    set_opt_bool(t, "allow_copying", game.allow_copying);
    set_opt_str(t, "visibility", game.visibility.map(visibility_str));
    set_opt_bool(
        t,
        "studio_access_to_apis_allowed",
        game.studio_access_to_apis_allowed,
    );
    set_opt_bool(t, "beta_mode", game.beta_mode);

    // sub-tables
    write_private_server_sub(doc, key, game.private_server.as_ref());
    write_devices_sub(doc, key, &game.devices);
    write_server_fill_sub(doc, key, game.server_fill.as_ref());
    write_social_links_sub(doc, key, &game.social_links);
}

fn write_private_server_sub(doc: &mut DocumentMut, parent: &str, ps: Option<&PrivateServer>) {
    match ps {
        Some(ps) => {
            let t = ensure_subtable(doc, parent, "private_server");
            set_value(t, "price", value(ps.price as i64));
        }
        None => remove_subtable(doc, parent, "private_server"),
    }
}

fn write_devices_sub(doc: &mut DocumentMut, parent: &str, devices: &Devices) {
    if devices.is_empty() {
        remove_subtable(doc, parent, "devices");
        return;
    }
    let t = ensure_subtable(doc, parent, "devices");
    set_opt_bool(t, "desktop", devices.desktop);
    set_opt_bool(t, "mobile", devices.mobile);
    set_opt_bool(t, "tablet", devices.tablet);
    set_opt_bool(t, "console", devices.console);
    set_opt_bool(t, "vr", devices.vr);
}

fn write_server_fill_sub(doc: &mut DocumentMut, parent: &str, sf: Option<&ServerFill>) {
    match sf {
        Some(sf) => {
            let t = ensure_subtable(doc, parent, "server_fill");
            set_value(t, "mode", value(server_fill_mode_str(sf)));
            match sf {
                ServerFill::Custom { reserved_slots } => {
                    set_value(t, "reserved_slots", value(*reserved_slots as i64));
                }
                _ => {
                    t.remove("reserved_slots");
                }
            }
        }
        None => remove_subtable(doc, parent, "server_fill"),
    }
}

fn write_social_links_sub(doc: &mut DocumentMut, parent: &str, links: &SocialLinks) {
    set_social(doc, parent, "facebook", &links.facebook);
    set_social(doc, parent, "twitter", &links.twitter);
    set_social(doc, parent, "youtube", &links.youtube);
    set_social(doc, parent, "twitch", &links.twitch);
    set_social(doc, parent, "discord", &links.discord);
    set_social(doc, parent, "roblox_group", &links.roblox_group);
    set_social(doc, parent, "guilded", &links.guilded);
}

fn write_media_block(doc: &mut DocumentMut, key: &str, media: &MediaConfig) {
    let t = ensure_table(doc, key);
    set_opt_path(t, "icon", media.icon.as_deref());
    set_path_array(t, "thumbnails", &media.thumbnails);
    set_opt_path(t, "dir", media.dir.as_deref());
    if media.bleed {
        t.remove("bleed");
    } else {
        set_value(t, "bleed", value(false));
    }
    if media.language_code == "en_us" {
        t.remove("language_code");
    } else {
        set_value(t, "language_code", value(media.language_code.clone()));
    }
}

/// Write or update `[envs.<name>]` overlays in the doc. Removes envs that
/// no longer exist in the config (or are entirely empty).
fn sync_envs_tables(doc: &mut DocumentMut, config: &Config) {
    // Add/update overlays
    for (name, overlay) in &config.envs {
        if overlay.is_empty() {
            remove_env_section(doc, name);
            continue;
        }
        write_env_overlay(doc, name, overlay);
    }

    // Remove envs from the doc that are not in the config anymore.
    if let Some(envs) = doc.get_mut("envs").and_then(|i| i.as_table_mut()) {
        let to_remove: Vec<String> = envs
            .iter()
            .filter_map(|(k, _)| {
                if !config.envs.contains_key(k)
                    || config.envs.get(k).map(|o| o.is_empty()).unwrap_or(true)
                {
                    Some(k.to_string())
                } else {
                    None
                }
            })
            .collect();
        for k in to_remove {
            envs.remove(&k);
        }
        if envs.is_empty() {
            doc.remove("envs");
        }
    }
}

fn write_env_overlay(doc: &mut DocumentMut, env: &str, overlay: &EnvOverlay) {
    let envs_table = ensure_table(doc, "envs");
    if !envs_table.contains_key(env) {
        envs_table.insert(env, Item::Table(Table::new()));
    }
    let t = envs_table[env]
        .as_table_mut()
        .expect("env entry is not a table");

    set_opt_str(t, "name", overlay.name.as_deref());
    set_opt_str(t, "description", overlay.description.as_deref());
    set_opt_int(t, "server_size", overlay.server_size.map(|v| v as i64));
    set_opt_bool(t, "voice_chat", overlay.voice_chat);
    set_opt_bool(t, "allow_copying", overlay.allow_copying);
    set_opt_str(t, "visibility", overlay.visibility.map(visibility_str));
    set_opt_bool(
        t,
        "studio_access_to_apis_allowed",
        overlay.studio_access_to_apis_allowed,
    );
    set_opt_bool(t, "beta_mode", overlay.beta_mode);

    // sub-tables under envs.<env>.<sub>
    let env_parent = format!("envs.{}", env);
    write_private_server_sub(doc, &env_parent, overlay.private_server.as_ref());
    write_devices_sub(doc, &env_parent, &overlay.devices);
    write_server_fill_sub(doc, &env_parent, overlay.server_fill.as_ref());
    write_social_links_sub(doc, &env_parent, &overlay.social_links);
    write_media_overlay(doc, &env_parent, &overlay.media);
}

fn write_media_overlay(doc: &mut DocumentMut, parent: &str, media: &MediaOverlay) {
    if media.is_empty() {
        remove_subtable_dotted(doc, parent, "media");
        return;
    }
    let t = ensure_subtable_dotted(doc, parent, "media");
    set_opt_path(t, "icon", media.icon.as_deref());
    match &media.thumbnails {
        Some(thumbs) => set_path_array(t, "thumbnails", thumbs),
        None => {
            t.remove("thumbnails");
        }
    }
    set_opt_path(t, "dir", media.dir.as_deref());
    set_opt_bool(t, "bleed", media.bleed);
    set_opt_str(t, "language_code", media.language_code.as_deref());
}

fn remove_env_section(doc: &mut DocumentMut, env: &str) {
    if let Some(envs) = doc.get_mut("envs").and_then(|i| i.as_table_mut()) {
        envs.remove(env);
        if envs.is_empty() {
            doc.remove("envs");
        }
    }
}

// ---------------------------------------------------------------------------
// toml_edit helpers
// ---------------------------------------------------------------------------

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if !doc.contains_key(key) {
        doc.insert(key, Item::Table(Table::new()));
    }
    doc[key].as_table_mut().expect("section is not a table")
}

/// Walk a dotted parent path (e.g. "envs.dev") to get a mutable subtable, then
/// create-or-fetch the child. Used for `envs.<name>.<sub>` style.
fn ensure_subtable<'a>(doc: &'a mut DocumentMut, parent: &str, child: &str) -> &'a mut Table {
    let parts: Vec<&str> = parent.split('.').collect();
    let mut current: &mut Table = ensure_path(doc, &parts);
    if !current.contains_key(child) {
        current.insert(child, Item::Table(Table::new()));
    }
    current = current[child]
        .as_table_mut()
        .expect("subsection is not a table");
    current
}

fn ensure_subtable_dotted<'a>(
    doc: &'a mut DocumentMut,
    parent: &str,
    child: &str,
) -> &'a mut Table {
    ensure_subtable(doc, parent, child)
}

fn ensure_path<'a>(doc: &'a mut DocumentMut, parts: &[&str]) -> &'a mut Table {
    let mut current = doc.as_table_mut();
    for p in parts {
        if !current.contains_key(p) {
            current.insert(p, Item::Table(Table::new()));
        }
        current = current[*p]
            .as_table_mut()
            .expect("intermediate is not a table");
    }
    current
}

fn remove_subtable(doc: &mut DocumentMut, parent: &str, child: &str) {
    let parts: Vec<&str> = parent.split('.').collect();
    let mut current = doc.as_table_mut();
    for p in &parts {
        match current.get_mut(p).and_then(|i| i.as_table_mut()) {
            Some(t) => current = t,
            None => return,
        }
    }
    current.remove(child);
}

fn remove_subtable_dotted(doc: &mut DocumentMut, parent: &str, child: &str) {
    remove_subtable(doc, parent, child)
}

/// Assign `item` to `t[key]`, carrying over the decor of whatever was there.
///
/// `toml_edit` stores a value's surrounding trivia — the whitespace before it
/// and, crucially, any trailing `# comment` — on the value itself. A plain
/// `t[key] = value(..)` therefore drops the user's inline comment on every key
/// pull rewrites, which defeats the point of maintaining this mirror at all.
fn set_value(t: &mut Table, key: &str, mut item: Item) {
    if let Some(decor) = t
        .get(key)
        .and_then(|i| i.as_value())
        .map(|v| v.decor().clone())
    {
        if let Some(v) = item.as_value_mut() {
            *v.decor_mut() = decor;
        }
    }
    t[key] = item;
}

fn set_opt_str(t: &mut Table, key: &str, val: Option<&str>) {
    match val {
        Some(v) => set_value(t, key, value(v.to_string())),
        None => {
            t.remove(key);
        }
    }
}

fn set_opt_int(t: &mut Table, key: &str, val: Option<i64>) {
    match val {
        Some(v) => set_value(t, key, value(v)),
        None => {
            t.remove(key);
        }
    }
}

fn set_opt_bool(t: &mut Table, key: &str, val: Option<bool>) {
    match val {
        Some(v) => set_value(t, key, value(v)),
        None => {
            t.remove(key);
        }
    }
}

fn set_opt_path(t: &mut Table, key: &str, val: Option<&Path>) {
    match val {
        Some(p) => set_value(t, key, value(path_to_toml_str(p))),
        None => {
            t.remove(key);
        }
    }
}

fn set_path_array(t: &mut Table, key: &str, paths: &[PathBuf]) {
    if paths.is_empty() {
        t.remove(key);
    } else {
        let mut arr = Array::new();
        for p in paths {
            arr.push(path_to_toml_str(p));
        }
        set_value(t, key, value(arr));
    }
}

fn set_social(doc: &mut DocumentMut, parent: &str, platform: &str, link: &Option<SocialLink>) {
    match link {
        Some(l) => {
            let social = ensure_subtable(doc, parent, "social_links");
            if !social.contains_key(platform) {
                social.insert(platform, Item::Table(Table::new()));
            }
            let platform_t = social[platform]
                .as_table_mut()
                .expect("platform entry is not a table");
            set_value(platform_t, "title", value(l.title.clone()));
            set_value(platform_t, "url", value(l.url.clone()));
        }
        None => {
            // Walk to the social_links sub-table and remove the platform.
            let parts: Vec<&str> = parent.split('.').collect();
            let mut current = doc.as_table_mut();
            for p in &parts {
                match current.get_mut(p).and_then(|i| i.as_table_mut()) {
                    Some(t) => current = t,
                    None => return,
                }
            }
            if let Some(social) = current
                .get_mut("social_links")
                .and_then(|i| i.as_table_mut())
            {
                social.remove(platform);
            }
        }
    }
}

fn path_to_toml_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn visibility_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Private => "private",
    }
}

fn server_fill_mode_str(sf: &ServerFill) -> &'static str {
    match sf {
        ServerFill::Automatic => "automatic",
        ServerFill::Empty => "empty",
        ServerFill::Custom { .. } => "custom",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Experience, Game};
    use std::collections::BTreeMap;

    /// Seed a config file and return `(tempdir, path)`. The tempdir must stay
    /// alive for the path to remain valid.
    fn seed(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rbxmeta.toml");
        std::fs::write(&path, content).expect("seed write");
        (dir, path)
    }

    fn social(title: &str) -> SocialLink {
        SocialLink {
            title: title.to_string(),
            url: format!("https://example.test/{}", title.to_ascii_lowercase()),
        }
    }

    /// Every device toggle set, with mixed values so a field written to the
    /// wrong key is not masked by an identical neighbour.
    fn all_devices() -> Devices {
        Devices {
            desktop: Some(true),
            mobile: Some(false),
            tablet: Some(true),
            console: Some(false),
            vr: Some(true),
        }
    }

    fn all_social_links() -> SocialLinks {
        SocialLinks {
            facebook: Some(social("facebook")),
            twitter: Some(social("twitter")),
            youtube: Some(social("youtube")),
            twitch: Some(social("twitch")),
            discord: Some(social("discord")),
            roblox_group: Some(social("robloxgroup")),
            guilded: Some(social("guilded")),
        }
    }

    /// A `Config` with every optional field populated.
    ///
    /// The struct literals here are deliberately exhaustive — no
    /// `..Default::default()`. A field added to `config.rs` breaks this
    /// constructor at compile time, which forces whoever adds it to decide what
    /// the `toml_edit` mirror should do with it. That is the whole point: the
    /// mirror is a second serializer, and nothing else makes drift visible.
    fn fully_populated_config() -> Config {
        let mut envs = BTreeMap::new();
        envs.insert(
            "prod".to_string(),
            EnvOverlay {
                name: Some("overlay name".to_string()),
                description: Some("overlay description".to_string()),
                server_size: Some(70),
                voice_chat: Some(false),
                allow_copying: Some(false),
                visibility: Some(Visibility::Private),
                studio_access_to_apis_allowed: Some(false),
                beta_mode: Some(false),
                private_server: Some(PrivateServer { price: 0 }),
                devices: Devices {
                    desktop: Some(false),
                    mobile: Some(true),
                    tablet: Some(false),
                    console: Some(true),
                    vr: Some(false),
                },
                social_links: SocialLinks {
                    facebook: Some(social("prod-facebook")),
                    twitter: Some(social("prod-twitter")),
                    youtube: Some(social("prod-youtube")),
                    twitch: Some(social("prod-twitch")),
                    discord: Some(social("prod-discord")),
                    roblox_group: Some(social("prod-robloxgroup")),
                    guilded: Some(social("prod-guilded")),
                },
                server_fill: Some(ServerFill::Empty),
                media: MediaOverlay {
                    icon: Some(PathBuf::from("assets/prod/icon.png")),
                    thumbnails: Some(vec![
                        PathBuf::from("assets/prod/one.png"),
                        PathBuf::from("assets/prod/two.png"),
                    ]),
                    dir: Some(PathBuf::from("assets/prod")),
                    bleed: Some(false),
                    language_code: Some("de_de".to_string()),
                },
            },
        );

        Config {
            experience: Some(Experience {
                universe_id: 1234,
                place_id: 5678,
            }),
            game: Game {
                name: Some("Parity Game".to_string()),
                description: Some("A description".to_string()),
                server_size: Some(42),
                voice_chat: Some(true),
                private_server: Some(PrivateServer { price: 25 }),
                devices: all_devices(),
                social_links: all_social_links(),
                server_fill: Some(ServerFill::Custom { reserved_slots: 7 }),
                allow_copying: Some(true),
                visibility: Some(Visibility::Public),
                studio_access_to_apis_allowed: Some(true),
                beta_mode: Some(true),
            },
            media: MediaConfig {
                icon: Some(PathBuf::from("assets/icon.png")),
                thumbnails: vec![
                    PathBuf::from("assets/one.png"),
                    PathBuf::from("assets/two.png"),
                ],
                dir: Some(PathBuf::from("assets")),
                // Both deliberately non-default: the mirror omits these keys at
                // their default value, so a default-valued test would pass even
                // if the mirror never wrote them at all.
                bleed: false,
                language_code: "fr_fr".to_string(),
            },
            envs,
        }
    }

    /// The parity test. Anything the mirror fails to write is data the user
    /// silently loses on the next `rbx meta pull`.
    #[test]
    fn toml_edit_mirror_writes_every_field_the_serde_model_reads() {
        let config = fully_populated_config();
        let (_dir, path) = seed("");

        write_config_toml(&path, &config).expect("write through the toml_edit mirror");

        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(
            reparsed,
            config,
            "toml_edit mirror and serde model disagree.\n--- file written ---\n{}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
    }

    /// Default-valued `media.bleed` / `media.language_code` are omitted by the
    /// mirror on purpose. That is only correct as long as serde's defaults agree.
    #[test]
    fn omitted_media_defaults_reparse_to_the_same_values() {
        let mut config = fully_populated_config();
        config.media.bleed = true;
        config.media.language_code = "en_us".to_string();
        // Clear the env media overlay too, so the assertions below can look at
        // the whole file rather than having to isolate the base `[media]` table.
        config
            .envs
            .get_mut("prod")
            .expect("prod overlay")
            .media
            .bleed = None;
        config
            .envs
            .get_mut("prod")
            .expect("prod overlay")
            .media
            .language_code = None;
        let (_dir, path) = seed("");

        write_config_toml(&path, &config).expect("write through the toml_edit mirror");

        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(
            !written.contains("bleed"),
            "default bleed should not be written: {written}"
        );
        assert!(
            !written.contains("language_code"),
            "default language_code should not be written: {written}"
        );
        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(reparsed, config);
    }

    /// A cleared field must be removed from the file, not left stale.
    #[test]
    fn mirror_removes_fields_that_are_no_longer_set() {
        let (_dir, path) = seed(
            r#"[experience]
universe_id = 1
place_id = 2

[game]
name = "old"
beta_mode = true

[game.private_server]
price = 100

[game.devices]
desktop = true

[game.server_fill]
mode = "custom"
reserved_slots = 3

[game.social_links.discord]
title = "Discord"
url = "https://example.test/discord"

[envs.dev]
name = "dev name"
"#,
        );

        let empty = Config {
            experience: None,
            game: Game::default(),
            media: MediaConfig::default(),
            envs: BTreeMap::new(),
        };
        write_config_toml(&path, &empty).expect("write through the toml_edit mirror");

        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(reparsed, empty);
    }

    /// Round-trip with user comments and an env overlay: the mirror exists only
    /// to preserve the former, so a test that ignores them tests nothing.
    #[test]
    fn round_trip_preserves_user_comments_and_overlays() {
        let (_dir, path) = seed(
            r#"# rbx meta configuration
# Managed by hand, please keep these notes.

[experience]
universe_id = 1 # the live universe
place_id = 2

[game]
# The public-facing name. Marketing owns this string.
name = "old name"
server_size = 10

# Private servers are a revenue line; do not disable without asking.
[game.private_server]
price = 50

[media]
# Icons live next to the source art.
icon = "assets/old-icon.png"

[envs.dev]
# Dev is deliberately hidden.
visibility = "private"
"#,
        );

        let mut config = Config::load(&path).expect("load seed");
        config.game.name = Some("new name".to_string());
        config.game.server_size = Some(25);
        config.media.icon = Some(PathBuf::from("assets/new-icon.png"));
        config.envs.get_mut("dev").expect("dev overlay").description = Some("dev only".to_string());

        write_config_toml(&path, &config).expect("write through the toml_edit mirror");
        let written = std::fs::read_to_string(&path).expect("read back");

        for comment in [
            "# rbx meta configuration",
            "# Managed by hand, please keep these notes.",
            "# The public-facing name. Marketing owns this string.",
            "# Private servers are a revenue line; do not disable without asking.",
            "# Icons live next to the source art.",
            "# Dev is deliberately hidden.",
            "# the live universe",
        ] {
            assert!(
                written.contains(comment),
                "comment lost on write-back: {comment}\n--- file written ---\n{written}"
            );
        }

        let reparsed = Config::load(&path).expect("re-parse through serde");
        assert_eq!(reparsed, config);
    }
}
