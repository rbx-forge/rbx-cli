//! The three `apply_*` paths, over HTTP against a mock server.
//!
//! These are the calls that create paid products on a real account, so the
//! tests are as much about what `sync` refuses to send as about what it sends:
//! nothing at all on `--dry-run`, no icon upload when the icon has not
//! changed, no metadata patch when only the icon has.
//!
//! Everything is driven through `run` rather than by calling `apply_*`
//! directly. That keeps the assertions pinned to observable behaviour — the
//! requests on the wire and the resulting lockfile — so they stay meaningful
//! when the dispatch behind them is reorganised.
//!
//! Moved out of `sync.rs` without a line of them changing, and moved here
//! rather than into `tests/`: driving `run` means building a `ShopCtx` with
//! `base_url` set, and that field is `cfg(test)` on a type in a private
//! module, so an integration test has no way to reach the mock server.

#![allow(clippy::unwrap_used)]

use super::*;
use crate::ctx::ShopCtx;
use rbx_core::GlobalFlags;
use serde_json::json;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const UNIVERSE: u64 = 66778899001;
const ENV: &str = "dev";

/// A 1×1 RGBA PNG. Icon uploads run the file through `image::load_from_memory`,
/// so the bytes have to decode; the diff hashes the file as it sits on disk,
/// so the content only has to be stable.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc, 0xcf, 0xc0, 0x50,
    0x0f, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xa9, 0x8c, 0x21, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

// ── paths ──

fn pass_collection() -> String {
    format!("/game-passes/v1/universes/{UNIVERSE}/game-passes")
}
fn pass_item(id: u64) -> String {
    format!("{}/{id}", pass_collection())
}
fn badge_collection() -> String {
    format!("/legacy-badges/v1/universes/{UNIVERSE}/badges")
}
fn badge_item(id: u64) -> String {
    format!("/legacy-badges/v1/badges/{id}")
}
fn badge_icon(id: u64) -> String {
    format!("/legacy-publish/v1/badges/{id}/icon")
}
fn product_collection() -> String {
    format!("/developer-products/v2/universes/{UNIVERSE}/developer-products")
}
fn product_item(id: u64) -> String {
    format!("{}/{id}", product_collection())
}

// ── fixture ──

struct Shop {
    dir: tempfile::TempDir,
}

impl Shop {
    /// A shop directory holding `rbxshop.toml` and an `rbxplace.toml` that
    /// resolves `--env dev` to `UNIVERSE`.
    fn new(config: &str) -> Self {
        let shop = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(shop.config_path(), config).unwrap();
        std::fs::write(
            shop.places_path(),
            format!("[{ENV}]\nuniverse_id = {UNIVERSE}\n"),
        )
        .unwrap();
        shop
    }

    fn config_path(&self) -> std::path::PathBuf {
        self.dir.path().join("rbxshop.toml")
    }

    fn places_path(&self) -> std::path::PathBuf {
        self.dir.path().join("rbxplace.toml")
    }

    fn lock_path(&self) -> std::path::PathBuf {
        self.dir.path().join(crate::lockfile::LOCKFILE_NAME)
    }

    fn write_lock(&self, contents: &str) {
        std::fs::write(self.lock_path(), contents).unwrap();
    }

    /// Write an icon file and return the hash `sync` will compare against
    /// the lockfile.
    fn icon(&self, name: &str, bytes: &[u8]) -> String {
        std::fs::write(self.dir.path().join(name), bytes).unwrap();
        rbx_core::image::hash_bytes(bytes)
    }

    fn lock(&self) -> Lockfile {
        Lockfile::load(&self.lock_path()).unwrap()
    }

    fn env_lock(&self) -> EnvLock {
        self.lock().env(ENV).cloned().unwrap()
    }

    async fn sync(&self, server: &MockServer, dry_run: bool) -> Result<()> {
        self.sync_only(server, dry_run, None).await
    }

    /// A sync that has been told the duplicate names are deliberate.
    async fn sync_allowing_duplicates(&self, server: &MockServer) -> Result<()> {
        self.sync_with(server, false, None, true).await
    }

    async fn sync_only(
        &self,
        server: &MockServer,
        dry_run: bool,
        only: Option<Vec<ResourceKind>>,
    ) -> Result<()> {
        self.sync_with(server, dry_run, only, false).await
    }

    async fn sync_with(
        &self,
        server: &MockServer,
        dry_run: bool,
        only: Option<Vec<ResourceKind>>,
        allow_duplicate_names: bool,
    ) -> Result<()> {
        let global = GlobalFlags {
            api_key: Some("test-key".into()),
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: Some(ENV.into()),
            place: None,
            places: self.places_path(),
            universe_id: None,
            place_id: Vec::new(),
        };
        let ctx = ShopCtx {
            config: self.config_path(),
            global: &global,
            base_url: Some(server.uri()),
        };
        run(&ctx, dry_run, only, 0, true, allow_duplicate_names).await
    }
}

// ── request inspection ──

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

fn is_mutating(r: &Request) -> bool {
    !matches!(r.method, wiremock::http::Method::GET)
}

fn body_of(r: &Request) -> String {
    String::from_utf8_lossy(&r.body).into_owned()
}

/// Read one text field out of a `multipart/form-data` body.
fn field(r: &Request, name: &str) -> Option<String> {
    let body = body_of(r);
    let needle = format!("name=\"{name}\"\r\n\r\n");
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    Some(rest[..rest.find("\r\n")?].to_string())
}

/// Whether the request carries an image part. This is the assertion that
/// distinguishes "icon re-uploaded" from "icon left alone".
fn carries_an_icon(r: &Request) -> bool {
    body_of(r).contains("filename=\"icon.png\"")
}

fn only<'a>(reqs: &'a [Request], method: &str, path: &str) -> &'a Request {
    let matched: Vec<_> = reqs
        .iter()
        .filter(|r| r.method.as_str() == method && r.url.path() == path)
        .collect();
    assert_eq!(
        matched.len(),
        1,
        "expected exactly one {method} {path}, saw {:?}",
        reqs.iter()
            .map(|r| format!("{} {}", r.method.as_str(), r.url.path()))
            .collect::<Vec<_>>()
    );
    matched[0]
}

fn count(reqs: &[Request], method: &str, path: &str) -> usize {
    reqs.iter()
        .filter(|r| r.method.as_str() == method && r.url.path() == path)
        .count()
}

// ── mocks ──

async fn mock(server: &MockServer, m: &str, p: String, body: serde_json::Value) {
    Mock::given(method(m))
        .and(path_matcher(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// One config declaring one of each resource kind, none of them locked.
const ONE_OF_EACH: &str = r#"
[owner]
type = "user"
id = 1

[passes.VIP]
price = 499

[badges.Welcome]

[products.Coins]
price = 99
"#;

/// The listings `preflight` reads before any create, answering "nothing here
/// yet".
///
/// Every test that creates a resource needs these mounted: the guard lists the
/// experience before the first write, and an unmocked listing would 404 into a
/// failure that says nothing about what the test was checking. Mounted as
/// *empty* because that is the honest remote state for a fixture whose
/// resources do not exist yet — a test wanting the collision case mounts its
/// own non-empty listing instead.
async fn mount_no_existing(server: &MockServer) {
    mount_existing(server, json!([]), json!([]), json!([])).await;
}

/// The same three listings, answering with whatever the experience already
/// holds. The collision tests are the reason this takes arguments.
///
/// These are the read paths, which are not the write paths: a pass is created
/// at `.../game-passes` and listed at `.../game-passes/creator`, and badges
/// are created on the cloud host but listed on the badges host. Getting that
/// wrong produces a 404 that looks like a broken guard.
async fn mount_existing(
    server: &MockServer,
    passes: serde_json::Value,
    badges: serde_json::Value,
    products: serde_json::Value,
) {
    mock(
        server,
        "GET",
        format!("{}/creator", pass_collection()),
        json!({ "gamePasses": passes, "nextPageToken": "" }),
    )
    .await;
    mock(
        server,
        "GET",
        format!("/v1/universes/{UNIVERSE}/badges"),
        json!({ "data": badges, "nextPageCursor": "" }),
    )
    .await;
    mock(
        server,
        "GET",
        format!("{}/creator", product_collection()),
        json!({ "developerProducts": products, "nextPageToken": "" }),
    )
    .await;
}

/// Empty catalogues plus the three create endpoints: what a first sync of
/// `ONE_OF_EACH` needs to succeed.
async fn mount_creates(server: &MockServer) {
    mount_no_existing(server).await;
    mount_creates_only(server).await;
}

/// The create endpoints alone, for tests that mount their own non-empty
/// catalogues.
async fn mount_creates_only(server: &MockServer) {
    mock(
        server,
        "POST",
        pass_collection(),
        json!({ "gamePassId": 111, "name": "VIP" }),
    )
    .await;
    mock(
        server,
        "POST",
        badge_collection(),
        json!({ "id": 222, "name": "Welcome", "iconImageId": 888 }),
    )
    .await;
    mock(
        server,
        "POST",
        product_collection(),
        json!({ "productId": 333, "name": "Coins" }),
    )
    .await;
}

// ── the dry-run invariant ──

/// The invariant the whole command hangs on. No mocks are mounted, so any
/// request at all would 404 and surface as an error too — but the count is
/// asserted directly, because "it happened to fail" is not the property.
#[tokio::test]
async fn a_dry_run_sends_no_request_and_writes_no_lockfile() {
    let shop = Shop::new(ONE_OF_EACH);
    let server = MockServer::start().await;

    shop.sync(&server, true).await.unwrap();

    assert!(
        requests(&server).await.is_empty(),
        "a dry run must not talk to Roblox at all"
    );
    assert!(
        !shop.lock_path().exists(),
        "a dry run must not write a lockfile"
    );
}

/// The control for the test above: the same fixture, applied for real,
/// does create all three. Without this, the dry-run assertion would also
/// pass on a plan that had nothing to do.
#[tokio::test]
async fn the_same_plan_applied_for_real_creates_all_three() {
    let shop = Shop::new(ONE_OF_EACH);
    let server = MockServer::start().await;
    mount_creates(&server).await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&reqs, "POST", &pass_collection()), 1);
    assert_eq!(count(&reqs, "POST", &badge_collection()), 1);
    assert_eq!(count(&reqs, "POST", &product_collection()), 1);

    let env = shop.env_lock();
    assert_eq!(env.universe_id, UNIVERSE);
    assert_eq!(env.passes["VIP"].id, 111);
    assert_eq!(env.badges["Welcome"].id, 222);
    assert_eq!(env.products["Coins"].id, 333);
}

/// `--only` narrows what is applied, not just what is printed.
#[tokio::test]
async fn only_passes_leaves_badges_and_products_untouched() {
    let shop = Shop::new(ONE_OF_EACH);
    let server = MockServer::start().await;
    mount_creates(&server).await;

    shop.sync_only(&server, false, Some(vec![ResourceKind::Pass]))
        .await
        .unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&reqs, "POST", &pass_collection()), 1);
    assert_eq!(count(&reqs, "POST", &badge_collection()), 0);
    assert_eq!(count(&reqs, "POST", &product_collection()), 0);

    let env = shop.env_lock();
    assert!(env.passes.contains_key("VIP"));
    assert!(env.badges.is_empty());
    assert!(env.products.is_empty());
}

/// Nothing to do means nothing sent — and, as it stands, no lockfile
/// rewrite either. Pinned so that a future change to the up-to-date path
/// is a deliberate one.
#[tokio::test]
async fn an_up_to_date_shop_sends_nothing() {
    let shop = Shop::new(
        r#"
[passes.VIP]
price = 499
"#,
    );
    let lock_toml = format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.passes.VIP]\nid = 4242\nname = \"VIP\"\nprice = 499\n\
         for_sale = true\nregional_pricing = false\n"
    );
    shop.write_lock(&lock_toml);
    let server = MockServer::start().await;

    shop.sync(&server, false).await.unwrap();

    assert!(requests(&server).await.is_empty());
    assert_eq!(
        std::fs::read_to_string(shop.lock_path()).unwrap(),
        lock_toml,
        "an up-to-date sync leaves the lockfile byte-identical"
    );
}

// ── passes ──

#[tokio::test]
async fn creating_a_pass_sends_its_fields_and_records_the_returned_id() {
    let shop = Shop::new(
        r#"
[passes.VIP]
name = "VIP Pass"
price = 499
description = "all the perks"
icon = "vip.png"
regional_pricing = true
"#,
    );
    let hash = shop.icon("vip.png", PNG);
    let server = MockServer::start().await;
    mount_no_existing(&server).await;
    mock(
        &server,
        "POST",
        pass_collection(),
        json!({ "gamePassId": 111, "iconAssetId": 777 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    let create = only(&reqs, "POST", &pass_collection());
    assert_eq!(field(create, "name").as_deref(), Some("VIP Pass"));
    assert_eq!(field(create, "price").as_deref(), Some("499"));
    assert_eq!(
        field(create, "description").as_deref(),
        Some("all the perks")
    );
    assert_eq!(field(create, "isForSale").as_deref(), Some("true"));
    assert_eq!(
        field(create, "isRegionalPricingEnabled").as_deref(),
        Some("true")
    );
    assert!(carries_an_icon(create));

    let locked = &shop.env_lock().passes["VIP"];
    assert_eq!(locked.id, 111);
    assert_eq!(locked.name, "VIP Pass");
    assert_eq!(locked.price, Some(499));
    assert_eq!(locked.icon_asset_id, Some(777));
    // The hash of the file on disk, not of the re-encoded upload — the
    // next diff hashes the file, so anything else re-uploads forever.
    assert_eq!(locked.icon_hash.as_deref(), Some(hash.as_str()));
}

#[tokio::test]
async fn updating_a_pass_patches_the_id_from_the_lockfile() {
    let shop = Shop::new(
        r#"
[passes.VIP]
price = 999
"#,
    );
    shop.write_lock(&format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.passes.VIP]\nid = 4242\nname = \"VIP\"\nprice = 499\n\
         for_sale = true\nregional_pricing = false\n"
    ));
    let server = MockServer::start().await;
    mock(
        &server,
        "PATCH",
        pass_item(4242),
        json!({ "gamePassId": 4242 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    // Never a create: an update that POSTs makes a second paid pass.
    assert_eq!(count(&reqs, "POST", &pass_collection()), 0);
    let patch = only(&reqs, "PATCH", &pass_item(4242));
    assert_eq!(field(patch, "price").as_deref(), Some("999"));

    let locked = &shop.env_lock().passes["VIP"];
    assert_eq!(locked.id, 4242, "the id must survive an update");
    assert_eq!(locked.price, Some(999));
}

/// A false negative on the icon hash re-uploads the same image on every
/// sync; the request either carries the file part or it does not.
#[tokio::test]
async fn an_unchanged_icon_is_not_re_uploaded() {
    let shop = Shop::new(
        r#"
[passes.VIP]
price = 999
icon = "vip.png"
"#,
    );
    let hash = shop.icon("vip.png", PNG);
    shop.write_lock(&format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.passes.VIP]\nid = 4242\nname = \"VIP\"\nprice = 499\n\
         icon_hash = \"{hash}\"\nfor_sale = true\nregional_pricing = false\n"
    ));
    let server = MockServer::start().await;
    mock(
        &server,
        "PATCH",
        pass_item(4242),
        json!({ "gamePassId": 4242 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    let patch = only(&reqs, "PATCH", &pass_item(4242));
    assert_eq!(field(patch, "price").as_deref(), Some("999"));
    assert!(
        !carries_an_icon(patch),
        "the icon hash matched, so no image should be sent"
    );
    assert_eq!(
        shop.env_lock().passes["VIP"].icon_hash.as_deref(),
        Some(hash.as_str())
    );
}

/// The mirror image: a false positive leaves a stale icon live forever.
#[tokio::test]
async fn a_changed_icon_is_uploaded_and_rehashed() {
    let shop = Shop::new(
        r#"
[passes.VIP]
price = 499
icon = "vip.png"
"#,
    );
    let hash = shop.icon("vip.png", PNG);
    shop.write_lock(&format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.passes.VIP]\nid = 4242\nname = \"VIP\"\nprice = 499\n\
         icon_hash = \"stale\"\nfor_sale = true\nregional_pricing = false\n"
    ));
    let server = MockServer::start().await;
    mock(
        &server,
        "PATCH",
        pass_item(4242),
        json!({ "gamePassId": 4242, "iconAssetId": 777 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    assert!(carries_an_icon(only(&reqs, "PATCH", &pass_item(4242))));

    let locked = &shop.env_lock().passes["VIP"];
    assert_eq!(locked.icon_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(locked.icon_asset_id, Some(777));
}

/// A refused create must not leave a half-written entry behind: the id
/// would be wrong or absent, and the next sync would create a second one.
#[tokio::test]
async fn a_refused_create_fails_without_locking_the_resource() {
    let shop = Shop::new(
        r#"
[passes.VIP]
price = 499
"#,
    );
    let server = MockServer::start().await;
    // Mounted so the run reaches the create and fails there. Without it the
    // preflight listing would 404 first, and the test would pass on the wrong
    // error.
    mount_no_existing(&server).await;
    Mock::given(method("POST"))
        .and(path_matcher(pass_collection()))
        .respond_with(ResponseTemplate::new(403).set_body_string("no scope"))
        .mount(&server)
        .await;

    assert!(shop.sync(&server, false).await.is_err());
    assert!(
        !shop.lock_path().exists(),
        "a failed create must not be recorded"
    );
}

// ── badges ──

/// The payment source is a 1/2 enum for user/group. Sending the wrong one
/// bills the wrong account.
#[tokio::test]
async fn creating_a_badge_derives_the_payment_source_from_the_owner() {
    let shop = Shop::new(
        r#"
[owner]
type = "group"
id = 7

[badges.Welcome]
description = "first login"
icon = "welcome.png"
"#,
    );
    let hash = shop.icon("welcome.png", PNG);
    let server = MockServer::start().await;
    mount_no_existing(&server).await;
    mock(
        &server,
        "POST",
        badge_collection(),
        json!({ "id": 222, "iconImageId": 888 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    let create = only(&reqs, "POST", &badge_collection());
    assert_eq!(field(create, "paymentSourceType").as_deref(), Some("2"));
    assert_eq!(field(create, "expectedCost").as_deref(), Some("0"));
    assert_eq!(field(create, "name").as_deref(), Some("Welcome"));
    assert!(carries_an_icon(create));

    let locked = &shop.env_lock().badges["Welcome"];
    assert_eq!(locked.id, 222);
    assert_eq!(locked.icon_asset_id, Some(888));
    assert_eq!(locked.icon_hash.as_deref(), Some(hash.as_str()));
}

/// Badge metadata and badge icons are two different endpoints. A metadata
/// change must not drag the icon along.
#[tokio::test]
async fn a_badge_metadata_change_does_not_touch_the_icon_endpoint() {
    let shop = Shop::new(
        r#"
[badges.Welcome]
description = "first login"
enabled = false
icon = "welcome.png"
"#,
    );
    let hash = shop.icon("welcome.png", PNG);
    shop.write_lock(&format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.badges.Welcome]\nid = 555\nname = \"Welcome\"\nenabled = true\n\
         icon_hash = \"{hash}\"\n"
    ));
    let server = MockServer::start().await;
    mock(&server, "PATCH", badge_item(555), json!({ "id": 555 })).await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    let patch = only(&reqs, "PATCH", &badge_item(555));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body_of(patch)).unwrap(),
        json!({ "name": "Welcome", "description": "first login", "enabled": false })
    );
    assert_eq!(count(&reqs, "POST", &badge_icon(555)), 0);
}

/// And the reverse: an icon-only change must not send a metadata patch,
/// which would rewrite name/description from the config for no reason.
#[tokio::test]
async fn a_badge_icon_change_does_not_send_a_metadata_patch() {
    let shop = Shop::new(
        r#"
[badges.Welcome]
icon = "welcome.png"
"#,
    );
    let hash = shop.icon("welcome.png", PNG);
    shop.write_lock(&format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.badges.Welcome]\nid = 555\nname = \"Welcome\"\nenabled = true\n\
         icon_hash = \"stale\"\n"
    ));
    let server = MockServer::start().await;
    mock(&server, "POST", badge_icon(555), json!({ "targetId": 999 })).await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&reqs, "PATCH", &badge_item(555)), 0);
    assert!(carries_an_icon(only(&reqs, "POST", &badge_icon(555))));

    let locked = &shop.env_lock().badges["Welcome"];
    assert_eq!(locked.icon_asset_id, Some(999));
    assert_eq!(locked.icon_hash.as_deref(), Some(hash.as_str()));
}

// ── products ──

#[tokio::test]
async fn creating_a_product_sends_its_price_and_records_the_returned_id() {
    let shop = Shop::new(
        r#"
[products.Coins]
name = "100 Coins"
price = 99
description = "a pile"
"#,
    );
    let server = MockServer::start().await;
    mount_no_existing(&server).await;
    mock(
        &server,
        "POST",
        product_collection(),
        json!({ "productId": 333, "iconImageAssetId": 666 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    let create = only(&reqs, "POST", &product_collection());
    assert_eq!(field(create, "name").as_deref(), Some("100 Coins"));
    assert_eq!(field(create, "price").as_deref(), Some("99"));
    assert_eq!(field(create, "description").as_deref(), Some("a pile"));
    assert!(!carries_an_icon(create));

    let locked = &shop.env_lock().products["Coins"];
    assert_eq!(locked.id, 333);
    assert_eq!(locked.price, 99);
    assert_eq!(locked.icon_asset_id, Some(666));
    assert!(locked.icon_hash.is_none());
}

#[tokio::test]
async fn updating_a_product_patches_the_id_from_the_lockfile() {
    let shop = Shop::new(
        r#"
[products.Coins]
price = 199
store_page = true
"#,
    );
    shop.write_lock(&format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.products.Coins]\nid = 3131\nname = \"Coins\"\nprice = 99\n\
         for_sale = true\nregional_pricing = false\nstore_page = false\n"
    ));
    let server = MockServer::start().await;
    mock(
        &server,
        "PATCH",
        product_item(3131),
        json!({ "productId": 3131 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&reqs, "POST", &product_collection()), 0);
    let patch = only(&reqs, "PATCH", &product_item(3131));
    assert_eq!(field(patch, "price").as_deref(), Some("199"));
    assert_eq!(field(patch, "storePageEnabled").as_deref(), Some("true"));

    let locked = &shop.env_lock().products["Coins"];
    assert_eq!(locked.id, 3131);
    assert_eq!(locked.price, 199);
    assert!(locked.store_page);
}

/// Roblox validates `isForSale` against the *current* remote state, so
/// taking a product off sale needs the store page removed first. Two
/// patches, in that order — one patch and the change is silently refused.
#[tokio::test]
async fn taking_a_product_off_sale_clears_the_store_page_first() {
    let shop = Shop::new(
        r#"
[products.Coins]
price = 99
for_sale = false
"#,
    );
    shop.write_lock(&format!(
        "version = 2\n\n[envs.{ENV}]\nuniverse_id = {UNIVERSE}\n\n\
         [envs.{ENV}.products.Coins]\nid = 3131\nname = \"Coins\"\nprice = 99\n\
         for_sale = true\nregional_pricing = false\nstore_page = true\n"
    ));
    let server = MockServer::start().await;
    mock(
        &server,
        "PATCH",
        product_item(3131),
        json!({ "productId": 3131 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    let patches: Vec<_> = reqs
        .iter()
        .filter(|r| r.method.as_str() == "PATCH" && r.url.path() == product_item(3131))
        .collect();
    assert_eq!(patches.len(), 2, "off-sale takes two patches");
    assert_eq!(field(patches[0], "isForSale").as_deref(), Some("true"));
    assert_eq!(
        field(patches[0], "storePageEnabled").as_deref(),
        Some("false")
    );
    assert_eq!(field(patches[1], "isForSale").as_deref(), Some("false"));

    assert!(!shop.env_lock().products["Coins"].for_sale);
}

/// Every request `sync` makes is authenticated; an unauthenticated one
/// would 401 in production and read here as a mock-shape problem.
#[tokio::test]
async fn every_mutating_request_carries_the_api_key() {
    let shop = Shop::new(ONE_OF_EACH);
    let server = MockServer::start().await;
    mount_creates(&server).await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    assert!(!reqs.is_empty());
    for r in reqs.iter().filter(|r| is_mutating(r)) {
        assert_eq!(
            r.headers.get("x-api-key").map(|v| v.to_str().unwrap()),
            Some("test-key"),
            "{} {} went out unauthenticated",
            r.method.as_str(),
            r.url.path()
        );
    }
}

// ── the duplicate guard ──

/// The scenario the guard exists for. The lockfile was never committed, so the
/// plan says "create"; Roblox says the pass is already there. Creating it
/// again would mint a second paid product that cannot be deleted.
#[tokio::test]
async fn a_pass_that_already_exists_remotely_stops_the_sync() {
    let shop = Shop::new(ONE_OF_EACH);
    let server = MockServer::start().await;
    mount_existing(
        &server,
        json!([{ "gamePassId": 111, "name": "VIP" }]),
        json!([]),
        json!([]),
    )
    .await;
    mount_creates_only(&server).await;

    let err = shop.sync(&server, false).await.unwrap_err().to_string();
    assert!(err.contains("VIP"), "{err}");
    assert!(err.contains("111"), "{err}");

    let reqs = requests(&server).await;
    assert_eq!(
        reqs.iter().filter(|r| is_mutating(r)).count(),
        0,
        "the guard must stop the run before anything is written"
    );
    assert!(
        !shop.lock_path().exists(),
        "a refused sync must not write a lockfile"
    );
}

/// The guard stops the whole run, not just the colliding resource. A partial
/// sync would leave the lockfile describing an env that was half applied,
/// which is harder to reason about than one that was not touched.
#[tokio::test]
async fn one_collision_stops_the_other_kinds_too() {
    let shop = Shop::new(ONE_OF_EACH);
    let server = MockServer::start().await;
    mount_existing(
        &server,
        json!([]),
        json!([{ "id": 222, "name": "Welcome" }]),
        json!([]),
    )
    .await;
    mount_creates_only(&server).await;

    assert!(shop.sync(&server, false).await.is_err());

    let reqs = requests(&server).await;
    assert_eq!(count(&reqs, "POST", &pass_collection()), 0);
    assert_eq!(count(&reqs, "POST", &product_collection()), 0);
}

/// The escape hatch, for the developer who really does want a second resource
/// by the same name.
#[tokio::test]
async fn allow_duplicate_names_creates_anyway() {
    let shop = Shop::new(ONE_OF_EACH);
    let server = MockServer::start().await;
    mount_existing(
        &server,
        json!([{ "gamePassId": 111, "name": "VIP" }]),
        json!([]),
        json!([]),
    )
    .await;
    mount_creates_only(&server).await;

    shop.sync_allowing_duplicates(&server).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(count(&reqs, "POST", &pass_collection()), 1);
    assert_eq!(shop.env_lock().passes["VIP"].id, 111);
}

/// A sync with nothing to create asks Roblox nothing about what exists. The
/// listing needs read scopes the write-only key of an update-only pipeline
/// does not carry, so making it unconditional would break those keys for a
/// check that could not have found anything.
#[tokio::test]
async fn a_sync_with_no_creates_does_not_list_anything() {
    let shop = Shop::new(
        r#"
[passes.VIP]
price = 999
"#,
    );
    shop.write_lock(&format!(
        r#"
version = 2
[envs.{ENV}]
universe_id = {UNIVERSE}
[envs.{ENV}.passes.VIP]
id = 111
name = "VIP"
price = 499
"#
    ));
    let server = MockServer::start().await;
    mock(
        &server,
        "PATCH",
        pass_item(111),
        json!({ "gamePassId": 111 }),
    )
    .await;

    shop.sync(&server, false).await.unwrap();

    let reqs = requests(&server).await;
    assert_eq!(
        reqs.iter().filter(|r| !is_mutating(r)).count(),
        0,
        "an update-only sync must not read the catalogues"
    );
    assert_eq!(count(&reqs, "PATCH", &pass_item(111)), 1);
}
