use anyhow::{bail, Context, Result};
use reqwest::{header, multipart};
use serde::{Deserialize, Serialize};

use super::RbxClient;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGroupResponse {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub owner: Option<GroupOwner>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GroupOwner {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserGroupEntry {
    pub group: UserGroupInfo,
    pub role: UserGroupRole,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserGroupInfo {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserGroupRole {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub rank: u32,
}

#[derive(Debug, Deserialize)]
struct AuthenticatedUser {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct UserGroupsResponse {
    data: Vec<UserGroupEntry>,
}

impl RbxClient {
    pub async fn create_group(
        &self,
        name: &str,
        description: &str,
        public_group: bool,
        icon_bytes: Vec<u8>,
        icon_name: String,
    ) -> Result<CreateGroupResponse> {
        let mime = guess_image_mime(&icon_name);
        let build_form = || {
            let part = multipart::Part::bytes(icon_bytes.clone())
                .file_name(icon_name.clone())
                .mime_str(mime)
                .expect("static mime string is valid");
            multipart::Form::new()
                .text("name", name.to_string())
                .text("description", description.to_string())
                .text("publicGroup", public_group.to_string())
                .text("buildersClubMembersOnly", "false".to_string())
                .part("Files", part)
        };

        match self
            .auth_multipart(
                reqwest::Method::POST,
                &self.hosts().groups.join("/v1/groups/create"),
                build_form,
            )
            .await
        {
            Ok(resp) => Ok(resp),
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("name has been taken") || msg.contains("name is taken") {
                    bail!(
                        "Group name '{}' is already taken on Roblox. Pick a different name.",
                        name
                    );
                }
                if msg.contains("inappropriate") || msg.contains("moderated") {
                    bail!(
                        "Group name '{}' was rejected by Roblox moderation. Try another.",
                        name
                    );
                }
                Err(e).context("Failed to create group")
            }
        }
    }

    pub async fn list_authenticated_user_groups(&self) -> Result<Vec<UserGroupEntry>> {
        let cookie = self.cookie_header()?;

        let user: AuthenticatedUser = {
            let response = self
                .execute_public(|| async {
                    Ok(self
                        .client
                        .get(self.hosts().users.join("/v1/users/authenticated"))
                        .header(header::COOKIE, &cookie)
                        .send()
                        .await?)
                })
                .await?;
            let body = response.text().await?;
            serde_json::from_str(&body).map_err(|e| {
                anyhow::anyhow!("Failed to parse authenticated user: {}\nBody: {}", e, body)
            })?
        };

        let groups = &self.hosts().groups;
        let url = groups.join(&format!("/v2/users/{}/groups/roles", user.id));
        let response = self
            .execute_public(|| async { Ok(self.client.get(&url).send().await?) })
            .await?;
        let body = response.text().await?;
        let parsed: UserGroupsResponse = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Failed to parse groups: {}\nBody: {}", e, body))?;
        Ok(parsed.data)
    }
}

fn guess_image_mime(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "image/png"
    }
}
