use anyhow::{bail, Result};
use colored::Colorize;

use crate::api::RbxClient;
use crate::config::Config;
use crate::ctx::MetaCtx;
use crate::diff::{build_plan, desired_order, IconPlan, SyncPlan};
use crate::lockfile::{Lockfile, MediaLock, LOCKFILE_NAME, LOCKFILE_VERSION};
use rbx_core::confirm::confirm_destructive;
use rbx_core::places::PlacesFile;

pub async fn run(ctx: &MetaCtx<'_>, dry_run: bool, yes: bool) -> Result<()> {
    let config = Config::load(&ctx.config)?;
    let config_dir = ctx
        .config
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let lockfile_path = config_dir.join(LOCKFILE_NAME);
    let mut lockfile = Lockfile::load(&lockfile_path)?;

    let (env, universe_id, place_id) = ctx.resolve_target(&config)?;
    let (game, media) = config.resolve_env(Some(&env));
    Config::validate_invariants(&game)?;
    Config::validate_media_paths(&media, &config_dir)?;

    let env_lock = lockfile.env_view(&env);

    // Safety: refuse to re-point this section silently, at either level.
    // `sync` diffs against it to decide what to send, so nothing about the
    // section may describe a different target. See `commands::ensure_not_repointed`.
    super::ensure_not_repointed(
        &env,
        &env_lock,
        universe_id,
        place_id,
        super::Repoint::Nothing,
    )?;

    let plan = build_plan(&game, &media, &env_lock.game, &env_lock.media, &config_dir)?;
    print_plan(&plan, &env);

    // Lockfile bookkeeping, before the "nothing to do" exit and outside the
    // confirmation gate below.
    //
    // A thumbnail entry with no `image_id` produces no remote work (nothing to
    // delete, nothing to reorder) so it never appears in a plan, and a
    // lockfile holding only these yields an empty plan. Pruned after that exit,
    // it would never be pruned at all, which is how these came to survive every
    // sync forever. It is a local write and touches nothing on Roblox, so it
    // does not belong behind a prompt about applying remote changes either.
    let stale_thumbnails = imageless_thumbnails(&lockfile.env_view(&env).media.thumbnails);
    if stale_thumbnails > 0 {
        println!(
            "\n{} {} stale thumbnail lock entr{} with no image_id",
            if dry_run { "Would drop" } else { "Dropping" }
                .cyan()
                .bold(),
            stale_thumbnails,
            if stale_thumbnails == 1 { "y" } else { "ies" }
        );
        if !dry_run {
            prune_imageless_thumbnails(&mut lockfile.env_mut(&env).media.thumbnails);
            lockfile.save(&lockfile_path)?;
        }
    }

    if plan.is_empty() {
        if stale_thumbnails == 0 {
            println!("{}", "Nothing to do, everything is in sync.".green());
        }
        return Ok(());
    }

    if dry_run {
        println!("\n{}", "(dry-run: no changes applied)".dimmed());
        return Ok(());
    }

    // env_requires_confirm is sourced from rbxplace.toml's `confirm = true` on
    // the targeted env. When `--env` isn't passed we fell back to
    // `[experience]` in rbxmeta.toml (standalone mode) and there's no env to
    // gate against, so we leave the gate off.
    let env_requires_confirm = if ctx.env().is_some() {
        PlacesFile::load(ctx.places_path())
            .ok()
            .and_then(|pf| pf.get(&env).ok().map(|e| e.confirm()))
            .unwrap_or(false)
    } else {
        false
    };
    // Both credential checks happen before the prompt, and before anything is
    // sent. A cookie problem is the user's to fix either way, and finding out
    // after typing "yes" (or worse, after half the plan has landed) is the
    // failure this ordering exists to prevent.
    let cookie = ctx.resolve_cookie();
    if plan.needs_cookie() && cookie.is_none() {
        bail!(
            "Cannot apply legacy / cookie-only fields (server_fill / allow_copying / visibility / \
             studio_access_to_apis_allowed / beta_mode / genre / avatar / \
             engine_avatar_settings / paid_access / permissions) without a cookie. \
             Pass --cookie, set RBX_COOKIE, or sign in to Roblox Studio."
        );
    }

    let client = RbxClient::new(
        ctx.api_key(),
        cookie,
        universe_id,
        place_id,
        media.bleed,
        media.language_code.clone(),
    );

    // Asked here so a dead session is a refusal the user meets before the
    // prompt rather than after it. `apply_plan` asks again, and gets the same
    // answer for free: the verdict is cached for the process, so this is one
    // round trip, not two.
    if plan.needs_cookie() {
        client.require_valid_session().await?;
    }

    // The cookie-only half of a plan is written as whichever account the
    // cookie signs in as, and an env name does not say which one that is. The
    // check above has already identified it when the plan needs a cookie, so
    // naming it here costs nothing; a key-only plan names no account, because
    // none was established.
    let account = client.known_account().await;
    confirm_destructive(
        &rbx_core::session::as_account(
            account.as_ref(),
            &format!("Apply planned changes to [{}]?", env),
        ),
        env_requires_confirm,
        yes,
    )?;

    apply_plan(
        &client,
        &plan,
        &game,
        ApplyTarget {
            env: &env,
            universe_id,
            place_id,
        },
        &mut lockfile,
        &lockfile_path,
    )
    .await
}

/// Which env the applied changes are recorded against.
///
/// A struct rather than three more parameters: `apply_plan` already takes a
/// client, a plan, a config and two pieces of lockfile, and three bare values
/// of which two are `u64` is the shape where a call site silently swaps two
/// arguments.
struct ApplyTarget<'a> {
    env: &'a str,
    universe_id: u64,
    place_id: u64,
}

/// Send the plan to Roblox, persisting each step to the lockfile as it lands.
///
/// Split out of `run` so it can be driven against a mock server: `run` resolves
/// config, builds the plan and prompts, none of which a test of the call
/// *ordering* wants to reproduce. The ordering is the reason this exists:
/// going public must activate before every other call and going private must
/// deactivate after them all, because Roblox rejects
/// `privateServerPriceRobux > 0` on a private universe. That rule lives here
/// and nowhere else, so it was untestable while this was inlined in `run`.
///
/// Each call is followed by its own `lockfile.save`, so a crash mid-sequence
/// leaves the lockfile agreeing with what Roblox actually has.
///
/// #63: the session check is the first statement, before the lockfile is even
/// touched. It is asked in `run` too, early enough to precede the prompt, but
/// the guarantee that no half of a sync can land on a dead cookie has to hold
/// here, where the writes are: a check that lives only at the caller is one
/// refactor away from being skipped. The verdict is cached per process, so
/// asking twice costs one round trip.
async fn apply_plan(
    client: &RbxClient,
    plan: &SyncPlan,
    game: &crate::config::Game,
    target: ApplyTarget<'_>,
    lockfile: &mut Lockfile,
    lockfile_path: &std::path::Path,
) -> Result<()> {
    let ApplyTarget {
        env,
        universe_id,
        place_id,
    } = target;

    if plan.needs_cookie() {
        client.require_valid_session().await?;
    }

    lockfile.version = LOCKFILE_VERSION;
    {
        let el = lockfile.env_mut(env);
        el.universe_id = universe_id;
        el.place_id = place_id;
    }

    // Activate first if going to PUBLIC. Other PATCHes (notably
    // privateServerPriceRobux > 0) require the universe to be public, so we
    // must flip visibility before sending them.
    if matches!(plan.visibility_change, Some(v) if v.is_public()) {
        println!(
            "\n{} visibility → public (first, unlocks other patches)...",
            "Setting".cyan().bold()
        );
        client.activate_universe().await?;
        lockfile.env_mut(env).game.visibility = Some(crate::config::Visibility::Public);
        lockfile.save(lockfile_path)?;
        println!("  {} visibility set to public", "✓".green());
    }

    // Apply universe patch.
    if let Some(patch) = &plan.universe_patch {
        println!("\n{} universe...", "Patching".cyan().bold());
        client
            .patch_universe(patch.body.clone(), &patch.mask)
            .await?;
        // Only the fields this call actually wrote.
        //
        // This used to assign `config_to_lock(game)` wholesale, which recorded
        // the cookie-only fields as applied too, before the legacy patch that
        // applies them had run, and regardless of whether it then failed. The
        // two blocks below already do it the narrow way; this one did not, and
        // the mismatch stopped being cosmetic when `permissions` and the
        // avatar scale tables arrived: those are write-only, so the lockfile is
        // the *only* record of them and no `pull` can ever correct it.
        let el = lockfile.env_mut(env);
        el.game.voice_chat = game.voice_chat;
        el.game.private_server = game.private_server.clone();
        el.game.devices = game.devices.clone();
        el.game.social_links = game.social_links.clone();
        lockfile.save(lockfile_path)?;
        println!("  {} universe patched", "✓".green());
    }

    // Apply place patch.
    if let Some(patch) = &plan.place_patch {
        println!("\n{} place...", "Patching".cyan().bold());
        client.patch_place(patch.body.clone(), &patch.mask).await?;
        let el = lockfile.env_mut(env);
        el.game.name = game.name.clone();
        el.game.description = game.description.clone();
        el.game.server_size = game.server_size;
        lockfile.save(lockfile_path)?;
        println!("  {} place patched", "✓".green());
    }

    // Apply legacy place patch (cookie-only fields).
    if let Some(patch) = &plan.place_legacy_patch {
        println!("\n{} place (legacy, cookie)...", "Patching".cyan().bold());
        client.patch_place_legacy(patch.body.clone()).await?;
        let el = lockfile.env_mut(env);
        el.game.server_fill = game.server_fill.clone();
        el.game.allow_copying = game.allow_copying;
        lockfile.save(lockfile_path)?;
        println!("  {} legacy place patched", "✓".green());
    }

    // Apply legacy universe-config patch (cookie-only fields).
    if let Some(patch) = &plan.universe_legacy_patch {
        println!(
            "\n{} universe config (legacy, cookie)...",
            "Patching".cyan().bold()
        );
        let response = client
            .patch_universe_config_legacy(patch.body.clone())
            .await?;
        report_engine_echo(patch, &response);
        let el = lockfile.env_mut(env);
        el.game.studio_access_to_apis_allowed = game.studio_access_to_apis_allowed;
        el.game.permissions = game.permissions;
        el.game.avatar = game.avatar;
        el.game.paid_access = game.paid_access.clone();
        el.game.genre = game.genre;
        // From the patch, not recomputed: the hash has to describe the exact
        // document that just went over the wire. `None` here means this patch
        // carried no engine settings, so whatever the lock held still stands.
        if let Some(hash) = &patch.engine_avatar_settings_hash {
            el.game.engine_avatar_settings_hash = Some(hash.clone());
        }
        lockfile.save(lockfile_path)?;
        println!("  {} legacy universe config patched", "✓".green());
    }

    // Beta mode (Experience Releases, cookie-only).
    if let Some(enabled) = plan.beta_mode_change {
        println!("\n{} beta_mode → {}...", "Setting".cyan().bold(), enabled);
        client.set_beta_mode(enabled).await?;
        lockfile.env_mut(env).game.beta_mode = Some(enabled);
        lockfile.save(lockfile_path)?;
        println!("  {} beta_mode set to {}", "✓".green(), enabled);
    }

    // Deactivate last if going to PRIVATE. Keeps the universe in its more
    // permissive state until all other patches have been applied.
    if matches!(plan.visibility_change, Some(v) if !v.is_public()) {
        println!(
            "\n{} visibility → private (last)...",
            "Setting".cyan().bold()
        );
        client.deactivate_universe().await?;
        lockfile.env_mut(env).game.visibility = Some(crate::config::Visibility::Private);
        lockfile.save(lockfile_path)?;
        println!("  {} visibility set to private", "✓".green());
    }

    // Icon.
    if let IconPlan::Upload { bytes, hash, path } = &plan.icon {
        println!("\n{} icon: {}", "Uploading".cyan().bold(), path.display());
        let resp = client.upload_icon(bytes.clone()).await?;
        lockfile.env_mut(env).media.icon = Some(MediaLock {
            hash: hash.clone(),
            image_id: resp.image_id,
        });
        lockfile.save(lockfile_path)?;
        println!(
            "  {} icon uploaded (image_id: {})",
            "✓".green(),
            resp.image_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "?".to_string())
        );
    }

    // Thumbnails. Each delete/upload is persisted to the lockfile right after
    // its API call so a crash mid-loop leaves remote and lockfile in sync.
    if !plan.thumbnails.is_empty() {
        println!("\n{} thumbnails...", "Syncing".cyan().bold());

        for image_id in &plan.thumbnails.deletes {
            println!("  Deleting thumbnail {}", image_id);
            client.delete_thumbnail(*image_id).await?;
            lockfile
                .env_mut(env)
                .media
                .thumbnails
                .retain(|m| m.image_id != Some(*image_id));
            lockfile.save(lockfile_path)?;
        }

        let mut new_image_ids: Vec<Option<u64>> = Vec::with_capacity(plan.thumbnails.uploads.len());
        for upload in &plan.thumbnails.uploads {
            println!("  Uploading {}", upload.path.display());
            let resp = client.upload_thumbnail(upload.bytes.clone()).await?;
            lockfile.env_mut(env).media.thumbnails.push(MediaLock {
                hash: upload.hash.clone(),
                image_id: resp.image_id,
            });
            lockfile.save(lockfile_path)?;
            new_image_ids.push(resp.image_id);
        }

        // Post-op order = whatever the lockfile reflects right now (deletes
        // removed, uploads appended). Compare to desired order from the plan.
        let post: Vec<u64> = lockfile
            .env_view(env)
            .media
            .thumbnails
            .iter()
            .filter_map(|m| m.image_id)
            .collect();
        let want = desired_order(&plan.thumbnails, &new_image_ids);
        if !want.is_empty() && want != post {
            println!("  Reordering thumbnails: {:?}", want);
            client.reorder_thumbnails(&want).await?;
            // Mirror the new order in the lockfile so it matches Roblox.
            let order_index: std::collections::HashMap<u64, usize> =
                want.iter().enumerate().map(|(i, id)| (*id, i)).collect();
            lockfile.env_mut(env).media.thumbnails.sort_by_key(|m| {
                m.image_id
                    .and_then(|id| order_index.get(&id))
                    .copied()
                    .unwrap_or(usize::MAX)
            });
            lockfile.save(lockfile_path)?;
        }

        println!("  {} thumbnails synced", "✓".green());
    }

    println!("\n{}", "Sync complete.".green().bold());
    Ok(())
}

/// Drop thumbnail lock entries carrying no `image_id`, returning how many went.
///
/// Such an entry records an upload whose response never came back with an id.
/// It is inert in every direction: `build_thumbnail_plan` cannot delete it
/// (there is no remote image), cannot reorder it (reordering is keyed on the
/// id), and re-uploads the file anyway when the hash still matches something
/// declared, so keeping it prevents nothing and enables nothing. What it does
/// do is survive, because `sync` only ever removed entries inside its deletes
/// loop, which is keyed on `image_id` and therefore skips exactly these.
///
/// A free function rather than inline so it can be tested: `sync::run` needs a
/// live client and this needs none.
fn prune_imageless_thumbnails(thumbnails: &mut Vec<MediaLock>) -> usize {
    let dropped = imageless_thumbnails(thumbnails);
    thumbnails.retain(|m| m.image_id.is_some());
    dropped
}

/// How many entries [`prune_imageless_thumbnails`] would drop.
///
/// Counted separately so `--dry-run` can report the number without writing,
/// and so the "nothing to do" line is not printed on a run that did something.
fn imageless_thumbnails(thumbnails: &[MediaLock]) -> usize {
    thumbnails.iter().filter(|m| m.image_id.is_none()).count()
}

fn print_plan(plan: &SyncPlan, env: &str) {
    if plan.is_empty() {
        return;
    }

    println!(
        "{} env: {}",
        "Planned changes for".bold(),
        env.cyan().bold()
    );

    if let Some(patch) = &plan.universe_patch {
        println!("\n  {} universe:", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(patch) = &plan.place_patch {
        println!("\n  {} place:", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(patch) = &plan.place_legacy_patch {
        println!("\n  {} place (legacy, cookie):", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(patch) = &plan.universe_legacy_patch {
        println!("\n  {} universe config (legacy, cookie):", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(v) = plan.visibility_change {
        println!("\n  {} visibility (cookie): → {:?}", "▸".cyan(), v);
    }

    if let Some(b) = plan.beta_mode_change {
        println!("\n  {} beta_mode (cookie): → {}", "▸".cyan(), b);
    }

    match &plan.icon {
        IconPlan::None => {}
        IconPlan::Upload { path, .. } => {
            println!(
                "\n  {} icon: upload {}",
                "▸".cyan(),
                path.display().to_string().yellow()
            );
        }
    }

    if !plan.thumbnails.is_empty() {
        println!("\n  {} thumbnails:", "▸".cyan());
        for id in &plan.thumbnails.deletes {
            println!("    • delete image {}", id);
        }
        for upload in &plan.thumbnails.uploads {
            println!(
                "    • upload {}",
                upload.path.display().to_string().yellow()
            );
        }
        if plan.thumbnails.needs_reorder {
            println!("    • reorder to match config order");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(hash: &str, image_id: Option<u64>) -> MediaLock {
        MediaLock {
            hash: hash.to_string(),
            image_id,
        }
    }

    /// #22. The entry that survived every sync: no `image_id`, no longer
    /// declared, so no plan entry could ever reach it.
    #[test]
    fn an_entry_with_no_image_id_is_pruned() {
        let mut thumbnails = vec![lock("gone", None)];

        assert_eq!(prune_imageless_thumbnails(&mut thumbnails), 1);
        assert!(thumbnails.is_empty());
    }

    #[test]
    fn entries_with_an_image_id_are_left_alone() {
        let mut thumbnails = vec![lock("a", Some(10)), lock("b", Some(20))];

        assert_eq!(prune_imageless_thumbnails(&mut thumbnails), 0);
        assert_eq!(thumbnails.len(), 2);
    }

    /// Order is the thing the lockfile carries beyond the ids themselves
    /// (`sync` mirrors Roblox's thumbnail order into it) so pruning must not
    /// disturb the entries it keeps.
    #[test]
    fn pruning_preserves_the_order_of_what_it_keeps() {
        let mut thumbnails = vec![
            lock("a", Some(10)),
            lock("dead", None),
            lock("b", Some(20)),
            lock("also-dead", None),
            lock("c", Some(30)),
        ];

        assert_eq!(prune_imageless_thumbnails(&mut thumbnails), 2);
        assert_eq!(
            thumbnails
                .iter()
                .filter_map(|m| m.image_id)
                .collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
    }

    /// A hash still declared in the config is not a reason to keep the entry:
    /// `build_thumbnail_plan` re-uploads it either way, and the fresh upload
    /// writes its own entry with the id this one lacks.
    #[test]
    fn a_matched_hash_does_not_save_an_entry_that_has_no_id() {
        let mut thumbnails = vec![lock("still-declared", None)];

        assert_eq!(prune_imageless_thumbnails(&mut thumbnails), 1);
        assert!(thumbnails.is_empty());
    }

    #[test]
    fn an_empty_lockfile_is_left_empty() {
        let mut thumbnails: Vec<MediaLock> = Vec::new();

        assert_eq!(prune_imageless_thumbnails(&mut thumbnails), 0);
        assert!(thumbnails.is_empty());
    }

    /// The count has to agree with what the prune actually removes: it is what
    /// `--dry-run` reports and what decides whether "nothing to do" is printed.
    #[test]
    fn the_count_matches_what_the_prune_removes() {
        let mut thumbnails = vec![lock("a", Some(1)), lock("x", None), lock("y", None)];

        let counted = imageless_thumbnails(&thumbnails);
        let removed = prune_imageless_thumbnails(&mut thumbnails);

        assert_eq!(counted, removed);
        assert_eq!(removed, 2);
    }

    #[test]
    fn counting_does_not_modify_anything() {
        let thumbnails = vec![lock("a", Some(1)), lock("x", None)];

        assert_eq!(imageless_thumbnails(&thumbnails), 1);
        assert_eq!(thumbnails.len(), 2, "counting must not prune");
    }
}

#[cfg(test)]
mod ordering_tests {
    use super::*;
    use crate::config::Visibility;
    use crate::diff::UniversePatch;
    use serde_json::json;
    use wiremock::matchers::path_regex;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 111;
    const PLACE: u64 = 222;

    /// Open Cloud and `develop` point at the same mock. Their paths do not
    /// overlap (`/cloud/v2/...` against `/v1/...` and `/v2/...`), so one server
    /// can record the interleaving of the two services, which is the whole
    /// point: the ordering under test spans both.
    ///
    /// The session check goes to a third server on purpose. It is a different
    /// question with a different answer, and keeping it off the recording
    /// server is what lets these tests keep asserting positions in the sequence
    /// of *writes* rather than counting a preflight into them.
    fn client(server: &MockServer, session: &MockServer, cookie: &str) -> RbxClient {
        RbxClient::new(
            Some("test-key".into()),
            Some(cookie.into()),
            UNIVERSE,
            PLACE,
            false,
            "en-us".into(),
        )
        .with_base_url(server.uri())
        .with_legacy_base_url(server.uri())
        .with_users_base_url(session.uri())
    }

    /// A `users.roblox.com` that answers `status` for the session check.
    async fn session_service(status: u16) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(path_regex("/v1/users/authenticated"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(json!({ "id": 42, "name": "tester" })),
            )
            .mount(&server)
            .await;
        server
    }

    /// Answer anything, so the sequence runs to the end and every request is
    /// recorded. What is asserted is the order they arrived in, not the
    /// responses.
    async fn accept_everything(server: &MockServer) {
        Mock::given(path_regex(".*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(server)
            .await;
    }

    async fn paths_in_order(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .expect("the mock server records requests")
            .iter()
            .map(|r| r.url.path().to_string())
            .collect()
    }

    fn universe_patch() -> Option<UniversePatch> {
        Some(UniversePatch {
            body: json!({ "displayName": "New" }),
            mask: vec!["displayName"],
            descriptions: vec!["name".into()],
        })
    }

    async fn apply(plan: &SyncPlan, server: &MockServer) -> Vec<String> {
        // A cookie unique to this call: the session verdict is cached per
        // process, and these tests run in parallel in one binary.
        let cookie = format!("live-{}", server.uri());
        let session = session_service(200).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let lockfile_path = dir.path().join(LOCKFILE_NAME);
        let mut lockfile = Lockfile::default();

        apply_plan(
            &client(server, &session, &cookie),
            plan,
            &crate::config::Game::default(),
            ApplyTarget {
                env: "test",
                universe_id: UNIVERSE,
                place_id: PLACE,
            },
            &mut lockfile,
            &lockfile_path,
        )
        .await
        .expect("apply_plan");

        paths_in_order(server).await
    }

    /// The avatar echo is read from a response that used to be discarded, so
    /// the wiring is worth one test: the run completes, and the lockfile
    /// records the hash the patch carried rather than one recomputed from a
    /// file that may have changed since.
    #[tokio::test]
    async fn an_avatar_echo_is_read_without_failing_the_sync() {
        let server = MockServer::start().await;
        // Mounted first: wiremock takes the earliest matching mock, and
        // `accept_everything` below would otherwise answer this path with `{}`.
        Mock::given(path_regex(r".*/v2/universes/.*/configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                // One key sent back, one silently dropped.
                "engineAvatarSettings": r#"{"AvatarRules":{"AvatarType":1}}"#
            })))
            .mount(&server)
            .await;
        accept_everything(&server).await;

        let plan = SyncPlan {
            universe_legacy_patch: Some(crate::diff::UniverseLegacyPatch {
                body: json!({
                    "engineAvatarSettings":
                        r#"{"AvatarRules":{"AvatarType":1,"AvatarTpye":2}}"#
                }),
                descriptions: vec!["engine_avatar_settings".into()],
                engine_avatar_settings_hash: Some("deadbeef".into()),
            }),
            ..SyncPlan::default()
        };

        let cookie = format!("live-{}", server.uri());
        let session = session_service(200).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let lockfile_path = dir.path().join(LOCKFILE_NAME);
        let mut lockfile = Lockfile::default();

        apply_plan(
            &client(&server, &session, &cookie),
            &plan,
            &crate::config::Game::default(),
            ApplyTarget {
                env: "test",
                universe_id: UNIVERSE,
                place_id: PLACE,
            },
            &mut lockfile,
            &lockfile_path,
        )
        .await
        .expect("a dropped key is a warning, not a failed sync");

        assert_eq!(
            lockfile.env_view("test").game.engine_avatar_settings_hash,
            Some("deadbeef".to_string()),
            "the hash recorded is the one the patch carried"
        );
    }

    fn position(paths: &[String], needle: &str) -> usize {
        paths
            .iter()
            .position(|p| p.contains(needle))
            .unwrap_or_else(|| panic!("{needle} was never requested; got {paths:?}"))
    }

    /// Going public activates first. Roblox rejects
    /// `privateServerPriceRobux > 0` on a private universe, so a patch sent
    /// before the activate can fail on a universe that is about to be public.
    #[tokio::test]
    async fn going_public_activates_before_every_other_call() {
        let server = MockServer::start().await;
        accept_everything(&server).await;

        let plan = SyncPlan {
            visibility_change: Some(Visibility::Public),
            universe_patch: universe_patch(),
            ..SyncPlan::default()
        };

        let paths = apply(&plan, &server).await;

        assert!(
            position(&paths, "/activate") < position(&paths, "/cloud/v2/universes"),
            "activate must come first, got {paths:?}"
        );
    }

    /// Going private deactivates last, for the mirror reason: the universe has
    /// to stay in its more permissive state until every other patch has landed.
    #[tokio::test]
    async fn going_private_deactivates_after_every_other_call() {
        let server = MockServer::start().await;
        accept_everything(&server).await;

        let plan = SyncPlan {
            visibility_change: Some(Visibility::Private),
            universe_patch: universe_patch(),
            ..SyncPlan::default()
        };

        let paths = apply(&plan, &server).await;

        assert!(
            position(&paths, "/deactivate") > position(&paths, "/cloud/v2/universes"),
            "deactivate must come last, got {paths:?}"
        );
    }

    /// The two are not symmetric by accident, and a refactor that treated
    /// visibility as "one more patch in the list" would break exactly one of
    /// them. Asserting both against the same plan shape keeps that visible.
    #[tokio::test]
    async fn the_two_directions_put_the_visibility_call_on_opposite_sides() {
        let server = MockServer::start().await;
        accept_everything(&server).await;
        let public = apply(
            &SyncPlan {
                visibility_change: Some(Visibility::Public),
                universe_patch: universe_patch(),
                ..SyncPlan::default()
            },
            &server,
        )
        .await;

        let other = MockServer::start().await;
        accept_everything(&other).await;
        let private = apply(
            &SyncPlan {
                visibility_change: Some(Visibility::Private),
                universe_patch: universe_patch(),
                ..SyncPlan::default()
            },
            &other,
        )
        .await;

        assert_eq!(position(&public, "/activate"), 0, "{public:?}");
        assert_eq!(
            position(&private, "/deactivate"),
            private.len() - 1,
            "{private:?}"
        );
    }

    /// No visibility change means neither endpoint is touched. Sending one
    /// anyway would be a write to a live universe that nothing asked for.
    #[tokio::test]
    async fn no_visibility_change_touches_neither_endpoint() {
        let server = MockServer::start().await;
        accept_everything(&server).await;

        let plan = SyncPlan {
            universe_patch: universe_patch(),
            ..SyncPlan::default()
        };

        let paths = apply(&plan, &server).await;

        assert!(
            !paths
                .iter()
                .any(|p| p.contains("/activate") || p.contains("/deactivate")),
            "got {paths:?}"
        );
    }

    /// #63, the case the issue is about.
    ///
    /// A plan with both halves (an Open Cloud universe patch and the
    /// cookie-only legacy patch, visibility and beta mode) against a session
    /// Roblox refuses. Before the check existed this landed the Open Cloud half
    /// and then failed on the first legacy call, leaving a live universe
    /// half-updated until somebody re-ran with a fresh cookie.
    ///
    /// What is asserted is not "it fails" but "nothing was sent": the mock
    /// recording every write endpoint must have received no request at all, and
    /// the lockfile must not exist, because a lockfile written for changes that
    /// never landed is the other half of the same bug.
    #[tokio::test]
    async fn a_dead_cookie_applies_neither_half_of_a_sync() {
        let server = MockServer::start().await;
        accept_everything(&server).await;
        let session = session_service(401).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let lockfile_path = dir.path().join(LOCKFILE_NAME);
        let mut lockfile = Lockfile::default();

        let plan = SyncPlan {
            universe_patch: universe_patch(),
            place_legacy_patch: Some(crate::diff::PlaceLegacyPatch {
                body: json!({ "allowCopying": true }),
                descriptions: vec!["allow_copying".into()],
            }),
            visibility_change: Some(Visibility::Public),
            beta_mode_change: Some(true),
            ..SyncPlan::default()
        };

        let error = apply_plan(
            &client(&server, &session, "dead-cookie"),
            &plan,
            &crate::config::Game::default(),
            ApplyTarget {
                env: "test",
                universe_id: UNIVERSE,
                place_id: PLACE,
            },
            &mut lockfile,
            &lockfile_path,
        )
        .await
        .expect_err("an expired session must stop the apply");

        let message = error.to_string();
        assert!(message.contains("expired"), "got {message}");
        assert!(message.contains("RBX_COOKIE"), "got {message}");

        assert!(
            paths_in_order(&server).await.is_empty(),
            "nothing may be applied on a refused session, got {:?}",
            paths_in_order(&server).await
        );
        assert!(
            !lockfile_path.exists(),
            "the lockfile must not record work that never happened"
        );
    }

    /// The mirror: a plan with no cookie-only field is pure Open Cloud, so it
    /// must not spend a round trip proving a session it does not use. This is
    /// the "only where it writes with the cookie" rule, asserted where it is
    /// cheapest to break.
    #[tokio::test]
    async fn an_open_cloud_only_plan_never_asks_about_the_session() {
        let server = MockServer::start().await;
        accept_everything(&server).await;
        let session = session_service(401).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lockfile = Lockfile::default();

        apply_plan(
            &client(&server, &session, "unused-cookie"),
            &SyncPlan {
                universe_patch: universe_patch(),
                ..SyncPlan::default()
            },
            &crate::config::Game::default(),
            ApplyTarget {
                env: "test",
                universe_id: UNIVERSE,
                place_id: PLACE,
            },
            &mut lockfile,
            &dir.path().join(LOCKFILE_NAME),
        )
        .await
        .expect("an Open Cloud plan does not depend on the cookie");

        assert!(
            session.received_requests().await.unwrap().is_empty(),
            "a plan that writes nothing with the cookie must not check it"
        );
    }

    /// A session check that could not run is not a refusal. Roblox being
    /// unreachable is a fault the writes themselves will report; turning it
    /// into "your session expired" sends somebody to re-authenticate a session
    /// nobody has shown to be dead.
    #[tokio::test]
    async fn an_unreachable_session_service_does_not_stop_the_apply() {
        let server = MockServer::start().await;
        accept_everything(&server).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lockfile = Lockfile::default();

        let client = RbxClient::new(
            Some("test-key".into()),
            Some("offline-cookie".into()),
            UNIVERSE,
            PLACE,
            false,
            "en-us".into(),
        )
        .with_base_url(server.uri())
        .with_legacy_base_url(server.uri())
        .with_users_base_url("http://127.0.0.1:1");

        apply_plan(
            &client,
            &SyncPlan {
                visibility_change: Some(Visibility::Public),
                universe_patch: universe_patch(),
                ..SyncPlan::default()
            },
            &crate::config::Game::default(),
            ApplyTarget {
                env: "test",
                universe_id: UNIVERSE,
                place_id: PLACE,
            },
            &mut lockfile,
            &dir.path().join(LOCKFILE_NAME),
        )
        .await
        .expect("an unanswered check is not a refused session");

        assert!(
            !paths_in_order(&server).await.is_empty(),
            "the apply must have gone ahead"
        );
    }
}

/// Say what Roblox did with the avatar document, if it was sent one.
///
/// Printed after the write rather than gating it: by the time there is an echo
/// to read, the write has landed. See `crate::engine_echo` for why a
/// difference is a warning and not an error.
fn report_engine_echo(patch: &crate::diff::UniverseLegacyPatch, response: &str) {
    let Some(sent) = patch
        .body
        .get("engineAvatarSettings")
        .and_then(|v| v.as_str())
    else {
        return;
    };
    let Some(echo) = crate::engine_echo::compare(sent, response) else {
        // Measured against a live universe on 2026-08-17: this is what actually
        // happens. The specification says the PATCH answers with
        // `UniverseSettingsResponseV2`, `engineAvatarSettings` included; the
        // endpoint returned nothing usable, and a deliberately misspelled key
        // went unreported.
        //
        // So the check says it could not run, rather than saying nothing and
        // leaving a reader to assume the document was verified. The byte count
        // is the diagnosis: `0 bytes` means an empty response, anything else
        // means a body that does not carry the field, and the two want
        // different fixes.
        println!(
            "{}",
            format!(
                "    Roblox returned no avatar echo to check against ({} bytes); \
                 misspelled keys cannot be reported.",
                response.len()
            )
            .dimmed()
        );
        return;
    };

    if !echo.dropped.is_empty() {
        println!(
            "  {} Roblox did not keep {} avatar key{}, {} not applied:",
            "!".yellow(),
            echo.dropped.len(),
            if echo.dropped.len() == 1 { "" } else { "s" },
            if echo.dropped.len() == 1 {
                "it was"
            } else {
                "they were"
            },
        );
        for key in &echo.dropped {
            println!("      {}", key.yellow());
        }
        println!(
            "{}",
            "    A misspelling is the usual cause. The rest of the document applied.".dimmed()
        );
    }

    if !echo.added.is_empty() {
        // Lower volume than a drop, and on purpose: Roblox completing a partial
        // document is the normal case, and the only reason to mention it is
        // that it is how somebody learns the full shape without guessing.
        println!(
            "{}",
            format!(
                "    Roblox filled in {} default{}: {}",
                echo.added.len(),
                if echo.added.len() == 1 { "" } else { "s" },
                echo.added.join(", ")
            )
            .dimmed()
        );
    }
}
