use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct UploadVersionResponse {
    #[serde(rename = "versionNumber")]
    pub version_number: u64,
}

#[derive(Debug, Deserialize)]
pub struct AssetDeliveryResponse {
    pub location: String,
}

#[derive(Debug, Deserialize)]
pub struct AssetVersionsPage {
    #[serde(rename = "assetVersions", default)]
    pub asset_versions: Vec<AssetVersionEntry>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AssetVersionEntry {
    pub path: String,
    #[serde(rename = "createTime")]
    pub create_time: String,
    pub published: Option<bool>,
}

impl AssetVersionEntry {
    pub fn version_number(&self) -> Option<u64> {
        self.path.split('/').next_back()?.parse().ok()
    }
}

/// Parsed version info returned by list_versions.
#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub version_number: u64,
    pub create_time: String,
    pub published: bool,
}

impl VersionInfo {
    /// Human-readable date: "2024-01-15 14:30 UTC"
    pub fn display_time(&self) -> String {
        self.create_time
            .replace('T', " ")
            .trim_end_matches('Z')
            .to_string()
            + " UTC"
    }
}

#[derive(Debug, Deserialize)]
pub struct RollbackResponse {
    pub path: String,
}

impl RollbackResponse {
    pub fn version_number(&self) -> Option<u64> {
        self.path.split('/').next_back()?.parse().ok()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct PlaceEntry {
    pub path: String,
    #[serde(rename = "displayName", default)]
    pub display_name: String,
    #[serde(rename = "maxPlayerCount", default)]
    pub max_player_count: u64,
}

impl PlaceEntry {
    pub fn place_id(&self) -> Option<u64> {
        self.path.split('/').next_back()?.parse().ok()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct DevelopPlacesPage {
    #[serde(rename = "nextPageCursor", default)]
    pub next_page_cursor: Option<String>,
    pub data: Vec<DevelopPlaceEntry>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DevelopPlaceEntry {
    pub id: u64,
    pub name: String,
}

impl From<DevelopPlaceEntry> for PlaceEntry {
    fn from(entry: DevelopPlaceEntry) -> Self {
        PlaceEntry {
            path: format!("places/{}", entry.id),
            display_name: entry.name,
            max_player_count: 0,
        }
    }
}
