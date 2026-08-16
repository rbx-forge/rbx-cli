use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Response from GET /repositories/InExperienceConfig
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

/// Response from PATCH /draft and PUT /draft:overwrite
#[derive(Debug, Deserialize)]
pub struct DraftResult {
    #[serde(rename = "draftHash")]
    pub draft_hash: Option<String>,
}

/// Response from POST /publish
#[derive(Debug, Deserialize)]
pub struct PublishResult {
    #[serde(rename = "configVersion")]
    pub config_version: u64,
}

/// Body for PUT /draft:overwrite
#[derive(Debug, Serialize)]
pub struct OverwriteBody<'a> {
    pub entries: &'a BTreeMap<String, Json>,
    #[serde(rename = "previousDraftHash", skip_serializing_if = "Option::is_none")]
    pub previous_draft_hash: Option<&'a str>,
}

/// Body for POST /publish
#[derive(Debug, Serialize)]
pub struct PublishBody<'a> {
    pub message: &'a str,
    #[serde(rename = "deploymentStrategy")]
    pub deployment_strategy: &'a str,
    #[serde(rename = "draftHash", skip_serializing_if = "Option::is_none")]
    pub draft_hash: Option<&'a str>,
}

/// Single revision entry from GET /revisions
#[derive(Debug, Clone, Deserialize)]
pub struct RevisionEntry {
    #[serde(rename = "revisionId")]
    pub revision_id: String,
    pub version: u64,
    pub time: String,
    #[serde(default)]
    pub message: Option<String>,
    /// Per-key change payloads. Only the key set is surfaced (`rbx config
    /// versions` reports a count), so this holds raw `Json` rather than a
    /// typed `{ before, after }` nobody reads: a speculative schema silently
    /// stops matching the API, and `#[allow(dead_code)]` is what makes the
    /// drift invisible. Anything that wants the payloads later can type it
    /// then, against what the endpoint actually sends.
    #[serde(default)]
    pub changes: HashMap<String, Json>,
}

/// Response from GET /revisions
#[derive(Debug, Deserialize)]
pub struct ListRevisionsResponse {
    #[serde(default)]
    pub revisions: Vec<RevisionEntry>,
}

/// Response from POST /revisions/{id}/restore
#[derive(Debug, Deserialize)]
pub struct RestoreResponse {
    #[serde(rename = "draftHash")]
    pub draft_hash: String,
}
