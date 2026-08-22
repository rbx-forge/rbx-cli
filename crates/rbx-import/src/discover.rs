//! Resolving what a universe *is*, before anything is written.
//!
//! Three answers are needed to lay down `rbxplace.toml`: the universe's own
//! name, who owns it, and which places it contains. The first two come from
//! Open Cloud with nothing but an API key; the place list only exists on the
//! legacy `develop` host, which is why that call is cookie-aware but does not
//! require one.
//!
//! Nothing here writes a file. A universe id that turns out to be wrong, or a
//! key without the scope to read it, has to fail before `import` has created
//! anything on disk.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use rbx_core::api::{ApiBase, DEFAULT_API_BASE};

/// Who a universe belongs to, in the shape `rbxplace.toml`'s `[owner]` block
/// wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    /// `user` or `group`.
    pub kind: &'static str,
    pub id: u64,
}

/// One place in the universe, as it will be written under `[<env>.places]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// Key for the TOML table: slugified, and `main` for the root place.
    pub key: String,
    pub id: u64,
    /// What Roblox calls it, kept for the progress line only.
    pub display_name: String,
}

/// Everything `import` learned about the universe before writing anything.
#[derive(Debug, Clone)]
pub struct Universe {
    pub id: u64,
    pub display_name: Option<String>,
    pub owner: Option<Owner>,
    /// Root place first, then the rest in the order Roblox returned them.
    pub places: Vec<Place>,
}

impl Universe {
    /// The place `rbx meta` operates on. Always present: a universe without a
    /// root place is not a universe.
    pub fn root_place(&self) -> Result<&Place> {
        self.places.first().ok_or_else(|| {
            anyhow::anyhow!(
                "Universe {} reported no places. Check the id, and that the key or cookie can \
                 see it.",
                self.id
            )
        })
    }
}

// ---------------------------------------------------------------------------
// Open Cloud: the universe itself
// ---------------------------------------------------------------------------

/// Only the fields `import` reads. `rbx-meta` models the rest of this payload
/// and is the crate that owns metadata; duplicating its struct here would be
/// two places to update when Roblox adds a field.
#[derive(Debug, Deserialize)]
struct CloudUniverse {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    /// `users/123` when a user owns it.
    user: Option<String>,
    /// `groups/456` when a group does.
    group: Option<String>,
}

/// Read the universe over Open Cloud.
///
/// `base` is injectable so the whole resolution can run against a mock server;
/// production passes [`DEFAULT_API_BASE`].
pub async fn fetch_universe(
    client: &reqwest::Client,
    base: &ApiBase,
    api_key: &str,
    universe_id: u64,
) -> Result<(Option<String>, Option<Owner>)> {
    let url = base.join(&format!("/cloud/v2/universes/{universe_id}"));
    let universe: CloudUniverse = rbx_core::api::execute_json(|| async {
        Ok(client.get(&url).header("x-api-key", api_key).send().await?)
    })
    .await
    .with_context(|| format!("Failed to read universe {universe_id} over Open Cloud"))?;

    let owner = match (&universe.user, &universe.group) {
        // A universe has one or the other, never both. If Roblox ever sends
        // both, the group is the meaningful one for a payment source.
        (_, Some(group)) => parse_owner("group", group),
        (Some(user), None) => parse_owner("user", user),
        (None, None) => None,
    };

    Ok((universe.display_name, owner))
}

/// `users/123` -> `Owner { kind: "user", id: 123 }`. A shape that does not
/// parse yields `None` rather than an error: the owner is a convenience for
/// `[owner]`, and failing the whole import over it would be out of proportion.
fn parse_owner(kind: &'static str, path: &str) -> Option<Owner> {
    path.rsplit('/')
        .next()?
        .parse()
        .ok()
        .map(|id| Owner { kind, id })
}

// ---------------------------------------------------------------------------
// Legacy develop host: the place list
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DevelopUniverse {
    #[serde(rename = "rootPlaceId", default)]
    root_place_id: u64,
}

#[derive(Debug, Deserialize)]
struct DevelopPlace {
    #[serde(default, alias = "placeId")]
    id: u64,
    #[serde(default, alias = "displayName")]
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevelopPlacesPage {
    data: Vec<DevelopPlace>,
    next_page_cursor: Option<String>,
}

/// List the universe's places, root first.
///
/// Open Cloud has no endpoint for this (`/cloud/v2/universes/{id}/places/{p}`
/// reads one place but cannot enumerate them) so this is the legacy host.
///
/// The cookie is accepted and buys nothing. Measured against a private
/// universe with no credential of any kind, this listing answers 200 with
/// every place and its name; visibility does not gate it. The parameter stays
/// because the caller has one to pass and sending it costs nothing, not
/// because a private universe needs it.
///
/// See `docs/cookie.md`, "What the cookie does not protect".
pub async fn fetch_places(
    client: &reqwest::Client,
    develop: &ApiBase,
    cookie: Option<&str>,
    universe_id: u64,
) -> Result<Vec<Place>> {
    let root_id = fetch_root_place_id(client, develop, cookie, universe_id).await?;

    let mut raw = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url = develop.join(&format!(
            "/v1/universes/{universe_id}/places?isUniverseCreation=false&limit=100&sortOrder=Asc"
        ));
        if let Some(c) = &cursor {
            // Encoded, not pasted: the cursor is an opaque token and a `+` or
            // `&` in it would silently re-request page one for ever.
            url.push_str("&cursor=");
            url.push_str(&rbx_core::api::encode_query_value(c));
        }
        let page: DevelopPlacesPage = rbx_core::api::execute_json(|| async {
            let mut req = client.get(&url);
            if let Some(c) = cookie {
                req = req.header(reqwest::header::COOKIE, c);
            }
            Ok(req.send().await?)
        })
        .await
        .with_context(|| format!("Failed to list places for universe {universe_id}"))?;

        raw.extend(page.data);
        match page.next_page_cursor {
            Some(c) if !c.is_empty() => cursor = Some(c),
            _ => break,
        }
    }

    // The root place may not come back in the listing at all (it does not for
    // some universes), so it is seeded rather than searched for.
    let root_name = raw
        .iter()
        .find(|p| p.id == root_id)
        .map(|p| p.name.clone())
        .unwrap_or_default();

    let mut places = vec![Place {
        // `main` is what the whole toolkit already treats as the default entry
        // (`rbx_core::places::resolve` prefers it), so the root place takes it
        // regardless of what Roblox calls it.
        key: "main".to_string(),
        id: root_id,
        display_name: root_name,
    }];

    for place in raw {
        if place.id == root_id {
            continue;
        }
        let key = unique_key(&slugify(&place.name), &places);
        places.push(Place {
            key,
            id: place.id,
            display_name: place.name,
        });
    }

    Ok(places)
}

async fn fetch_root_place_id(
    client: &reqwest::Client,
    develop: &ApiBase,
    cookie: Option<&str>,
    universe_id: u64,
) -> Result<u64> {
    let url = develop.join(&format!("/v1/universes/{universe_id}"));
    let info: DevelopUniverse = rbx_core::api::execute_json(|| async {
        let mut req = client.get(&url);
        if let Some(c) = cookie {
            req = req.header(reqwest::header::COOKIE, c);
        }
        Ok(req.send().await?)
    })
    .await
    .with_context(|| format!("Failed to resolve the root place of universe {universe_id}"))?;

    if info.root_place_id == 0 {
        // Deliberately does not suggest --cookie. This endpoint answers 200 on
        // a private universe with no credential at all, so a cookie cannot be
        // the missing piece and suggesting it sends the reader to fix the one
        // thing that was never wrong. A bad id is what is left.
        bail!(
            "Could not resolve the root place of universe {universe_id}. Check the id: this \
             listing answers without any credential, so a missing cookie is not the cause. \
             A place id passed where a universe id belongs is the usual mistake."
        );
    }
    Ok(info.root_place_id)
}

/// A place name reduced to something usable as a TOML key.
fn slugify(name: &str) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "place".to_string()
    } else {
        slug
    }
}

/// Two places can share a display name; two TOML keys cannot.
fn unique_key(base: &str, taken: &[Place]) -> String {
    if !taken.iter().any(|p| p.key == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|candidate| !taken.iter().any(|p| &p.key == candidate))
        .expect("the range is unbounded, so some suffix is always free")
}

/// The hosts `import` reads from. Injectable as one unit so a test can point
/// every call at a single mock server.
#[derive(Debug)]
pub struct Hosts {
    /// `apis.roblox.com`: the universe itself.
    pub cloud: ApiBase,
    /// `develop.roblox.com`: the place list, which Open Cloud does not serve.
    pub develop: ApiBase,
}

pub const DEVELOP_HOST: &str = "https://develop.roblox.com";

impl Default for Hosts {
    fn default() -> Self {
        Self {
            cloud: ApiBase::new(DEFAULT_API_BASE),
            develop: ApiBase::new(DEVELOP_HOST),
        }
    }
}

impl Hosts {
    /// Point both hosts at one server. Tests only.
    pub fn with_base_url(url: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            cloud: ApiBase::new(url.clone()),
            develop: ApiBase::new(url),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn an_owner_path_keeps_only_the_id() {
        assert_eq!(
            parse_owner("user", "users/123"),
            Some(Owner {
                kind: "user",
                id: 123
            })
        );
        assert_eq!(
            parse_owner("group", "groups/456"),
            Some(Owner {
                kind: "group",
                id: 456
            })
        );
    }

    /// An owner rbx cannot parse is skipped, not fatal: `[owner]` is a
    /// convenience, and failing the import over it would be out of proportion.
    #[test]
    fn an_unparseable_owner_is_none_rather_than_an_error() {
        assert_eq!(parse_owner("user", "users/not-a-number"), None);
        assert_eq!(parse_owner("user", ""), None);
    }

    #[test]
    fn place_names_become_toml_keys() {
        assert_eq!(slugify("Main Menu"), "main_menu");
        assert_eq!(slugify("  Lobby!  "), "lobby");
        assert_eq!(slugify("The Arena (v2)"), "the_arena__v2");
        // Nothing usable left: a key is still required.
        assert_eq!(slugify("!!!"), "place");
    }

    /// Roblox lets two places share a display name. Colliding keys would make
    /// the second silently replace the first in the TOML table.
    #[test]
    fn a_colliding_key_gets_a_suffix() {
        let taken = vec![
            Place {
                key: "main".into(),
                id: 1,
                display_name: "Main".into(),
            },
            Place {
                key: "lobby".into(),
                id: 2,
                display_name: "Lobby".into(),
            },
        ];
        assert_eq!(unique_key("arena", &taken), "arena");
        assert_eq!(unique_key("lobby", &taken), "lobby_2");
    }

    /// A cursor is an opaque token. Roblox returns base64url today, but the
    /// value is theirs to change, and one pasted raw into the query string is
    /// re-parsed by the server as something else: `a+b` decodes to `a b`, and
    /// everything after an `&` becomes a separate parameter. Both ask for page
    /// one again, so the place listing loops on the first page for ever rather
    /// than erroring, and an import silently writes only the first hundred
    /// places.
    #[tokio::test]
    async fn a_cursor_with_reserved_characters_reaches_the_server_intact() {
        use serde_json::json;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const UNIVERSE: u64 = 99887766554;
        const CURSOR: &str = "a+b/c=d&e f";

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/universes/{UNIVERSE}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "rootPlaceId": 1 })))
            .mount(&server)
            .await;

        let list_path = format!("/v1/universes/{UNIVERSE}/places");
        Mock::given(method("GET"))
            .and(path(list_path.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 1, "name": "Main" }],
                "nextPageCursor": CURSOR
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(list_path))
            .and(query_param("cursor", CURSOR))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 2, "name": "Lobby" }],
                "nextPageCursor": ""
            })))
            .mount(&server)
            .await;

        let hosts = Hosts::with_base_url(server.uri());
        let places = fetch_places(
            &rbx_core::api::build_client(),
            &hosts.develop,
            None,
            UNIVERSE,
        )
        .await
        .unwrap();

        assert_eq!(places.len(), 2, "the second page must have been fetched");
        assert_eq!(places[1].key, "lobby");
    }
}
