//! The `pull` tests, moved out of the command file without a line of them
//! changing.
//!
//! They stay inside the crate rather than becoming an integration test, and
//! both shapes below say why. The round trips build a `ShopCtx` with
//! `base_url` set, a `cfg(test)` field on a type that lives in a private
//! module; the rest call `compute_config_changes` and `apply_config_changes`,
//! which are private to `pull` itself. A test in `tests/` can reach neither,
//! and the only way to move them there would be to rewrite what they assert.

#![allow(clippy::unwrap_used)]
use super::*;
use crate::config::PassConfig;
use rbx_core::GlobalFlags;
use serde_json::json;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── the round trip through a real pull ──

const UNIVERSE: u64 = 66778899001;

/// Comments, a top-level table rbx does not model, and a stray key inside a
/// resource. All three used to be gone after any pull.
const USER_WRITTEN: &str = r#"# Our shop. Prices are agreed with finance — ask before editing.

# Read by our deploy script, not by rbx.
[deploy]
channel = "beta"

[experience]
universe_id = 66778899001

# The flagship pass.
[passes.VIP]
price = 499
internal_note = "renewal negotiated 2026-03"
"#;

struct Pulled {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Pulled {
    fn new(contents: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("rbxshop.toml");
        std::fs::write(&config, contents).unwrap();
        Self { dir, config }
    }

    fn contents(&self) -> String {
        std::fs::read_to_string(&self.config).unwrap()
    }

    /// Path of a sibling file in the same directory as the config.
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.path(name), contents).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).unwrap()
    }

    /// Seed the `default` env's lockfile section, as a prior pull or sync
    /// would have left it. Built from the model rather than from TOML: the
    /// point of these tests is what `pull` does with the section, not how
    /// it parses.
    fn write_lock(&self, env_lock: EnvLock) {
        Lockfile {
            version: LOCKFILE_VERSION,
            envs: BTreeMap::from([(DEFAULT_ENV.to_string(), env_lock)]),
        }
        .save(&self.path(LOCKFILE_NAME))
        .unwrap();
    }

    fn env_lock(&self) -> EnvLock {
        self.env_lock_named(DEFAULT_ENV)
    }

    fn env_lock_named(&self, env: &str) -> EnvLock {
        Lockfile::load(&self.path(LOCKFILE_NAME))
            .unwrap()
            .env(env)
            .cloned()
            .unwrap()
    }

    /// No `--env`: standalone mode, so the pull writes base rather than an
    /// env overlay.
    async fn pull(&self, server: &MockServer) -> Result<()> {
        self.run_pull(server, None, false, false).await
    }

    async fn pull_accepting(
        &self,
        server: &MockServer,
        accept_remote: bool,
        accept_local: bool,
    ) -> Result<()> {
        self.run_pull(server, None, accept_remote, accept_local)
            .await
    }

    /// `--env <name>`, resolved through the `rbxplace.toml` the caller
    /// wrote next to the config. Divergence from base lands in that env's
    /// overlay rather than on the base entry.
    async fn pull_env(&self, server: &MockServer, env: &str) -> Result<()> {
        self.run_pull(server, Some(env), false, false).await
    }

    async fn run_pull(
        &self,
        server: &MockServer,
        env: Option<&str>,
        accept_remote: bool,
        accept_local: bool,
    ) -> Result<()> {
        let global = GlobalFlags {
            api_key: Some("test-key".into()),
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: env.map(str::to_string),
            place: None,
            places: self.dir.path().join("rbxplace.toml"),
            universe_id: None,
            place_id: Vec::new(),
        };
        let ctx = ShopCtx {
            config: self.config.clone(),
            global: &global,
            base_url: Some(server.uri()),
        };
        run(&ctx, false, accept_remote, accept_local, true).await
    }
}

/// The three catalogue endpoints a pull always reads, each answered once
/// with the given array. `expect(1)` on all three: a pull of one env that
/// listed a catalogue twice would be a regression whether or not the
/// second answer agreed with the first.
async fn mount_lists(
    server: &MockServer,
    passes: serde_json::Value,
    badges: serde_json::Value,
    products: serde_json::Value,
) {
    Mock::given(method("GET"))
        .and(path_matcher(format!(
            "/game-passes/v1/universes/{UNIVERSE}/game-passes/creator"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "gamePasses": passes,
            "nextPageToken": ""
        })))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher(format!("/v1/universes/{UNIVERSE}/badges")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": badges,
            "nextPageCursor": ""
        })))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher(format!(
            "/developer-products/v2/universes/{UNIVERSE}/developer-products/creator"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "developerProducts": products,
            "nextPageToken": ""
        })))
        .expect(1)
        .mount(server)
        .await;
}

/// Remote state for one pass, and empty badge/product catalogues.
async fn mount_remote(server: &MockServer, price: u64) {
    mount_lists(
        server,
        json!([{
            "gamePassId": 111,
            "name": "VIP",
            "isForSale": true,
            "priceInformation": { "defaultPriceInRobux": price }
        }]),
        json!([]),
        json!([]),
    )
    .await;
}

/// How many requests the server received whose path is exactly `path`.
async fn count(server: &MockServer, path: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == path)
        .count()
}

/// The headline case, and the one that used to be worst: a pull that finds
/// nothing to change still rewrites the config file. Under the old serde
/// round trip that was enough to strip every comment in it.
#[tokio::test]
async fn a_pull_with_nothing_to_change_leaves_the_file_byte_identical() {
    let shop = Pulled::new(USER_WRITTEN);
    let server = MockServer::start().await;
    mount_remote(&server, 499).await;

    shop.pull(&server).await.unwrap();

    assert_eq!(shop.contents(), USER_WRITTEN);
}

/// And a pull that *does* change something changes only that.
#[tokio::test]
async fn a_pull_that_updates_a_price_keeps_comments_and_unmodeled_keys() {
    let shop = Pulled::new(USER_WRITTEN);
    let server = MockServer::start().await;
    mount_remote(&server, 999).await;

    shop.pull(&server).await.unwrap();

    let after = shop.contents();
    assert!(after.contains("price = 999"), "{after}");
    assert!(!after.contains("price = 499"));

    // The comments.
    assert!(
        after.contains("# Our shop. Prices are agreed with finance"),
        "{after}"
    );
    assert!(after.contains("# Read by our deploy script, not by rbx."));
    assert!(after.contains("# The flagship pass."));
    // The table rbx does not model.
    assert!(after.contains("[deploy]"));
    assert!(after.contains(r#"channel = "beta""#));
    // The key inside a resource that rbx does not model.
    assert!(after.contains(r#"internal_note = "renewal negotiated 2026-03""#));

    // The pull really ran: the lockfile now tracks the remote id.
    let lock = Lockfile::load(&shop.dir.path().join(LOCKFILE_NAME)).unwrap();
    assert_eq!(lock.env(DEFAULT_ENV).unwrap().passes["VIP"].id, 111);
}

/// A dry run reports and writes nothing at all.
#[tokio::test]
async fn a_dry_run_pull_writes_neither_config_nor_lockfile() {
    let shop = Pulled::new(USER_WRITTEN);
    let server = MockServer::start().await;
    mount_remote(&server, 999).await;

    let global = GlobalFlags {
        api_key: Some("test-key".into()),
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: None,
        place: None,
        places: shop.dir.path().join("rbxplace.toml"),
        universe_id: None,
        place_id: Vec::new(),
    };
    let ctx = ShopCtx {
        config: shop.config.clone(),
        global: &global,
        base_url: Some(server.uri()),
    };
    run(&ctx, true, false, false, true).await.unwrap();

    assert_eq!(shop.contents(), USER_WRITTEN);
    assert!(!shop.dir.path().join(LOCKFILE_NAME).exists());
}

// ── remote resolution ──

/// A config declaring the pass the mocked catalogue returns.
const ONE_PASS: &str = "\
[experience]
universe_id = 66778899001

[passes.VIP]
price = 499
";

/// A badge with no `enabled` of its own, so the default (`true`) is what
/// the remote gets compared against.
const ONE_BADGE: &str = "\
[experience]
universe_id = 66778899001

[badges.Welcome]
";

fn locked_pass(id: u64, name: &str, price: u64) -> PassLock {
    PassLock {
        id,
        name: name.to_string(),
        price: Some(price),
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
    }
}

fn locked_badge(id: u64, name: &str, enabled: bool) -> BadgeLock {
    BadgeLock {
        id,
        name: name.to_string(),
        description: None,
        enabled,
        icon_asset_id: None,
        icon_hash: None,
    }
}

fn one_locked_pass(lock: PassLock) -> EnvLock {
    EnvLock {
        universe_id: UNIVERSE,
        passes: BTreeMap::from([("VIP".to_string(), lock)]),
        ..Default::default()
    }
}

/// A resource is tracked by its id, not by its display name. Renaming a
/// pass on the website has to update the entry the lockfile already points
/// at — keying off the new name instead would strand the old key and add a
/// duplicate under the new one.
#[tokio::test]
async fn a_pass_renamed_remotely_updates_the_entry_its_id_points_at() {
    let shop = Pulled::new(ONE_PASS);
    shop.write_lock(one_locked_pass(locked_pass(111, "VIP", 499)));
    let server = MockServer::start().await;
    mount_lists(
        &server,
        json!([{
            "gamePassId": 111,
            "name": "Ultra VIP",
            "isForSale": true,
            "priceInformation": { "defaultPriceInRobux": 499 }
        }]),
        json!([]),
        json!([]),
    )
    .await;

    shop.pull(&server).await.unwrap();

    let env = shop.env_lock();
    assert_eq!(env.passes.keys().collect::<Vec<_>>(), vec!["VIP"]);
    assert_eq!(env.passes["VIP"].name, "Ultra VIP");

    let after = shop.contents();
    assert!(after.contains("[passes.VIP]"), "{after}");
    assert!(after.contains(r#"name = "Ultra VIP""#), "{after}");
}

/// The list endpoint does not report regional pricing at all, so a pull
/// that trusted it would clobber what `sync` recorded and show a phantom
/// diff on every subsequent run.
#[tokio::test]
async fn regional_pricing_survives_a_pull_that_cannot_observe_it() {
    let shop = Pulled::new(ONE_PASS);
    shop.write_lock(one_locked_pass(PassLock {
        regional_pricing: true,
        ..locked_pass(111, "VIP", 499)
    }));
    let server = MockServer::start().await;
    mount_remote(&server, 499).await;

    shop.pull(&server).await.unwrap();

    assert!(shop.env_lock().passes["VIP"].regional_pricing);
    assert_eq!(shop.contents(), ONE_PASS);
}

/// Disabled badges stopped coming back from the list endpoint in Aug 2024.
/// Reading that silence as "deleted" would drop a live badge from the
/// lockfile, and the next sync would create a second one.
#[tokio::test]
async fn a_badge_missing_from_the_list_is_refetched_rather_than_reported_removed() {
    let shop = Pulled::new(ONE_BADGE);
    shop.write_lock(EnvLock {
        universe_id: UNIVERSE,
        badges: BTreeMap::from([("Welcome".to_string(), locked_badge(222, "Welcome", true))]),
        ..Default::default()
    });
    let server = MockServer::start().await;
    mount_lists(&server, json!([]), json!([]), json!([])).await;
    Mock::given(method("GET"))
        .and(path_matcher("/v1/badges/222"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 222,
            "name": "Welcome",
            "enabled": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    shop.pull(&server).await.unwrap();

    let env = shop.env_lock();
    assert_eq!(env.badges["Welcome"].id, 222);
    assert!(!env.badges["Welcome"].enabled);

    let after = shop.contents();
    assert!(after.contains("enabled = false"), "{after}");
}

/// The other side of the same branch: when the individual fetch cannot
/// find it either, the badge really is gone and the entry goes with it.
#[tokio::test]
async fn a_badge_the_individual_fetch_cannot_find_either_is_dropped() {
    let shop = Pulled::new(ONE_BADGE);
    shop.write_lock(EnvLock {
        universe_id: UNIVERSE,
        badges: BTreeMap::from([("Welcome".to_string(), locked_badge(222, "Welcome", true))]),
        ..Default::default()
    });
    let server = MockServer::start().await;
    mount_lists(&server, json!([]), json!([]), json!([])).await;
    Mock::given(method("GET"))
        .and(path_matcher("/v1/badges/222"))
        .respond_with(ResponseTemplate::new(404).set_body_string("badge not found"))
        .expect(1)
        .mount(&server)
        .await;

    shop.pull(&server).await.unwrap();

    assert!(shop.env_lock().badges.is_empty());
}

/// The third case, which used to share a path with the second: the fetch
/// failed, so nothing was learned about the badge. Dropping it would make
/// the next `sync` create a duplicate on a live universe, and a Roblox
/// badge cannot be deleted — so the pull aborts and leaves the lockfile
/// exactly as it found it.
///
/// 403 rather than 500 because a 5xx is retried, and the assertion is
/// about the branch, not about the backoff schedule.
#[tokio::test]
async fn a_badge_whose_refetch_fails_aborts_the_pull_instead_of_dropping_it() {
    let shop = Pulled::new(ONE_BADGE);
    shop.write_lock(EnvLock {
        universe_id: UNIVERSE,
        badges: BTreeMap::from([("Welcome".to_string(), locked_badge(222, "Welcome", true))]),
        ..Default::default()
    });
    let before = shop.read(LOCKFILE_NAME);
    let server = MockServer::start().await;
    // Mounted one by one rather than through `mount_lists`: the pull is
    // meant to stop at the refetch, so the product catalogue is never
    // listed and that helper's `expect(1)` would fail on a working fix.
    Mock::given(method("GET"))
        .and(path_matcher(format!(
            "/game-passes/v1/universes/{UNIVERSE}/game-passes/creator"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "gamePasses": [],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher(format!("/v1/universes/{UNIVERSE}/badges")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [],
            "nextPageCursor": ""
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/v1/badges/222"))
        .respond_with(ResponseTemplate::new(403).set_body_string("api key expired"))
        .mount(&server)
        .await;

    let error = shop.pull(&server).await.unwrap_err();

    // The badge is named, so the user knows which one to check.
    let rendered = format!("{error:#}");
    assert!(rendered.contains("Welcome"), "{rendered}");
    assert!(rendered.contains("222"), "{rendered}");
    assert!(rendered.contains("duplicate"), "{rendered}");

    // And nothing was written: the entry is still there to be retried.
    assert_eq!(shop.read(LOCKFILE_NAME), before);
    assert_eq!(shop.env_lock().badges["Welcome"].id, 222);
    assert_eq!(shop.contents(), ONE_BADGE);
}

// ── icon persistence ──

/// Arbitrary bytes: this path writes the asset to disk and blake3-hashes
/// it, and decodes nothing — unlike the upload path in `sync`.
const ICON_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n-- stand-in asset --";

/// Two hops: the thumbnails service answers with a CDN url, and the bytes
/// come from there. Both have to happen exactly once, and the result
/// has to land in three places — the file, the lockfile hash, and the
/// config's `icon` key.
#[tokio::test]
async fn accept_remote_downloads_the_icon_and_records_it_in_config_and_lockfile() {
    let shop = Pulled::new(ONE_PASS);
    shop.write_lock(one_locked_pass(locked_pass(111, "VIP", 499)));
    let server = MockServer::start().await;
    mount_lists(
        &server,
        json!([{
            "gamePassId": 111,
            "name": "VIP",
            "isForSale": true,
            "iconAssetId": 900,
            "priceInformation": { "defaultPriceInRobux": 499 }
        }]),
        json!([]),
        json!([]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path_matcher("/v1/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "state": "Completed",
                "imageUrl": format!("{}/cdn/900.png", server.uri())
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/cdn/900.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(ICON_BYTES))
        .expect(1)
        .mount(&server)
        .await;

    shop.pull_accepting(&server, true, false).await.unwrap();

    assert_eq!(
        std::fs::read(shop.path("icons/pass-111-VIP.png")).unwrap(),
        ICON_BYTES
    );
    assert_eq!(
        shop.env_lock().passes["VIP"].icon_hash,
        Some(rbx_core::image::hash_bytes(ICON_BYTES))
    );
    let after = shop.contents();
    assert!(
        after.contains(r#"icon = "icons/pass-111-VIP.png""#),
        "{after}"
    );
}

/// `--accept-local` is the opposite decision, and it is expressed by
/// clearing the hash: that is what makes the next sync re-upload. Nothing
/// is fetched.
#[tokio::test]
async fn accept_local_clears_the_hash_and_downloads_nothing() {
    let shop = Pulled::new(ONE_PASS);
    shop.write_lock(one_locked_pass(PassLock {
        icon_asset_id: Some(900),
        icon_hash: Some("a-hash-from-the-last-sync".to_string()),
        ..locked_pass(111, "VIP", 499)
    }));
    let server = MockServer::start().await;
    mount_lists(
        &server,
        json!([{
            "gamePassId": 111,
            "name": "VIP",
            "isForSale": true,
            "iconAssetId": 901,
            "priceInformation": { "defaultPriceInRobux": 499 }
        }]),
        json!([]),
        json!([]),
    )
    .await;

    shop.pull_accepting(&server, false, true).await.unwrap();

    assert_eq!(shop.env_lock().passes["VIP"].icon_hash, None);
    assert_eq!(count(&server, "/v1/assets").await, 0);
}

const PASS_WITH_ICON: &str = "\
[experience]
universe_id = 66778899001

[passes.VIP]
price = 499
icon = \"vip.png\"
";

/// With neither flag, an icon that changed on both sides is a question the
/// tool refuses to answer for you — and refusing has to mean *nothing* was
/// written, config and lockfile alike.
#[tokio::test]
async fn an_icon_conflict_aborts_the_pull_before_anything_is_written() {
    let shop = Pulled::new(PASS_WITH_ICON);
    shop.write("vip.png", "local icon bytes");
    shop.write_lock(one_locked_pass(PassLock {
        icon_asset_id: Some(900),
        ..locked_pass(111, "VIP", 499)
    }));
    let lock_before = shop.read(LOCKFILE_NAME);
    let server = MockServer::start().await;
    mount_lists(
        &server,
        json!([{
            "gamePassId": 111,
            "name": "VIP",
            "isForSale": true,
            "iconAssetId": 901,
            "priceInformation": { "defaultPriceInRobux": 499 }
        }]),
        json!([]),
        json!([]),
    )
    .await;

    let err = shop.pull(&server).await.unwrap_err().to_string();
    assert!(err.contains("Icon conflicts detected"), "{err}");

    assert_eq!(shop.contents(), PASS_WITH_ICON);
    assert_eq!(shop.read(LOCKFILE_NAME), lock_before);
    assert_eq!(count(&server, "/v1/assets").await, 0);
}

// ── env overlays ──

const BASE_AND_DEV_OVERLAY: &str = "\
[experience]
universe_id = 66778899001

# Priced down while we are testing.
[passes.VIP]
price = 499

[envs.dev.passes.VIP]
price = 1
";

/// The dev universe has caught up with base, so the overlay recording the
/// divergence is noise. Dropping the entry is not enough on its own: the
/// `[envs.dev]` table it lived in has to go too, or the next load reads the
/// overlay straight back in.
#[tokio::test]
async fn an_overlay_that_no_longer_diverges_leaves_the_file_with_it() {
    let shop = Pulled::new(BASE_AND_DEV_OVERLAY);
    shop.write(
        "rbxplace.toml",
        &format!("[dev]\nuniverse_id = {UNIVERSE}\n"),
    );
    let server = MockServer::start().await;
    mount_remote(&server, 499).await;

    shop.pull_env(&server, "dev").await.unwrap();

    let after = shop.contents();
    assert!(!after.contains("[envs.dev"), "{after}");
    // Base and its comment are none of the overlay's business.
    assert!(
        after.contains("# Priced down while we are testing."),
        "{after}"
    );
    assert!(after.contains("price = 499"), "{after}");

    assert_eq!(shop.env_lock_named("dev").passes["VIP"].id, 111);
}

// ── multi-file configs ──

const MAIN_WITH_INCLUDE: &str = "\
[experience]
universe_id = 66778899001

[include]
files = [\"rbxshop.passes.toml\"]
";

const INCLUDED_PASSES: &str = "\
# Passes live here so the store team owns one file.
[passes.VIP]
price = 499
internal_note = \"renewal negotiated 2026-03\"
";

/// The write has to reach the file that declares the entry. Every file is
/// rewritten on a pull, so the main one doubles as the check that the
/// update did not get duplicated into it.
#[tokio::test]
async fn a_price_change_is_written_to_the_included_file_that_declares_the_entry() {
    let shop = Pulled::new(MAIN_WITH_INCLUDE);
    shop.write("rbxshop.passes.toml", INCLUDED_PASSES);
    let server = MockServer::start().await;
    mount_remote(&server, 999).await;

    shop.pull(&server).await.unwrap();

    let included = shop.read("rbxshop.passes.toml");
    assert!(included.contains("price = 999"), "{included}");
    assert!(included.contains("# Passes live here"), "{included}");
    assert!(included.contains("internal_note"), "{included}");

    assert_eq!(shop.contents(), MAIN_WITH_INCLUDE);
    assert_eq!(shop.env_lock().passes["VIP"].id, 111);
}

fn gift_enabled_pass() -> PassConfig {
    PassConfig {
        name: None,
        price: Some(499),
        description: None,
        icon: None,
        for_sale: true,
        regional_pricing: false,
        create_gift: true,
        path: None,
    }
}

fn gift_product_lock() -> ProductLock {
    ProductLock {
        id: 2,
        name: "[GIFT] VIP".into(),
        price: 499,
        description: None,
        icon_asset_id: None,
        icon_hash: None,
        for_sale: true,
        regional_pricing: false,
        store_page: false,
    }
}

fn config_with_gift_pass() -> Config {
    Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::from([("VIP".to_string(), gift_enabled_pass())]),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs: BTreeMap::new(),
    }
}

#[test]
fn compute_product_config_changes_skips_gift_twin() {
    let config = config_with_gift_pass();
    let product_locks = BTreeMap::from([("GiftVIP".to_string(), gift_product_lock())]);
    let changes = compute_config_changes::<ProductKind>(&config, "default", true, &product_locks);
    assert!(
        changes.is_empty(),
        "gift twin should never be reported as a config change: {changes:?}"
    );
}

#[test]
fn apply_product_config_changes_never_materializes_gift_twin() {
    let config = config_with_gift_pass();
    let product_locks = BTreeMap::from([("GiftVIP".to_string(), gift_product_lock())]);
    let mut files = vec![ConfigFile {
        path: PathBuf::from("rbxshop.toml"),
        config,
    }];
    let merged = Config::merge_loaded(&files).unwrap();
    apply_config_changes::<ProductKind>(&mut files, &merged, "default", true, &product_locks);
    assert!(
        !files[0].config.products.contains_key("GiftVIP"),
        "pull must never write the derived gift twin into rbxshop.toml as a real entry"
    );
}

#[test]
fn unrelated_product_named_like_a_gift_key_is_still_pulled_normally() {
    // "GiftCard" isn't derived from anything — "Card" doesn't exist as a
    // gift-enabled source — so pull should treat it as a normal new product.
    let config = config_with_gift_pass();
    let product_locks = BTreeMap::from([("GiftCard".to_string(), gift_product_lock())]);
    let changes = compute_config_changes::<ProductKind>(&config, "default", true, &product_locks);
    assert_eq!(changes.len(), 1);
}

fn pass_lock(price: u64) -> PassLock {
    locked_pass(1, "VIP", price)
}

fn pass_config(price: u64) -> PassConfig {
    PassConfig {
        name: None,
        price: Some(price),
        description: None,
        icon: None,
        for_sale: true,
        regional_pricing: false,
        create_gift: false,
        path: None,
    }
}

#[test]
fn apply_pass_config_changes_updates_base_in_its_owning_file() {
    // "VIP" lives in an included file, not main — the update must land
    // there, not silently re-create it in the main file.
    let main = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::new(),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs: BTreeMap::new(),
    };
    let included = Config {
        passes: BTreeMap::from([("VIP".to_string(), pass_config(499))]),
        ..main.clone()
    };
    let mut files = vec![
        ConfigFile {
            path: PathBuf::from("rbxshop.toml"),
            config: main,
        },
        ConfigFile {
            path: PathBuf::from("rbxshop.passes.toml"),
            config: included,
        },
    ];

    let locks = BTreeMap::from([("VIP".to_string(), pass_lock(999))]);
    let merged = Config::merge_loaded(&files).unwrap();
    apply_config_changes::<PassKind>(&mut files, &merged, "default", true, &locks);

    assert_eq!(files[1].config.passes["VIP"].price, Some(999));
    assert!(!files[0].config.passes.contains_key("VIP"));
}

#[test]
fn apply_pass_config_changes_adds_new_overlay_next_to_its_base() {
    // No overlay exists yet for "prod" — the new one should be created
    // in the same file as the base ("VIP" lives in the included file),
    // not in main.
    let main = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::new(),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs: BTreeMap::new(),
    };
    let included = Config {
        passes: BTreeMap::from([("VIP".to_string(), pass_config(499))]),
        ..main.clone()
    };
    let mut files = vec![
        ConfigFile {
            path: PathBuf::from("rbxshop.toml"),
            config: main,
        },
        ConfigFile {
            path: PathBuf::from("rbxshop.passes.toml"),
            config: included,
        },
    ];

    let locks = BTreeMap::from([("VIP".to_string(), pass_lock(999))]);
    let merged = Config::merge_loaded(&files).unwrap();
    apply_config_changes::<PassKind>(&mut files, &merged, "prod", false, &locks);

    assert!(files[1]
        .config
        .envs
        .get("prod")
        .is_some_and(|ov| ov.passes.contains_key("VIP")));
    assert!(!files[0].config.envs.contains_key("prod"));
}

#[test]
fn apply_pass_config_changes_new_base_entry_defaults_to_main_file() {
    let main = Config {
        experience: None,
        owner: None,
        codegen: Default::default(),
        icons: Default::default(),
        gifts: Default::default(),
        include: Default::default(),
        passes: BTreeMap::new(),
        badges: BTreeMap::new(),
        products: BTreeMap::new(),
        envs: BTreeMap::new(),
    };
    let included = main.clone();
    let mut files = vec![
        ConfigFile {
            path: PathBuf::from("rbxshop.toml"),
            config: main,
        },
        ConfigFile {
            path: PathBuf::from("rbxshop.passes.toml"),
            config: included,
        },
    ];

    let locks = BTreeMap::from([("NewPass".to_string(), pass_lock(100))]);
    let merged = Config::merge_loaded(&files).unwrap();
    apply_config_changes::<PassKind>(&mut files, &merged, "default", true, &locks);

    assert!(files[0].config.passes.contains_key("NewPass"));
    assert!(!files[1].config.passes.contains_key("NewPass"));
}
