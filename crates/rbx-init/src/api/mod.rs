pub mod groups;
pub mod places;
pub mod universes;

use anyhow::Result;
use rbx_core::api::{execute_with_retry, ApiBase, CsrfToken};
use reqwest::{header, multipart, Client, RequestBuilder, Response};

// One const per non-default host, named `<FIELD>_HOST` after the `Hosts`
// field it feeds. That naming is load-bearing: `rbx-spec-drift` resolves which
// host a `.join(...)` reaches by looking up `<RECEIVER>_HOST`, so a base whose
// const does not follow it silently gets attributed to `apis.roblox.com` and
// reported as drift that is not there.
const DEVELOP_HOST: &str = "https://develop.roblox.com";
const GROUPS_HOST: &str = "https://groups.roblox.com";
const USERS_HOST: &str = "https://users.roblox.com";
const GAMES_HOST: &str = "https://games.roblox.com";

const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct RbxClient {
    client: Client,
    cookie: Option<String>,
    csrf_token: CsrfToken,
    /// The five hosts this crate talks to, one field each.
    ///
    /// Injectable so the request shaping can be exercised against a mock
    /// server. Until this existed the URLs were `const`s and nothing here
    /// could run against a server of any kind, including the calls that
    /// create a real group, a real universe and a real place.
    ///
    /// Five rather than one because they are genuinely five services with
    /// different auth and different shapes; collapsing them would hide which
    /// call goes where, which is most of what this module has to get right.
    hosts: Hosts,
}

/// Every host `rbx-init` reaches, so a test can move them together.
pub(crate) struct Hosts {
    /// `apis.roblox.com`: universe and place creation.
    pub(crate) apis: ApiBase,
    /// `develop.roblox.com`: configuration reads and renames.
    pub(crate) develop: ApiBase,
    /// `groups.roblox.com`: group creation and membership.
    pub(crate) groups: ApiBase,
    /// `users.roblox.com`, who the cookie belongs to.
    pub(crate) users: ApiBase,
    /// `games.roblox.com`: listing a group's universes.
    pub(crate) games: ApiBase,
}

impl Default for Hosts {
    fn default() -> Self {
        Self {
            apis: ApiBase::default(),
            develop: ApiBase::new(DEVELOP_HOST),
            groups: ApiBase::new(GROUPS_HOST),
            users: ApiBase::new(USERS_HOST),
            games: ApiBase::new(GAMES_HOST),
        }
    }
}

impl RbxClient {
    pub fn new(cookie: Option<String>) -> Self {
        let client = rbx_core::api::build_client_with_user_agent(DEFAULT_UA);
        Self {
            client,
            cookie,
            csrf_token: CsrfToken::new(),
            hosts: Hosts::default(),
        }
    }

    /// Point every host at one server. Tests only.
    ///
    /// All five move together: a test wants one `wiremock` server answering
    /// whatever the code under test asks for, and splitting them would mean
    /// standing up five to exercise one call. Same shape `rbx-place` uses for
    /// its two.
    ///
    /// `cfg(test)` rather than `pub`: `RbxClient` is not re-exported, so
    /// outside the test build this is unreachable and `-D warnings` would
    /// reject it as dead code.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        self.hosts = Hosts {
            apis: ApiBase::new(url.clone()),
            develop: ApiBase::new(url.clone()),
            groups: ApiBase::new(url.clone()),
            users: ApiBase::new(url.clone()),
            games: ApiBase::new(url),
        };
        self
    }

    pub(crate) fn hosts(&self) -> &Hosts {
        &self.hosts
    }

    pub fn cookie_header(&self) -> Result<String> {
        self.optional_cookie_header().ok_or_else(|| {
            anyhow::anyhow!(
                "A .ROBLOSECURITY cookie is required for this command. Pass --cookie, set \
                 RBX_COOKIE, or sign in to Roblox Studio locally."
            )
        })
    }

    /// Build a `.ROBLOSECURITY=<value>` header if a cookie is available.
    /// Used by list endpoints that work without auth but need the cookie to reveal
    /// private resources (e.g. private universes / places owned by the user or group).
    ///
    /// The prefixing itself lives in `rbx_core::session`, so the session check
    /// and the calls it vouches for send the same bytes.
    pub fn optional_cookie_header(&self) -> Option<String> {
        self.cookie.as_deref().map(rbx_core::session::cookie_header)
    }

    /// Refuse to go on when the cookie is no longer a session (#63).
    ///
    /// Called by the five commands that create or rename something with it
    /// (`create-group`, `create-universe`, `create-place`, `rename-place`,
    /// `rename-universe`) before their first write. Not by the listings: they
    /// attach the cookie to a read that either answers or answers with less,
    /// and have nothing to leave behind.
    ///
    /// The check is one `users/authenticated` call cached per process by
    /// `rbx-core`, which is also where `list-groups` gets the account it lists
    /// for, so a run that does both spends one round trip, not two.
    ///
    /// A missing cookie is `cookie_header`'s error to raise, at the call that
    /// needs one.
    pub async fn require_valid_session(&self) -> Result<()> {
        let Some(cookie) = self.cookie.as_deref() else {
            return Ok(());
        };
        rbx_core::session::require_valid_with_host(&self.client, cookie, self.hosts.users.as_str())
            .await
    }

    /// The account the session check already identified, for a prompt.
    ///
    /// Never issues a request: `require_valid_session` has to have run first,
    /// and every caller here runs it immediately before asking. `None` when
    /// there is no cookie or the check could not answer, in which case the
    /// prompt stays exactly as it was.
    pub async fn known_account(&self) -> Option<rbx_core::session::SessionAccount> {
        let cookie = self.cookie.as_deref()?;
        rbx_core::session::known_account_with_host(cookie, self.hosts.users.as_str()).await
    }

    /// Execute a public (no-auth) GET.
    ///
    /// A pass-through to the shared helper, kept as a method because the name
    /// is what the call sites are saying: these endpoints take neither a
    /// cookie nor a key, and reading `execute_public` at the call site is how
    /// you know the missing auth is deliberate.
    ///
    /// It used to hold a private copy of the retry loop. The copy did not
    /// retry transient network errors and formatted the status into a string.
    pub async fn execute_public<F, Fut>(&self, make_request: F) -> Result<Response>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Response>>,
    {
        execute_with_retry(make_request).await
    }

    /// Send an authenticated request, retrying once on 403 with the refreshed CSRF token.
    /// Send a state-changing request, handling Roblox's CSRF token dance.
    ///
    /// The dance itself lives in `rbx_core::api::send_with_csrf`, which four
    /// crates carried their own copy of until they started disagreeing about
    /// whether `204` was a success. What stays here is the rendering of a
    /// refusal, which is this crate's to choose.
    pub async fn send_with_csrf<F>(&self, build: F) -> Result<Response>
    where
        F: Fn() -> RequestBuilder,
    {
        rbx_core::api::send_with_csrf(&self.csrf_token, build)
            .await
            .map_err(Into::into)
    }

    /// Authenticated JSON POST/PATCH/DELETE → parsed body.
    pub async fn auth_json<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let cookie = self.cookie_header()?;
        let response = self
            .send_with_csrf(|| {
                let mut req = self
                    .client
                    .request(method.clone(), url)
                    .header(header::COOKIE, &cookie);
                if let Some(b) = &body {
                    req = req.json(b);
                }
                req
            })
            .await?;
        let body_text = response.text().await?;
        serde_json::from_str(&body_text)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}\nBody: {}", e, body_text))
    }

    /// Authenticated multipart POST → parsed body. (Reqwest multipart can't be cloned,
    /// so we build the form fresh each attempt.)
    pub async fn auth_multipart<T, F>(
        &self,
        method: reqwest::Method,
        url: &str,
        build_form: F,
    ) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        F: Fn() -> multipart::Form,
    {
        let cookie = self.cookie_header()?;
        let response = self
            .send_with_csrf(|| {
                self.client
                    .request(method.clone(), url)
                    .header(header::COOKIE, &cookie)
                    .multipart(build_form())
            })
            .await?;
        let body_text = response.text().await?;
        serde_json::from_str(&body_text)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}\nBody: {}", e, body_text))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use reqwest::StatusCode;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> RbxClient {
        RbxClient::new(Some("test-cookie".into())).with_base_url(server.uri())
    }

    /// The retry Roblox's write endpoints require, and which had never run.
    ///
    /// Roblox answers the first authenticated write with `403` and the token
    /// to use in an `x-csrf-token` header. Without the retry every group,
    /// universe and place creation would fail on a fresh client.
    #[tokio::test]
    async fn a_403_carrying_a_csrf_token_is_retried_with_it() {
        let server = MockServer::start().await;
        // Answers the first call only, the way Roblox does: 403 plus the
        // token to use. `up_to_n_times(1)` is what makes the second call fall
        // through to the mock below.
        Mock::given(method("POST"))
            .and(path("/try"))
            .respond_with(ResponseTemplate::new(403).insert_header("x-csrf-token", "the-token"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // The retry, which only matches if the client actually carries the
        // token it was just handed.
        Mock::given(method("POST"))
            .and(path("/try"))
            .and(header("x-csrf-token", "the-token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let c = client(&server);
        let url = format!("{}/try", server.uri());
        let response = c.send_with_csrf(|| c.client.post(&url)).await.unwrap();

        assert!(response.status().is_success());
        // Asserted on the wire rather than on the cache: the token now lives
        // in `rbx_core`, and what this crate owns is that its writes go
        // through the dance at all. Two requests, the second carrying the
        // token, is that fact from the outside.
        let seen = server.received_requests().await.unwrap();
        assert_eq!(seen.len(), 2, "one refusal, one retry");
        assert_eq!(
            seen[1].headers.get("x-csrf-token").unwrap(),
            "the-token",
            "the retry echoes the token Roblox handed back"
        );
    }

    /// A 403 that carries no token is a real refusal (a cookie that is not
    /// allowed to do this) and must surface rather than spin.
    #[tokio::test]
    async fn a_403_without_a_token_is_an_error_and_not_a_retry_loop() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/try"))
            .respond_with(ResponseTemplate::new(403).set_body_string("nope"))
            .mount(&server)
            .await;

        let c = client(&server);
        let url = format!("{}/try", server.uri());
        let error = c.send_with_csrf(|| c.client.post(&url)).await.unwrap_err();

        assert!(error.to_string().contains("403"), "got: {error}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "no token to retry with means one attempt, not two"
        );
    }

    /// The loop is bounded. A server that answers 403-with-token forever must
    /// end in an error rather than hanging the command.
    #[tokio::test]
    async fn a_server_that_always_asks_for_a_new_token_gives_up() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/try"))
            .respond_with(ResponseTemplate::new(403).insert_header("x-csrf-token", "another"))
            .mount(&server)
            .await;

        let c = client(&server);
        let url = format!("{}/try", server.uri());
        let error = c.send_with_csrf(|| c.client.post(&url)).await.unwrap_err();

        assert!(error.to_string().contains("403"), "got: {error}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "one retry, then stop"
        );
    }

    /// #63. The five creating and renaming commands ask this before their
    /// first write, so an expired session costs a re-run rather than a group
    /// nobody paid for or a universe named after nothing.
    #[tokio::test]
    async fn an_expired_session_is_refused_with_the_way_to_renew_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/users/authenticated"))
            .respond_with(ResponseTemplate::new(401).set_body_string("{}"))
            .mount(&server)
            .await;

        let error = RbxClient::new(Some("init-expired".into()))
            .with_base_url(server.uri())
            .require_valid_session()
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("expired"), "got {error}");
        assert!(error.contains("Roblox Studio"), "got {error}");
        assert!(
            !error.contains("401"),
            "a status code is not a remedy: {error}"
        );
    }

    /// Roblox being unreachable is not a dead session. The write that follows
    /// will report the network for what it is.
    #[tokio::test]
    async fn an_unreachable_service_does_not_become_an_expired_session() {
        let client =
            RbxClient::new(Some("init-offline".into())).with_base_url("http://127.0.0.1:1");

        assert!(client.require_valid_session().await.is_ok());
    }

    #[tokio::test]
    async fn a_204_counts_as_success_rather_than_a_failure_to_parse() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/try"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let c = client(&server);
        let url = format!("{}/try", server.uri());
        assert_eq!(
            c.send_with_csrf(|| c.client.post(&url))
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
    }
}
