use serde::{Deserialize, Serialize};

use crate::config::SocialLink;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Universe {
    #[serde(default)]
    pub path: Option<String>,

    #[serde(default)]
    pub display_name: Option<String>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub user: Option<String>,

    #[serde(default)]
    pub group: Option<String>,

    #[serde(default)]
    pub visibility: Option<String>,

    #[serde(default)]
    pub age_rating: Option<String>,

    #[serde(default)]
    pub voice_chat_enabled: Option<bool>,

    #[serde(default)]
    pub private_server_price_robux: Option<u64>,

    #[serde(default)]
    pub desktop_enabled: Option<bool>,
    #[serde(default)]
    pub mobile_enabled: Option<bool>,
    #[serde(default)]
    pub tablet_enabled: Option<bool>,
    #[serde(default)]
    pub console_enabled: Option<bool>,
    #[serde(default)]
    pub vr_enabled: Option<bool>,

    #[serde(default)]
    pub facebook_social_link: Option<ApiSocialLink>,
    #[serde(default)]
    pub twitter_social_link: Option<ApiSocialLink>,
    #[serde(default)]
    pub youtube_social_link: Option<ApiSocialLink>,
    #[serde(default)]
    pub twitch_social_link: Option<ApiSocialLink>,
    #[serde(default)]
    pub discord_social_link: Option<ApiSocialLink>,
    #[serde(default)]
    pub roblox_group_social_link: Option<ApiSocialLink>,
    #[serde(default)]
    pub guilded_social_link: Option<ApiSocialLink>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiSocialLink {
    pub title: String,
    pub uri: String,
}

impl From<&SocialLink> for ApiSocialLink {
    fn from(link: &SocialLink) -> Self {
        Self {
            title: link.title.clone(),
            uri: link.url.clone(),
        }
    }
}

impl From<&ApiSocialLink> for SocialLink {
    fn from(link: &ApiSocialLink) -> Self {
        Self {
            title: link.title.clone(),
            url: link.uri.clone(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    #[serde(default)]
    pub path: Option<String>,

    #[serde(default)]
    pub display_name: Option<String>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub server_size: Option<u32>,

    #[serde(default)]
    pub root: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct IconUploadResponse {
    #[serde(default, alias = "mediaAssetId", alias = "imageId", alias = "targetId")]
    pub image_id: Option<u64>,
    /// Parsed from the response but not consumed today.
    #[serde(default, rename = "languageCode")]
    #[allow(dead_code)]
    pub language_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ThumbnailUploadResponse {
    /// Roblox sends this as a JSON **string**: the document gives the response
    /// as `{mediaAssetId: string}`. Read as a plain `u64`, the whole body failed
    /// to parse, the failure was swallowed, and every upload was recorded in
    /// the lockfile without an id. The next sync then saw nothing it could
    /// match, uploaded the same images again, and Roblox kept every copy.
    /// A number is accepted too, in case Roblox ever sends one.
    #[serde(
        default,
        alias = "mediaAssetId",
        alias = "imageId",
        alias = "targetId",
        deserialize_with = "id_from_string_or_number"
    )]
    pub image_id: Option<u64>,
}

/// An id that may arrive as `"123"` or as `123`.
fn id_from_string_or_number<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(u64),
        Text(String),
    }

    match Option::<Raw>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Raw::Number(id)) => Ok(Some(id)),
        Some(Raw::Text(text)) => text
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| serde::de::Error::custom(format!("{text:?} is not a media asset id"))),
    }
}
