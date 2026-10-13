//! HTTP transport: headers, timeouts, retries and response classification.

use std::fmt;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, RETRY_AFTER, USER_AGENT};
use serde_json::Value;

use crate::error::{ApiError, Error};
use crate::routes::{self, Method, Route};

/// Production API host.
pub const DEFAULT_BASE_URL: &str = "https://api.ingfah.ai";
/// Sent on every request; Cloudflare rejects requests without a User-Agent (error 1010).
pub const USER_AGENT_VALUE: &str = concat!("ingfah-cli/", env!("CARGO_PKG_VERSION"));

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ATTEMPTS: u32 = 3;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// A client API key. `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key, trimming surrounding whitespace as the server does.
    pub fn new(raw: &str) -> Self {
        Self(raw.trim().to_owned())
    }

    /// The raw key, for the header only.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True for `sk-ing-v1-` + 64 hex, or legacy `ing_sk_` + 22 of `[A-Za-z0-9_-]`.
    pub fn has_valid_format(&self) -> bool {
        if let Some(rest) = self.0.strip_prefix("sk-ing-v1-") {
            return rest.len() == 64 && rest.chars().all(|c| c.is_ascii_hexdigit());
        }
        if let Some(rest) = self.0.strip_prefix("ing_sk_") {
            return rest.len() == 22
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        }
        false
    }

    /// The key with everything but its prefix and last four characters hidden.
    pub fn masked(&self) -> String {
        let prefix = ["sk-ing-v1-", "ing_sk_"]
            .into_iter()
            .find(|p| self.0.starts_with(p))
            .unwrap_or("");
        let tail_start = self.0.len().saturating_sub(4).max(prefix.len());
        let tail = self.0.get(tail_start..).unwrap_or("");
        format!("{prefix}…{tail}")
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey({})", self.masked())
    }
}

/// A response body, classified by content type.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// `application/json`.
    Json(Value),
    /// `text/*`, CSV.
    Text(String),
    /// Anything else, and every body of a raw request.
    Bytes(Vec<u8>),
}

impl Body {
    /// The JSON value, if this is JSON.
    pub fn json(&self) -> Option<&Value> {
        match self {
            Self::Json(v) => Some(v),
            _ => None,
        }
    }
}

/// One request to send.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// HTTP method.
    pub method: Method,
    /// Concrete path, starting with `/`, without a query string.
    pub path: String,
    /// Query pairs; repeat a key to send several values.
    pub query: Vec<(String, String)>,
    /// JSON body.
    pub body: Option<Value>,
    /// Keep the body as bytes whatever its content type (for `--out`).
    pub raw: bool,
}

impl Request {
    /// A request with no query and no body.
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            query: Vec::new(),
            body: None,
            raw: false,
        }
    }

    /// A GET request.
    pub fn get(path: impl Into<String>) -> Self {
        Self::new(Method::Get, path)
    }

    /// A POST request.
    pub fn post(path: impl Into<String>) -> Self {
        Self::new(Method::Post, path)
    }

    /// Adds a query pair.
    #[must_use]
    pub fn query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((key.into(), value.into()));
        self
    }

    /// Adds a query pair when the value is present.
    #[must_use]
    pub fn query_opt(self, key: &str, value: Option<impl ToString>) -> Self {
        match value {
            Some(v) => self.query(key, v.to_string()),
            None => self,
        }
    }

    /// Sets the JSON body.
    #[must_use]
    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    /// Keeps the response body as bytes.
    #[must_use]
    pub fn raw(mut self) -> Self {
        self.raw = true;
        self
    }

    /// Resolves the request's route, refusing anything not in the registry.
    pub fn route(&self) -> Result<&'static Route, Error> {
        routes::resolve(self.method, &self.path).ok_or_else(|| Error::UnknownRoute {
            method: self.method,
            path: self.path.clone(),
        })
    }
}

/// A successful response.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// HTTP status.
    pub status: u16,
    /// The route the request resolved to.
    pub route: &'static Route,
    /// The body.
    pub body: Body,
}

/// How to reach the API.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Base URL; https, or http for loopback hosts.
    pub base_url: String,
    /// The API key.
    pub api_key: ApiKey,
    /// Timeout for ordinary requests; CSV, XLSX and audio downloads get 120 s.
    pub timeout: Duration,
    /// Backoff unit between retries; attempt `n` waits `n` units.
    pub retry_backoff: Duration,
}

impl ClientConfig {
    /// Defaults: 30 s timeout, 1 s backoff unit.
    pub fn new(base_url: impl Into<String>, api_key: ApiKey) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            timeout: DEFAULT_TIMEOUT,
            retry_backoff: Duration::from_secs(1),
        }
    }
}

/// The Ingfah client API, authenticated with an API key.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    config: ClientConfig,
}

/// Normalises a base URL: strips trailing `/`, requires https except for loopback hosts.
pub fn normalize_base_url(raw: &str) -> Result<String, Error> {
    let trimmed = raw.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(trimmed).map_err(|_| Error::BaseUrl(raw.to_owned()))?;
    let loopback = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    match url.scheme() {
        "https" => Ok(trimmed.to_owned()),
        "http" if loopback => Ok(trimmed.to_owned()),
        _ => Err(Error::BaseUrl(raw.to_owned())),
    }
}

impl Client {
    /// Builds a client. Fails on an unusable base URL.
    pub fn new(config: ClientConfig) -> Result<Self, Error> {
        let base_url = normalize_base_url(&config.base_url)?;
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            http,
            base_url,
            config,
        })
    }

    /// The normalised base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Sends a request through the registry, retrying GETs on transient failures.
    /// Error envelopes, including 2xx bodies with `"status":"error"`, become [`Error::Api`].
    pub async fn send(&self, req: &Request) -> Result<Response, Error> {
        let route = req.route()?;
        let timeout = if route.path.ends_with("/csv") || route.path.ends_with("/download") {
            DOWNLOAD_TIMEOUT
        } else {
            self.config.timeout
        };
        let mut attempt = 1;
        loop {
            let started = Instant::now();
            let result = self.send_once(req, timeout).await;
            let retry_after = match &result {
                Ok((status, headers, _)) if is_retryable(*status) => {
                    Some(parse_retry_after(headers))
                }
                Err(e) if e.is_timeout() || e.is_connect() => Some(None),
                _ => None,
            };
            if let Ok((status, _, _)) = &result {
                tracing::debug!(
                    "{} {} -> {} ({} ms)",
                    req.method,
                    req.path,
                    status,
                    started.elapsed().as_millis()
                );
            }
            match retry_after {
                Some(wait) if req.method.is_idempotent() && attempt < MAX_ATTEMPTS => {
                    let wait = wait.unwrap_or(self.config.retry_backoff * attempt);
                    tracing::info!(
                        "retrying {} {} in {} ms (attempt {} of {})",
                        req.method,
                        req.path,
                        wait.as_millis(),
                        attempt + 1,
                        MAX_ATTEMPTS
                    );
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                _ => {
                    let (status, headers, bytes) = result?;
                    return classify(status, &headers, bytes, req.raw, route);
                }
            }
        }
    }

    async fn send_once(
        &self,
        req: &Request,
        timeout: Duration,
    ) -> Result<(u16, HeaderMap, Vec<u8>), reqwest::Error> {
        let url = format!("{}{}", self.base_url, req.path);
        let mut builder = self
            .http
            .request(req.method.into(), &url)
            .timeout(timeout)
            .header("X-Api-Key", self.config.api_key.expose())
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, USER_AGENT_VALUE);
        if !req.query.is_empty() {
            builder = builder.query(&req.query);
        }
        if let Some(body) = &req.body {
            builder = builder.json(body);
        }
        tracing::trace!(
            "{} {} query={:?} headers: X-Api-Key={} Accept=application/json User-Agent={}",
            req.method,
            url,
            req.query,
            self.config.api_key.masked(),
            USER_AGENT_VALUE
        );
        let resp = builder.send().await?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let bytes = resp.bytes().await?.to_vec();
        Ok((status, headers, bytes))
    }

    /// GETs a path and returns the JSON envelope.
    pub async fn get_json(&self, req: Request) -> Result<Value, Error> {
        match self.send(&req).await?.body {
            Body::Json(v) => Ok(v),
            Body::Text(t) => Ok(Value::String(t)),
            Body::Bytes(_) => Err(Error::Refused(format!(
                "{} {} returned a binary body; use --out",
                req.method, req.path
            ))),
        }
    }
}

fn is_retryable(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|s| Duration::from_secs(s).min(MAX_RETRY_AFTER))
}

fn classify(
    status: u16,
    headers: &HeaderMap,
    bytes: Vec<u8>,
    raw: bool,
    route: &'static Route,
) -> Result<Response, Error> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_json = content_type.contains("json");
    if !(200..300).contains(&status) {
        return Err(Error::Api(Box::new(ApiError::from_response(
            status, &bytes, route,
        ))));
    }
    if is_json
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
        && value.get("status").and_then(Value::as_str) == Some("error")
    {
        return Err(Error::Api(Box::new(ApiError::from_response(
            status, &bytes, route,
        ))));
    }
    let body = if raw {
        Body::Bytes(bytes)
    } else if is_json {
        match serde_json::from_slice(&bytes) {
            Ok(v) => Body::Json(v),
            Err(_) => Body::Text(String::from_utf8_lossy(&bytes).into_owned()),
        }
    } else if content_type.starts_with("text/") || content_type.contains("csv") {
        match String::from_utf8(bytes) {
            Ok(t) => Body::Text(t),
            Err(e) => Body::Bytes(e.into_bytes()),
        }
    } else {
        Body::Bytes(bytes)
    };
    Ok(Response {
        status,
        route,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_key_format_is_valid() {
        let key = ApiKey::new(&format!("sk-ing-v1-{}", "a1".repeat(32)));
        assert!(key.has_valid_format());
    }

    #[test]
    fn legacy_key_format_is_valid() {
        assert!(ApiKey::new("ing_sk_abcdefghij-_KLMNOPQRST").has_valid_format());
    }

    #[test]
    fn backoffice_key_format_is_invalid() {
        let key = ApiKey::new(&format!("sk-ing-bo-v1-{}", "a".repeat(64)));
        assert!(!key.has_valid_format());
    }

    #[test]
    fn masked_key_shows_prefix_and_last_four() {
        let key = ApiKey::new(&format!("sk-ing-v1-{}abcd", "0".repeat(60)));
        assert_eq!(key.masked(), "sk-ing-v1-…abcd");
    }

    #[test]
    fn debug_never_prints_the_key() {
        let raw = format!("sk-ing-v1-{}", "f".repeat(64));
        assert!(!format!("{:?}", ApiKey::new(&raw)).contains(&raw));
    }

    #[test]
    fn base_url_strips_trailing_slash() {
        assert_eq!(
            normalize_base_url("https://api.ingfah.ai/").ok().as_deref(),
            Some("https://api.ingfah.ai")
        );
    }

    #[test]
    fn base_url_refuses_plain_http_for_remote_hosts() {
        assert!(normalize_base_url("http://api.ingfah.ai").is_err());
    }

    #[test]
    fn base_url_allows_http_for_loopback() {
        assert!(normalize_base_url("http://127.0.0.1:8080").is_ok());
    }
}
