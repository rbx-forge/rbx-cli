pub mod experience_releases;
pub mod legacy;
pub mod media;
pub mod models;
pub mod places;
pub mod universe;

use anyhow::Result;
use rbx_core::api::CsrfToken;
use reqwest::{Client, Response};

use rbx_core::api::ApiBase;

/// The cookie-authenticated legacy service, for the fields Open Cloud v2 still
/// does not expose.
pub(crate) const LEGACY_HOST: &str = "https://develop.roblox.com";

/// Where "is this session still good" is asked, once, before the first
/// cookie-authenticated write. See `rbx_core::session`.
pub(crate) const USERS_HOST: &str = "https://users.roblox.com";

pub struct RbxClient {
    client: Client,
    api_key: Option<String>,
    cookie: Option<String>,
    csrf_token: CsrfToken,
    universe_id: u64,
    pub place_id: u64,
    /// Reserved: meta does not currently apply icon bleed (unlike rbx-shop).
    #[allow(dead_code)]
    bleed: bool,
    pub language_code: String,
    /// Where the `apis.roblox.com` endpoints live.
    ///
    /// Injectable so the request shaping can be exercised against a mock
    /// server. Until this existed the URLs were built inline, and nothing in
    /// this crate could be tested over HTTP, including the two PATCHes that
    /// write to a live universe and a live place.
    ///
    /// `thumbnails.roblox.com` (public reads) is a different service and stays
    /// a literal until a test needs it, the same call `rbx-apikey` made for its
    /// introspect URL.
    base: ApiBase,

    /// Where the `develop.roblox.com` endpoints live.
    ///
    /// These used to be literals, on the stated reasoning that they should stay
    /// that way "until a test needs them". A test needs them: the visibility
    /// ordering in `sync` (activate before every other call when going public,
    /// deactivate after them all when going private, because Roblox rejects
    /// `privateServerPriceRobux > 0` on a private universe) runs entirely
    /// through `activate_universe` / `deactivate_universe` and was therefore
    /// unreachable from any test. The decision is covered in `diff`; the order
    /// was not covered anywhere.
    legacy_base: ApiBase,

    /// Where `users.roblox.com` lives, for the session check alone.
    ///
    /// A third base rather than a reuse of `legacy_base`: it is a third
    /// service, and a test that mocks the develop endpoints must not have its
    /// session check silently answered by the same mock: the whole point of
    /// the check is that it is a separate question with a separate answer.
    users_base: ApiBase,
}

impl RbxClient {
    pub fn new(
        api_key: Option<String>,
        cookie: Option<String>,
        universe_id: u64,
        place_id: u64,
        bleed: bool,
        language_code: String,
    ) -> Self {
        Self {
            client: rbx_core::api::build_client(),
            api_key,
            cookie,
            csrf_token: CsrfToken::new(),
            universe_id,
            place_id,
            bleed,
            language_code,
            base: ApiBase::default(),
            legacy_base: ApiBase::new(LEGACY_HOST),
            users_base: ApiBase::new(USERS_HOST),
        }
    }

    /// Point the `apis.roblox.com` endpoints at another host. Tests only, and
    /// compiled only for them: the module is private, so a `pub` nothing in a
    /// normal build calls is dead code rather than API. Same guard
    /// `rbx-apikey` uses for its own.
    #[cfg(test)]
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = ApiBase::new(base);
        self
    }

    /// Point the `develop.roblox.com` endpoints at another host. Tests only,
    /// for the same reason as [`RbxClient::with_base_url`].
    ///
    /// Separate from it because they are separate services: a test that mocks
    /// one and not the other should still send the other's traffic to the real
    /// host and fail loudly, rather than have it silently answered by a mock
    /// that was never told about it.
    #[cfg(test)]
    pub fn with_legacy_base_url(mut self, base: impl Into<String>) -> Self {
        self.legacy_base = ApiBase::new(base);
        self
    }

    /// Point the session check at another host. Tests only, for the same
    /// reason as `RbxClient::with_base_url`.
    #[cfg(test)]
    pub fn with_users_base_url(mut self, base: impl Into<String>) -> Self {
        self.users_base = ApiBase::new(base);
        self
    }

    /// Refuse to go on when the cookie is no longer a session.
    ///
    /// One `users/authenticated` call, cached per process by `rbx_core`, so a
    /// sync that writes several cookie-only fields asks once. Called before the
    /// first write of an apply, never on a read: `meta init` and `meta pull`
    /// attach the cookie to reads that either answer or report the fields they
    /// could not read, and have nothing to leave half-applied.
    ///
    /// A missing cookie is not this function's error to raise: `sync` names
    /// the fields that need one before it gets here, which is a better message
    /// than anything available at this level.
    pub async fn require_valid_session(&self) -> Result<()> {
        let Some(cookie) = self.cookie.as_deref() else {
            return Ok(());
        };
        rbx_core::session::require_valid_with_host(&self.client, cookie, self.users_base.as_str())
            .await
    }

    /// `https://apis.roblox.com/<path>`, or the mock server's equivalent.
    pub(crate) fn api_url(&self, path: &str) -> String {
        self.base.join(path)
    }

    /// `https://develop.roblox.com/<path>`, or the mock server's equivalent.
    pub(crate) fn legacy_url(&self, path: &str) -> String {
        self.legacy_base.join(path)
    }

    pub fn api_key_header(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!("--api-key or RBX_API_KEY env var is required for this operation")
        })
    }

    /// The account the session check already identified, for a prompt.
    ///
    /// Never issues a request: `require_valid_session` runs before the
    /// confirmation in `sync`, so the answer is cached by the time this is
    /// asked. `None` when the plan needs no cookie, or the check could not
    /// answer, and the question then stays exactly as it was.
    pub async fn known_account(&self) -> Option<rbx_core::session::SessionAccount> {
        let cookie = self.cookie.as_deref()?;
        rbx_core::session::known_account_with_host(cookie, self.users_base.as_str()).await
    }

    pub fn cookie_header(&self) -> Result<&str> {
        self.cookie.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "A .ROBLOSECURITY cookie is required for this field: pass --cookie, set \
                 RBX_COOKIE, or sign in to Roblox Studio locally"
            )
        })
    }

    pub fn has_cookie(&self) -> bool {
        self.cookie.is_some()
    }

    /// Retry + JSON-parse wrapper: delegates to `rbx_core::api`.
    pub async fn execute_json<T: serde::de::DeserializeOwned, F, Fut>(
        &self,
        make_request: F,
    ) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Response>>,
    {
        rbx_core::api::execute_json(make_request).await
    }
}
