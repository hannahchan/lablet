//! Where the adapter sends, made once so that whoever checks a config sees
//! what building the adapter refuses: the messages URL, the endpoint a run
//! reports, the key and a gateway's credentials as header values, the
//! model, and the HTTP client with its two policies.

use std::fmt;

use base64::Engine as _;
use lablet_model::{Endpoint, ModelRef, ProviderApi};
use reqwest::header::HeaderValue;
use reqwest::{Client, Url, redirect, retry};

/// Where the API is served when no base URL is given.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The path of the Messages API under a base URL.
const MESSAGES_PATH: &str = "/v1/messages";

/// What a base URL is, said where one is refused.
const BASE_URL_SHAPE: &str = "a base URL names a scheme, a host, an optional port and a path, as \
                              https://gateway.example/anthropic does, and has no query or fragment";

/// The API key, as a type of its own so that it can't be swapped with the
/// model name at the call, and so that no `Debug` of it shows the key.
#[derive(Clone, Copy)]
pub struct Key<'a>(pub &'a str);

impl fmt::Debug for Key<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[secret]")
    }
}

/// Why a target couldn't be made. Each is a refusal of the config at
/// `check`, and the reason shows nothing of the key and nothing of the URL
/// but its shape.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    /// The base URL isn't one the adapter sends to.
    #[error("{reason}")]
    BaseUrl {
        /// What a base URL is.
        reason: String,
    },
    /// The key isn't a header value.
    #[error("{reason}")]
    Key {
        /// What a key is.
        reason: String,
    },
    /// The HTTP client couldn't be made.
    #[error("{reason}")]
    Client {
        /// The client's own reason, with no URL in it.
        reason: String,
    },
}

/// Where one adapter sends: the messages URL with no user information in
/// it, the endpoint a run reports, the key and a gateway's `Basic`
/// credentials as header values, the model, and the client that sends.
///
/// Its `Debug` shows each credential as the header value shows itself,
/// `Sensitive`, so a value that lost that mark would show in a test.
pub struct Target {
    url: Url,
    endpoint: Endpoint,
    key: HeaderValue,
    authorization: Option<HeaderValue>,
    model: ModelRef,
    cache: bool,
    client: Client,
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("url", &self.url.as_str())
            .field("endpoint", &self.endpoint)
            .field("key", &self.key)
            .field("authorization", &self.authorization)
            .field("model", &self.model)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl Target {
    /// The target of `model` at `base_url`, `https://api.anthropic.com` when
    /// there's none, reached with `key`, with `cache_control` markers sent
    /// when `cache` is on.
    ///
    /// The base URL's user information is taken off the URL and sent as an
    /// `Authorization: Basic` header of its own, so the URL the client holds
    /// carries none. Making the client loads the platform's trust roots and
    /// opens no connection.
    ///
    /// # Errors
    ///
    /// Returns [`TargetError::BaseUrl`] for a base URL that doesn't parse,
    /// isn't `http` or `https`, has no host, or has a query or a fragment,
    /// since a query is a credential nowhere registered;
    /// [`TargetError::Key`] for a key a header can't carry; and
    /// [`TargetError::Client`] when the HTTP client can't be made.
    pub fn new(
        model: &str,
        key: Key<'_>,
        base_url: Option<&str>,
        cache: bool,
    ) -> Result<Self, TargetError> {
        let mut url = parsed(base_url.unwrap_or(DEFAULT_BASE_URL))?;
        let authorization = credentials(&mut url)?;
        let endpoint = Endpoint {
            host: url
                .host_str()
                .map(str::to_owned)
                .ok_or_else(base_url_refused)?,
            port: url.port_or_known_default().ok_or_else(base_url_refused)?,
        };
        let path = format!("{}{MESSAGES_PATH}", url.path().trim_end_matches('/'));
        url.set_path(&path);
        // `HeaderValue::from_str` takes a tab and every byte of 128 and above,
        // which a pasted key can carry and the API's keys never do; refusing
        // them here catches the slip at `check` rather than as a 401.
        let mut key = Some(key.0)
            .filter(|key| {
                key.bytes()
                    .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
            })
            .and_then(|key| HeaderValue::from_str(key).ok())
            .ok_or_else(|| TargetError::Key {
                reason: "the key holds a character a header can't carry".to_owned(),
            })?;
        key.set_sensitive(true);
        // No redirect is followed: the default policy keeps `x-api-key` on a
        // cross-host hop, so a gateway that redirected would be sent the key
        // elsewhere. Nothing is resent by the client: the loop counts one
        // request per attempt. No client-wide timeout: each request carries
        // the attempt's deadline.
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .retry(retry::never())
            .build()
            .map_err(|error| TargetError::Client {
                reason: error.without_url().to_string(),
            })?;
        Ok(Self {
            url,
            endpoint,
            key,
            authorization,
            model: ModelRef {
                api: ProviderApi::Messages,
                name: model.to_owned(),
                replays_reasoning: true,
            },
            cache,
            client,
        })
    }

    /// The model, as a run reports it.
    #[must_use]
    pub const fn model(&self) -> &ModelRef {
        &self.model
    }

    /// Where the API is served, for `server.address` and `server.port`.
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The URL each request goes to, with no user information in it.
    #[must_use]
    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// Whether `cache_control` markers are sent.
    #[must_use]
    pub const fn cache(&self) -> bool {
        self.cache
    }

    /// Whether the base URL held a user or a password, which each request
    /// carries as an `Authorization: Basic` header. A user and a password
    /// that are both empty are none, as the URL parser drops them.
    #[must_use]
    pub const fn carries_credentials(&self) -> bool {
        self.authorization.is_some()
    }

    /// Whether a URL whose authority is `host_and_port`, a host and an
    /// optional port with no user information, reaches the host and port
    /// each request goes to. The text is read as the base URL was, so a
    /// host's case, an international name and an address's spelling are
    /// compared as the parser writes them, and a caller that read the base
    /// URL another way can tell whether it read the same host.
    #[must_use]
    pub fn sends_to(&self, host_and_port: &str) -> bool {
        Url::parse(&format!("{}://{host_and_port}", self.url.scheme())).is_ok_and(|url| {
            url.host_str() == self.url.host_str()
                && url.port_or_known_default() == self.url.port_or_known_default()
        })
    }

    pub(crate) const fn messages_url(&self) -> &Url {
        &self.url
    }

    pub(crate) const fn key(&self) -> &HeaderValue {
        &self.key
    }

    pub(crate) const fn authorization(&self) -> Option<&HeaderValue> {
        self.authorization.as_ref()
    }

    pub(crate) const fn client(&self) -> &Client {
        &self.client
    }
}

fn base_url_refused() -> TargetError {
    TargetError::BaseUrl {
        reason: BASE_URL_SHAPE.to_owned(),
    }
}

/// `base_url` as a URL the adapter sends to, before its path is set.
fn parsed(base_url: &str) -> Result<Url, TargetError> {
    let url = Url::parse(base_url).map_err(|_| base_url_refused())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(base_url_refused());
    }
    Ok(url)
}

/// The `Authorization: Basic` header of `url`'s user information, when it
/// has any, which is then taken off the URL. The user and the password are
/// percent-decoded first, as the client would decode them itself, since a
/// password holding `@` or `:` can only be written escaped.
fn credentials(url: &mut Url) -> Result<Option<HeaderValue>, TargetError> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(None);
    }
    let user = decoded(url.username())?;
    let password = decoded(url.password().unwrap_or_default())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    let mut value =
        HeaderValue::from_str(&format!("Basic {encoded}")).map_err(|_| base_url_refused())?;
    value.set_sensitive(true);
    url.set_username("").map_err(|()| base_url_refused())?;
    url.set_password(None).map_err(|()| base_url_refused())?;
    Ok(Some(value))
}

fn decoded(text: &str) -> Result<String, TargetError> {
    percent_encoding::percent_decode_str(text)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|_| TargetError::BaseUrl {
            reason: "the base URL's user information isn't UTF-8 once its escapes are decoded"
                .to_owned(),
        })
}
