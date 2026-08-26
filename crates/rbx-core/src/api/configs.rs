//! The Open Cloud Configs API, which is one transport serving several products.
//!
//! `/creator-configs-public-api/v1/configs/universes/{id}/repositories/{repo}`
//! takes the repository as a **path parameter**, and every verb under it is
//! identical whichever repository is named: stage a draft, read it, publish it,
//! list revisions, restore one.
//!
//! This lived in `rbx-config` with the repository welded in as a constant, which
//! made seven of the eight repositories unreachable and would have made a second
//! consumer copy the client. `rbx_core::api::send_with_csrf` is here for exactly
//! the same reason: four crates carried their own copy until they started
//! disagreeing about whether `204` was a success.
//!
//! # What the repositories hold
//!
//! Only two are documented. `InExperienceConfig` is the live config
//! `ConfigService` reads in-experience. `DataStoresConfig` holds the
//! right-to-be-forgotten deletion templates, documented on a different page
//! than the configs guide, whose repository table still lists one row.
//!
//! The other six are in the enum and documented nowhere. That is deliberate on
//! Roblox's side, and the enum's own description says so: "Only values exposed
//! by the public API are included; internal repository types are not exposed to
//! allow development and testing before enabling." They are forward
//! declarations, so this module carries the transport and takes no view on what
//! any repository's entries mean.

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{bail, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::{execute_json, is_api_status, ApiBase};

const CONFIGS_PATH: &str = "/creator-configs-public-api/v1/configs/universes";

/// A configs repository, as the `{repository}` path segment spells it.
///
/// The variants are the `Repository` enum of the vendored OpenAPI document,
/// verbatim. Modelled as a closed enum rather than a string because a typo is
/// otherwise a 400 from Roblox naming nothing a reader can act on, and because
/// the list is what `--repository` offers when a name does not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repository {
    /// The live config `ConfigService` reads in-experience. The default, and
    /// the only repository `rbx config` addressed before this existed.
    #[default]
    InExperienceConfig,
    /// Data store right-to-be-forgotten deletion templates.
    DataStoresConfig,
    RecommendationServicesConfig,
    ExtendedServicesConfig,
    LeaderboardsConfig,
    ExperienceUserConfig,
    JourneysConfig,
    AntiCheatConfig,
}

impl Repository {
    /// Every repository the public API exposes, in the spec's order.
    pub const ALL: &'static [Repository] = &[
        Repository::InExperienceConfig,
        Repository::RecommendationServicesConfig,
        Repository::DataStoresConfig,
        Repository::ExtendedServicesConfig,
        Repository::LeaderboardsConfig,
        Repository::ExperienceUserConfig,
        Repository::JourneysConfig,
        Repository::AntiCheatConfig,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::InExperienceConfig => "InExperienceConfig",
            Self::RecommendationServicesConfig => "RecommendationServicesConfig",
            Self::DataStoresConfig => "DataStoresConfig",
            Self::ExtendedServicesConfig => "ExtendedServicesConfig",
            Self::LeaderboardsConfig => "LeaderboardsConfig",
            Self::ExperienceUserConfig => "ExperienceUserConfig",
            Self::JourneysConfig => "JourneysConfig",
            Self::AntiCheatConfig => "AntiCheatConfig",
        }
    }

    /// Whether this repository has a documented entry schema.
    ///
    /// Two of the eight do. The rest are reachable and usable by anyone who
    /// knows their keys, which is why they are not hidden, but a command that
    /// offers to help with their contents would be offering to guess.
    pub fn is_documented(self) -> bool {
        matches!(self, Self::InExperienceConfig | Self::DataStoresConfig)
    }
}

impl fmt::Display for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Repository {
    type Err = anyhow::Error;

    /// Case-insensitive, because the path segment is PascalCase and nobody
    /// types that from memory. The canonical spelling is what goes on the wire.
    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|repo| repo.as_str().eq_ignore_ascii_case(value))
            .ok_or_else(|| {
                let available: Vec<&str> = Self::ALL.iter().map(|r| r.as_str()).collect();
                anyhow::anyhow!(
                    "'{}' is not a configs repository.\nAvailable: {}",
                    value,
                    available.join(", ")
                )
            })
    }
}

/// Roblox's stated ceiling on how many entries one repository holds.
pub const MAX_KEYS_PER_REPOSITORY: usize = 100;

/// Roblox's stated ceiling on the length of one entry key, in characters.
pub const MAX_KEY_LENGTH: usize = 256;

/// Refuse an entry set Roblox would refuse.
///
/// Both bounds are from the configs guide's limits table. Checked here for the
/// reason every other bound in this suite is: a publish is the last step of a
/// deploy, and a 400 that names neither the key nor the limit arrives at the
/// worst possible moment. A local bound looser than the server's is worse than
/// none, so these track the documented numbers and say which they are.
///
/// The key length is counted in `char`s rather than bytes. The guide says
/// "256 characters", and a byte count would refuse a key of 200 accented
/// characters that Roblox accepts.
pub fn validate_entries(entries: &BTreeMap<String, Json>) -> Result<()> {
    if entries.len() > MAX_KEYS_PER_REPOSITORY {
        bail!(
            "{} entries, over Roblox's limit of {MAX_KEYS_PER_REPOSITORY} keys per repository. \
             Remove some, or split them across repositories.",
            entries.len()
        );
    }
    for key in entries.keys() {
        let length = key.chars().count();
        if length > MAX_KEY_LENGTH {
            bail!(
                "the key starting \"{}\" is {length} characters, over Roblox's \
                 {MAX_KEY_LENGTH}-character limit.",
                key.chars().take(40).collect::<String>()
            );
        }
    }
    Ok(())
}

/// A client aimed at one universe's one repository.
///
/// The repository is a field rather than a parameter on every method: it is
/// fixed for the life of a command, and threading it through nine call sites
/// invites the tenth to pass the wrong one.
pub struct ConfigsClient {
    client: Client,
    api_key: String,
    base: ApiBase,
    repository: Repository,
}

/// Hand-written rather than derived, because the derive would print `api_key`,
/// and a client is exactly the sort of value that ends up in a `{:?}` inside an
/// error context or a debug log.
impl fmt::Debug for ConfigsClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigsClient")
            .field("base", &self.base)
            .field("repository", &self.repository)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl ConfigsClient {
    pub fn new(api_key: String, repository: Repository) -> Self {
        Self {
            client: super::build_client(),
            api_key,
            base: ApiBase::default(),
            repository,
        }
    }

    /// Point the client at another host.
    ///
    /// `pub` rather than test-only because the consumers live in other crates
    /// and a `#[cfg(test)]` seam does not cross a crate boundary. The host was
    /// a baked-in constant once, which is why this client had no HTTP tests at
    /// all.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base = ApiBase::new(url);
        self
    }

    pub fn repository(&self) -> Repository {
        self.repository
    }

    fn repo_url(&self, universe_id: u64) -> String {
        format!(
            "{}/{}/repositories/{}",
            self.base.join(CONFIGS_PATH),
            universe_id,
            self.repository
        )
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, url: String) -> Result<T> {
        let api_key = self.api_key.clone();
        execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await
    }

    /// The live published config. An empty snapshot on 404.
    ///
    /// A universe that has never published answers 404, and that is the
    /// starting state rather than a failure: `sync` builds its first draft from
    /// it. A 403 is **not** folded in, deliberately: treating a refusal as
    /// "nothing published" would make `sync` diff against an empty remote and
    /// offer to overwrite a live config with everything in the file.
    pub async fn get_config(&self, universe_id: u64) -> Result<ConfigSnapshot> {
        match self.get(self.repo_url(universe_id)).await {
            Ok(snapshot) => Ok(snapshot),
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => {
                Ok(ConfigSnapshot::default())
            }
            Err(error) => Err(error),
        }
    }

    /// The staged draft, if there is one.
    ///
    /// Read before a write so the hash can be handed back as
    /// the concurrency hash. 404 is "no draft", which is the ordinary state.
    pub async fn get_draft(&self, universe_id: u64) -> Result<RepositoryDraft> {
        let url = format!("{}/draft", self.repo_url(universe_id));
        match self.get(url).await {
            Ok(draft) => Ok(draft),
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => {
                Ok(RepositoryDraft::default())
            }
            Err(error) => Err(error),
        }
    }

    /// Replace the whole draft. Any key omitted is treated as removed.
    ///
    /// `previous_draft_hash` is Roblox's optimistic concurrency check: when it
    /// does not match the server's current draft the request fails, which is
    /// what stops a `sync` from silently discarding an edit somebody made in
    /// the Creator Hub between this command's read and its write.
    pub async fn overwrite_draft(
        &self,
        universe_id: u64,
        entries: &BTreeMap<String, Json>,
        previous_draft_hash: Option<&str>,
    ) -> Result<DraftResult> {
        let url = format!("{}/draft:overwrite", self.repo_url(universe_id));
        let api_key = self.api_key.clone();
        let body = serde_json::to_string(&OverwriteBody {
            entries,
            previous_draft_hash,
        })?;

        execute_json(|| async {
            Ok(self
                .client
                .put(&url)
                .header("x-api-key", &api_key)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await?)
        })
        .await
    }

    pub async fn publish(
        &self,
        universe_id: u64,
        message: &str,
        strategy: &str,
        draft_hash: Option<&str>,
    ) -> Result<PublishResult> {
        let url = format!("{}/publish", self.repo_url(universe_id));
        let api_key = self.api_key.clone();
        let body = serde_json::to_string(&PublishBody {
            message,
            deployment_strategy: strategy,
            draft_hash,
        })?;

        execute_json(|| async {
            Ok(self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await?)
        })
        .await
    }

    /// Stage the entries and publish them, guarding against a concurrent draft.
    ///
    /// Three requests rather than two, and the extra one is the point: the
    /// draft is read first so its hash travels back as `draftHash`.
    /// Without it this call overwrote whatever was staged, in silence, which is
    /// the one outcome a human cannot recover from because the draft is gone
    /// before they learn it existed.
    ///
    /// Returns the publish result alongside the entries the discarded draft
    /// held, so a caller can say what it replaced.
    pub async fn overwrite_and_publish(
        &self,
        universe_id: u64,
        entries: &BTreeMap<String, Json>,
        message: &str,
        strategy: &str,
    ) -> Result<(PublishResult, ReplacedDraft)> {
        validate_entries(entries)?;
        let existing = self.get_draft(universe_id).await?;
        let replaced = ReplacedDraft {
            keys: existing.entries.keys().cloned().collect(),
        };
        let draft = self
            .overwrite_draft(universe_id, entries, existing.draft_hash.as_deref())
            .await?;
        let published = self
            .publish(universe_id, message, strategy, draft.draft_hash.as_deref())
            .await?;
        Ok((published, replaced))
    }

    pub async fn list_revisions(&self, universe_id: u64, max: usize) -> Result<Vec<RevisionEntry>> {
        let url = format!(
            "{}/revisions?MaxPageSize={}&SortOrder=SORT_ORDER_DESCENDING",
            self.repo_url(universe_id),
            max
        );
        let response: ListRevisionsResponse = self.get(url).await?;
        Ok(response.revisions)
    }

    /// Stage a revert to `revision_id`. Does **not** publish: the returned
    /// hash is what a following `publish` takes.
    pub async fn restore_revision(&self, universe_id: u64, revision_id: &str) -> Result<String> {
        let url = format!(
            "{}/revisions/{}:restore",
            self.repo_url(universe_id),
            revision_id
        );
        let api_key = self.api_key.clone();

        let response: RestoreResponse = execute_json(|| async {
            Ok(self
                .client
                .post(&url)
                .header("x-api-key", &api_key)
                .send()
                .await?)
        })
        .await?;

        Ok(response.draft_hash)
    }
}

/// What a draft held before a write replaced it.
///
/// Keys only. The values are not carried because nothing prints them, and a
/// discarded draft's contents in a log is a config value in a place nobody
/// audited.
#[derive(Debug, Default)]
pub struct ReplacedDraft {
    pub keys: Vec<String>,
}

impl ReplacedDraft {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Response from `GET /repositories/{repository}`.
#[derive(Debug, Deserialize, Default)]
pub struct ConfigSnapshot {
    #[serde(default)]
    pub metadata: ConfigMetadata,
    #[serde(default)]
    pub entries: BTreeMap<String, Json>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ConfigMetadata {
    #[serde(rename = "configVersion", default)]
    pub config_version: u64,
}

/// Response from `GET /draft`.
#[derive(Debug, Deserialize, Default)]
pub struct RepositoryDraft {
    #[serde(rename = "draftHash", default)]
    pub draft_hash: Option<String>,
    /// Keyed by entry name. The value shape is the draft's own
    /// (`{value, description}`), which nothing here reads, so it stays `Json`
    /// rather than a speculative struct that would silently stop matching.
    #[serde(default)]
    pub entries: BTreeMap<String, Json>,
}

/// Response from `PATCH /draft` and `PUT /draft:overwrite`.
#[derive(Debug, Deserialize)]
pub struct DraftResult {
    #[serde(rename = "draftHash")]
    pub draft_hash: Option<String>,
}

/// Response from `POST /publish`.
#[derive(Debug, Deserialize)]
pub struct PublishResult {
    #[serde(rename = "configVersion")]
    pub config_version: u64,
}

/// Body for `PUT /draft:overwrite` and `PATCH /draft`.
///
/// **The concurrency field is `draftHash`, not `previousDraftHash`.** Roblox's
/// configs guide says the latter in prose; the vendored spec's
/// `UpdateDraftRequest` defines `draftHash` ("The previous draft hash for
/// concurrency control") and does not mention `previousDraftHash` anywhere.
///
/// The spec wins, and here the choice is not a coin flip: `UpdateDraftRequest`
/// carries `additionalProperties: false`, so the guide's spelling would be
/// **rejected** rather than ignored, and every `sync` against a repository with
/// a staged draft would fail on an opaque 4xx. Sending the spec's name is also
/// the safe side of the other branch: if the service happened to want the
/// guide's name, the guard is merely inert, which is where this code was before
/// it sent anything at all.
#[derive(Debug, Serialize)]
struct OverwriteBody<'a> {
    entries: &'a BTreeMap<String, Json>,
    #[serde(rename = "draftHash", skip_serializing_if = "Option::is_none")]
    previous_draft_hash: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct PublishBody<'a> {
    message: &'a str,
    #[serde(rename = "deploymentStrategy")]
    deployment_strategy: &'a str,
    #[serde(rename = "draftHash", skip_serializing_if = "Option::is_none")]
    draft_hash: Option<&'a str>,
}

/// One entry of `GET /revisions`.
#[derive(Debug, Clone, Deserialize)]
pub struct RevisionEntry {
    #[serde(rename = "revisionId")]
    pub revision_id: String,
    pub version: u64,
    pub time: String,
    #[serde(default)]
    pub message: Option<String>,
    /// Per-key change payloads. Only the key set is surfaced, so this holds raw
    /// `Json` rather than a typed `{ before, after }` nobody reads: a
    /// speculative schema silently stops matching the API, and
    /// `#[allow(dead_code)]` is what makes the drift invisible.
    #[serde(default)]
    pub changes: std::collections::HashMap<String, Json>,
}

#[derive(Debug, Deserialize)]
struct ListRevisionsResponse {
    #[serde(default)]
    revisions: Vec<RevisionEntry>,
}

#[derive(Debug, Deserialize)]
struct RestoreResponse {
    #[serde(rename = "draftHash")]
    draft_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn every_repository_round_trips_through_its_wire_spelling() {
        for repo in Repository::ALL {
            assert_eq!(Repository::from_str(repo.as_str()).unwrap(), *repo);
        }
        assert_eq!(Repository::ALL.len(), 8, "the spec's enum has eight values");
    }

    /// The path segment is PascalCase and nobody types that from memory, but
    /// what goes on the wire is the canonical spelling either way.
    #[test]
    fn a_name_is_matched_without_regard_to_case_and_normalised() {
        assert_eq!(
            Repository::from_str("datastoresconfig").unwrap(),
            Repository::DataStoresConfig
        );
        assert_eq!(
            Repository::from_str("DATASTORESCONFIG").unwrap().as_str(),
            "DataStoresConfig"
        );
    }

    #[test]
    fn an_unknown_repository_lists_the_ones_that_exist() {
        let err = Repository::from_str("DataStore").unwrap_err().to_string();
        assert!(err.contains("is not a configs repository"), "{err}");
        assert!(err.contains("InExperienceConfig"), "{err}");
        assert!(err.contains("DataStoresConfig"), "{err}");
    }

    /// The one that has to stay true for every existing invocation: no
    /// `--repository` means the repository `rbx config` has always used.
    #[test]
    fn the_default_is_the_repository_this_suite_started_with() {
        assert_eq!(Repository::default(), Repository::InExperienceConfig);
    }

    #[test]
    fn only_the_two_documented_repositories_say_so() {
        let documented: Vec<&str> = Repository::ALL
            .iter()
            .filter(|r| r.is_documented())
            .map(|r| r.as_str())
            .collect();
        assert_eq!(documented, ["InExperienceConfig", "DataStoresConfig"]);
    }

    fn entries(count: usize) -> BTreeMap<String, Json> {
        (0..count)
            .map(|i| (format!("key{i}"), Json::from(i)))
            .collect()
    }

    #[test]
    fn the_key_count_ceiling_is_roblox_s_and_the_error_names_it() {
        assert!(validate_entries(&entries(MAX_KEYS_PER_REPOSITORY)).is_ok());

        let err = validate_entries(&entries(MAX_KEYS_PER_REPOSITORY + 1))
            .unwrap_err()
            .to_string();
        assert!(err.contains("101 entries"), "{err}");
        assert!(err.contains("100 keys per repository"), "{err}");
    }

    /// Characters, not bytes. The guide says "256 characters", and a byte count
    /// would refuse a key of accented characters Roblox accepts.
    #[test]
    fn the_key_length_ceiling_counts_characters_rather_than_bytes() {
        let mut long: BTreeMap<String, Json> = BTreeMap::new();
        long.insert("é".repeat(MAX_KEY_LENGTH), Json::from(1));
        assert!(validate_entries(&long).is_ok(), "512 bytes, 256 characters");

        let mut over: BTreeMap<String, Json> = BTreeMap::new();
        over.insert("a".repeat(MAX_KEY_LENGTH + 1), Json::from(1));
        let err = validate_entries(&over).unwrap_err().to_string();
        assert!(err.contains("257 characters"), "{err}");
        assert!(err.contains("256-character limit"), "{err}");
    }

    /// The error has to be readable next to a key that is 257 characters long,
    /// so it quotes the start rather than the whole thing.
    #[test]
    fn an_oversized_key_is_named_by_its_beginning() {
        let mut over: BTreeMap<String, Json> = BTreeMap::new();
        over.insert(
            format!("features.{}", "x".repeat(MAX_KEY_LENGTH)),
            Json::from(1),
        );
        let err = validate_entries(&over).unwrap_err().to_string();
        assert!(err.contains("features.xxx"), "{err}");
        assert!(err.len() < 200, "the whole key must not be echoed: {err}");
    }

    // -----------------------------------------------------------------------
    // Over HTTP.
    //
    // These three rules moved here with the client and lost their tests on the
    // way: they lived in `rbx-config`'s own module, which this replaced. The
    // middle one is the reason the move is worth testing at all, and it is
    // restated in `get_config`'s doc comment because nothing else says it.
    // -----------------------------------------------------------------------

    mod over_http {
        use super::super::*;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const UNIVERSE: u64 = 109876543210987;

        fn repo_path(repository: Repository) -> String {
            format!(
                "/creator-configs-public-api/v1/configs/universes/{UNIVERSE}/repositories/{repository}"
            )
        }

        fn client(server: &MockServer, repository: Repository) -> ConfigsClient {
            ConfigsClient::new("test-key".into(), repository).with_base_url(server.uri())
        }

        /// The starting state, not a failure: a universe that has never
        /// published answers 404, and `sync` builds its first draft from that.
        #[tokio::test]
        async fn a_universe_with_no_config_yet_reads_as_empty() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(repo_path(Repository::InExperienceConfig)))
                .respond_with(ResponseTemplate::new(404).set_body_string(""))
                .mount(&server)
                .await;

            let snapshot = client(&server, Repository::InExperienceConfig)
                .get_config(UNIVERSE)
                .await
                .expect("404 means no config published, not a failure");
            assert!(snapshot.entries.is_empty());
            assert_eq!(snapshot.metadata.config_version, 0);
        }

        /// **The dangerous confusion.** Folding a refusal into "nothing
        /// published" would make `sync` diff against an empty remote and offer
        /// to overwrite a live config with everything in the file. A key
        /// missing one scope would silently become a config wipe.
        #[tokio::test]
        async fn a_refusal_is_not_mistaken_for_an_empty_config() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(repo_path(Repository::InExperienceConfig)))
                .respond_with(ResponseTemplate::new(403).set_body_string("no access"))
                .mount(&server)
                .await;

            let error = client(&server, Repository::InExperienceConfig)
                .get_config(UNIVERSE)
                .await
                .expect_err("403 is a failure")
                .to_string();
            assert!(error.contains("403"), "got: {error}");
        }

        /// Same rule, one level down. A draft nobody can read must not be
        /// reported as "no draft", or the concurrency guard silently sends no
        /// hash and the overwrite it was protecting goes through.
        #[tokio::test]
        async fn a_refused_draft_read_is_not_mistaken_for_an_absent_draft() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!(
                    "{}/draft",
                    repo_path(Repository::InExperienceConfig)
                )))
                .respond_with(ResponseTemplate::new(403).set_body_string("no access"))
                .mount(&server)
                .await;

            let error = client(&server, Repository::InExperienceConfig)
                .get_draft(UNIVERSE)
                .await
                .expect_err("403 is a failure")
                .to_string();
            assert!(error.contains("403"), "got: {error}");
        }

        #[tokio::test]
        async fn a_universe_with_no_draft_reads_as_an_absent_one() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!(
                    "{}/draft",
                    repo_path(Repository::InExperienceConfig)
                )))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;

            let draft = client(&server, Repository::InExperienceConfig)
                .get_draft(UNIVERSE)
                .await
                .expect("no draft is the ordinary state");
            assert!(draft.draft_hash.is_none());
            assert!(draft.entries.is_empty());
        }

        #[tokio::test]
        async fn the_api_key_is_sent_on_reads() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(repo_path(Repository::InExperienceConfig)))
                .and(header("x-api-key", "test-key"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
                .expect(1)
                .mount(&server)
                .await;

            client(&server, Repository::InExperienceConfig)
                .get_config(UNIVERSE)
                .await
                .unwrap();
        }

        /// The whole point of the move: the repository is a path segment, so a
        /// client built for one must not reach another's. Asserted by mounting
        /// only `DataStoresConfig` and letting a wrong path 404 into an empty
        /// snapshot, which would pass silently, so the request count is what
        /// actually proves it.
        #[tokio::test]
        async fn the_repository_is_the_path_segment_it_was_built_with() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(repo_path(Repository::DataStoresConfig)))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
                .expect(1)
                .mount(&server)
                .await;

            client(&server, Repository::DataStoresConfig)
                .get_config(UNIVERSE)
                .await
                .unwrap();

            let asked: Vec<String> = server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .map(|r| r.url.path().to_string())
                .collect();
            assert_eq!(asked.len(), 1, "one read, at one path: {asked:?}");
            assert!(
                asked[0].ends_with("/repositories/DataStoresConfig"),
                "got: {asked:?}"
            );
        }

        /// The guard, end to end: the draft is read first and its hash travels
        /// back as `draftHash`. Without this the overwrite discarded
        /// whatever was staged, in silence.
        #[tokio::test]
        async fn overwrite_and_publish_hands_back_the_hash_of_the_draft_it_read() {
            let server = MockServer::start().await;
            let base = repo_path(Repository::InExperienceConfig);

            Mock::given(method("GET"))
                .and(path(format!("{base}/draft")))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "draftHash": "staged-elsewhere",
                    "entries": { "someone.else": { "value": 1 } }
                })))
                .expect(1)
                .mount(&server)
                .await;

            Mock::given(method("PUT"))
                .and(path(format!("{base}/draft:overwrite")))
                .and(wiremock::matchers::body_json(serde_json::json!({
                    "entries": { "mine": true },
                    "draftHash": "staged-elsewhere"
                })))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"draftHash": "new"})),
                )
                .expect(1)
                .mount(&server)
                .await;

            Mock::given(method("POST"))
                .and(path(format!("{base}/publish")))
                .and(wiremock::matchers::body_json(serde_json::json!({
                    "message": "m",
                    "deploymentStrategy": "Immediate",
                    "draftHash": "new"
                })))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"configVersion": 7})),
                )
                .expect(1)
                .mount(&server)
                .await;

            let entries = BTreeMap::from([("mine".to_string(), Json::Bool(true))]);
            let (published, replaced) = client(&server, Repository::InExperienceConfig)
                .overwrite_and_publish(UNIVERSE, &entries, "m", "Immediate")
                .await
                .unwrap();

            assert_eq!(published.config_version, 7);
            // Named so a caller can say what it replaced, since the draft is
            // gone by the time anyone could ask.
            assert_eq!(replaced.keys, ["someone.else"]);
        }

        /// An entry set the limits refuse must cost no request at all: the
        /// validation is the first thing `overwrite_and_publish` does, ahead of
        /// even the draft read.
        #[tokio::test]
        async fn an_entry_set_over_the_limit_never_reaches_the_network() {
            let server = MockServer::start().await;
            let entries: BTreeMap<String, Json> = (0..=MAX_KEYS_PER_REPOSITORY)
                .map(|i| (format!("key{i}"), Json::from(i)))
                .collect();

            let error = client(&server, Repository::InExperienceConfig)
                .overwrite_and_publish(UNIVERSE, &entries, "m", "Immediate")
                .await
                .expect_err("over the limit")
                .to_string();
            assert!(error.contains("101 entries"), "got: {error}");
            assert!(
                server.received_requests().await.unwrap().is_empty(),
                "a payload the tool refuses must not cost a request"
            );
        }
    }
}
