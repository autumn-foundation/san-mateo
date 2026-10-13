//! HTTP transport. Only this module does network I/O.

// The fake holds its lock for each whole call, so each call is atomic.
#![allow(clippy::significant_drop_tightening)]
//!
//! - [`ReqwestTransport`]: real HTTPS. It never follows redirects.
//! - [`MemoryTransport`]: a fake for tests. It records requests and replies
//!   with set responses.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

/// Header names that hold secrets. `Debug` output hides their values.
pub const SECRET_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
];

/// One HTTP request.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// Upper-case method, for example `GET`.
    pub method: String,
    /// Full URL.
    pub url: String,
    /// Header names in lower case, and values.
    pub headers: Vec<(String, String)>,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(k, v)| {
                let v = if is_secret_header(k) {
                    "<redacted>"
                } else {
                    v.as_str()
                };
                (k.as_str(), v)
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &redact_query(&self.url))
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Returns `true` for a header that holds a secret.
#[must_use]
pub fn is_secret_header(name: &str) -> bool {
    SECRET_HEADERS.iter().any(|h| h.eq_ignore_ascii_case(name))
}

/// Returns the URL with no query, for logs. A query can hold a key.
#[must_use]
pub fn redact_query(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?<redacted>"),
        None => url.to_owned(),
    }
}

impl HttpRequest {
    /// Makes a request with no headers and no body.
    #[must_use]
    pub fn new(method: &str, url: impl Into<String>) -> Self {
        Self {
            method: method.to_ascii_uppercase(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Adds a header. The name becomes lower case.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    /// Sets the body.
    #[must_use]
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    /// Returns the first value of a header. The name match ignores case.
    #[must_use]
    pub fn header_value(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }
}

/// One HTTP reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// HTTP status.
    pub status: u16,
    /// Header names in lower case, and values.
    pub headers: Vec<(String, String)>,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Makes a reply with a status and a body.
    #[must_use]
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// Makes a JSON reply.
    #[must_use]
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self::new(status, value.to_string()).header("content-type", "application/json")
    }

    /// Adds a header. The name becomes lower case.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    /// Returns the first value of a header. The name match ignores case.
    #[must_use]
    pub fn header_value(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }
}

/// Returns the first value of a header. The name match ignores case.
#[must_use]
pub fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// A transport failure with no HTTP reply.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The connection failed.
    #[error("connect failed: {0}")]
    Connect(String),
    /// The request took too long.
    #[error("timed out")]
    Timeout,
    /// The reply body is larger than the limit.
    #[error("reply body too large")]
    BodyTooLarge,
    /// The request is not valid, for example a bad URL.
    #[error("invalid request: {0}")]
    Invalid(String),
}

/// Sends HTTP requests.
#[async_trait]
pub trait HttpTransport: Send + Sync + 'static {
    /// Sends one request. A non-2xx status is a reply, not an error.
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// Settings for [`ReqwestTransport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportConfig {
    /// Total time for one request.
    pub timeout: Duration,
    /// Time to connect.
    pub connect_timeout: Duration,
    /// Largest reply body.
    pub max_body_bytes: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(10),
            max_body_bytes: 25 * 1024 * 1024,
        }
    }
}

/// Real HTTPS with `reqwest` and rustls. It never follows redirects.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    max_body_bytes: usize,
}

impl ReqwestTransport {
    /// Makes a transport.
    ///
    /// # Errors
    ///
    /// Fails when the TLS setup fails.
    pub fn new(config: TransportConfig) -> Result<Self, TransportError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .use_rustls_tls()
            .build()
            .map_err(|e| TransportError::Invalid(e.without_url().to_string()))?;
        Ok(Self {
            client,
            max_body_bytes: config.max_body_bytes,
        })
    }

    fn map_error(e: reqwest::Error) -> TransportError {
        if e.is_timeout() {
            TransportError::Timeout
        } else if e.is_builder() || e.is_request() && !e.is_connect() {
            TransportError::Invalid(e.without_url().to_string())
        } else {
            TransportError::Connect(e.without_url().to_string())
        }
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| TransportError::Invalid("bad HTTP method".to_owned()))?;
        let url = reqwest::Url::parse(&request.url)
            .map_err(|_| TransportError::Invalid("bad URL".to_owned()))?;
        let mut builder = self.client.request(method, url);
        for (k, v) in &request.headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|_| TransportError::Invalid("bad header name".to_owned()))?;
            let value = reqwest::header::HeaderValue::from_str(v)
                .map_err(|_| TransportError::Invalid("bad header value".to_owned()))?;
            builder = builder.header(name, value);
        }
        let mut response = builder
            .body(request.body)
            .send()
            .await
            .map_err(Self::map_error)?;
        if response
            .content_length()
            .is_some_and(|n| n > self.max_body_bytes as u64)
        {
            return Err(TransportError::BodyTooLarge);
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Self::map_error)? {
            if body.len() + chunk.len() > self.max_body_bytes {
                return Err(TransportError::BodyTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// The replies of one route. `used` is `true` when the last reply has been
/// given at least one time.
#[derive(Debug, Default)]
struct Script {
    items: VecDeque<Result<HttpResponse, TransportError>>,
    used: bool,
}

/// A fake transport for tests.
///
/// Set replies with [`MemoryTransport::reply`]. A route is a method and a URL
/// with no query. Replies for a route come in order. The last reply repeats
/// until a new reply is added. A request with no route gets a 404 reply.
#[derive(Debug, Clone, Default)]
pub struct MemoryTransport {
    routes: Arc<Mutex<HashMap<(String, String), Script>>>,
    log: Arc<Mutex<Vec<HttpRequest>>>,
}

impl MemoryTransport {
    /// Makes an empty fake.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a reply for `method` and `url`. The query of `url` is ignored.
    pub fn reply(&self, method: &str, url: &str, response: HttpResponse) {
        self.push(method, url, Ok(response));
    }

    fn push(&self, method: &str, url: &str, item: Result<HttpResponse, TransportError>) {
        let key = (method.to_ascii_uppercase(), strip_query(url).to_owned());
        let mut routes = lock(&self.routes);
        let script = routes.entry(key).or_default();
        if script.used {
            script.items.clear();
            script.used = false;
        }
        script.items.push_back(item);
    }

    /// Adds a failure for `method` and `url`.
    pub fn fail(&self, method: &str, url: &str, error: TransportError) {
        self.push(method, url, Err(error));
    }

    /// Returns all requests, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<HttpRequest> {
        lock(&self.log).clone()
    }

    /// Returns the requests to a URL with no query, in order.
    #[must_use]
    pub fn requests_to(&self, url: &str) -> Vec<HttpRequest> {
        let target = strip_query(url);
        lock(&self.log)
            .iter()
            .filter(|r| strip_query(&r.url) == target)
            .cloned()
            .collect()
    }
}

fn strip_query(url: &str) -> &str {
    url.split_once('?').map_or(url, |(base, _)| base)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[async_trait]
impl HttpTransport for MemoryTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let key = (request.method.clone(), strip_query(&request.url).to_owned());
        lock(&self.log).push(request);
        let mut routes = lock(&self.routes);
        let Some(script) = routes.get_mut(&key) else {
            return Ok(HttpResponse::new(404, "no route in MemoryTransport"));
        };
        let item = if script.items.len() > 1 {
            script.items.pop_front()
        } else {
            script.used = true;
            script.items.front().cloned()
        };
        item.unwrap_or_else(|| Ok(HttpResponse::new(404, "no route in MemoryTransport")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_builder_and_debug() {
        let r = HttpRequest::new("post", "https://a.example.com/x")
            .header("Authorization", "Bearer secret-1")
            .header("X-Api-Key", "secret-2")
            .header("Accept", "application/json")
            .body(b"secret-body".to_vec());
        assert_eq!(r.method, "POST");
        assert_eq!(r.header_value("authorization"), Some("Bearer secret-1"));
        assert_eq!(r.header_value("ACCEPT"), Some("application/json"));
        let d = format!("{r:?}");
        assert!(
            !d.contains("secret-1") && !d.contains("secret-2") && !d.contains("secret-body"),
            "{d}"
        );
        assert!(
            d.contains("application/json") && d.contains("body_len"),
            "{d}"
        );
    }

    #[test]
    fn response_helpers() {
        let r = HttpResponse::json(201, &serde_json::json!({"a": 1})).header("Retry-After", "5");
        assert_eq!(r.status, 201);
        assert_eq!(r.header_value("content-type"), Some("application/json"));
        assert_eq!(r.header_value("retry-after"), Some("5"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["a"],
            1
        );
    }

    #[tokio::test]
    async fn memory_routes_in_order_and_last_repeats() {
        let t = MemoryTransport::new();
        t.reply("GET", "https://a/x", HttpResponse::new(500, "e"));
        t.reply("GET", "https://a/x", HttpResponse::new(200, "ok"));
        let send = |u: &str| t.send(HttpRequest::new("GET", u));
        assert_eq!(send("https://a/x?p=1").await.unwrap().status, 500);
        assert_eq!(send("https://a/x").await.unwrap().status, 200);
        assert_eq!(send("https://a/x").await.unwrap().status, 200);
        assert_eq!(send("https://a/other").await.unwrap().status, 404);
        assert_eq!(
            t.send(HttpRequest::new("POST", "https://a/x"))
                .await
                .unwrap()
                .status,
            404
        );
        assert_eq!(t.requests().len(), 5);
        assert_eq!(t.requests_to("https://a/x").len(), 4, "all methods");
    }

    #[tokio::test]
    async fn memory_failures() {
        let t = MemoryTransport::new();
        t.fail("GET", "https://a/x", TransportError::Timeout);
        assert_eq!(
            t.send(HttpRequest::new("GET", "https://a/x")).await,
            Err(TransportError::Timeout)
        );
    }

    #[tokio::test]
    async fn reqwest_transport_reports_connect_errors() {
        let t = ReqwestTransport::new(TransportConfig {
            timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(1),
            max_body_bytes: 1024,
        })
        .unwrap();
        let err = t
            .send(HttpRequest::new("GET", "http://127.0.0.1:9/"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, TransportError::Connect(_) | TransportError::Timeout),
            "{err:?}"
        );
        let err = t
            .send(HttpRequest::new("GET", "not a url"))
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::Invalid(_)), "{err:?}");
        let err = t
            .send(HttpRequest::new("BAD METHOD", "http://127.0.0.1:9/"))
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::Invalid(_)), "{err:?}");
    }
}
