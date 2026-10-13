//! HTTP transport. Only this module does network I/O.
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
        let _ = f;
        todo!()
    }
}

impl HttpRequest {
    /// Makes a request with no headers and no body.
    #[must_use]
    pub fn new(method: &str, url: impl Into<String>) -> Self {
        let _ = (method, url);
        todo!()
    }

    /// Adds a header. The name becomes lower case.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        let _ = (&mut self, name, value);
        todo!()
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
        let _ = (status, value);
        todo!()
    }

    /// Adds a header. The name becomes lower case.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        let _ = (&mut self, name, value);
        todo!()
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
    let _ = (headers, name);
    todo!()
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
        let _ = config;
        todo!()
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let _ = (request, &self.client, self.max_body_bytes);
        todo!()
    }
}

type Script = VecDeque<Result<HttpResponse, TransportError>>;

/// A fake transport for tests.
///
/// Set replies with [`MemoryTransport::reply`]. A route is a method and a URL
/// with no query. Replies for a route come in order. The last reply repeats.
/// A request with no route gets a 404 reply.
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
        let _ = (method, url, response);
        todo!()
    }

    /// Adds a failure for `method` and `url`.
    pub fn fail(&self, method: &str, url: &str, error: TransportError) {
        let _ = (method, url, error);
        todo!()
    }

    /// Returns all requests, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<HttpRequest> {
        todo!()
    }

    /// Returns the requests to a URL with no query, in order.
    #[must_use]
    pub fn requests_to(&self, url: &str) -> Vec<HttpRequest> {
        let _ = url;
        todo!()
    }
}

#[async_trait]
impl HttpTransport for MemoryTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let _ = (request, &self.routes, &self.log);
        todo!()
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
        assert!(!d.contains("secret-1") && !d.contains("secret-2") && !d.contains("secret-body"), "{d}");
        assert!(d.contains("application/json") && d.contains("body_len"), "{d}");
    }

    #[test]
    fn response_helpers() {
        let r = HttpResponse::json(201, &serde_json::json!({"a": 1})).header("Retry-After", "5");
        assert_eq!(r.status, 201);
        assert_eq!(r.header_value("content-type"), Some("application/json"));
        assert_eq!(r.header_value("retry-after"), Some("5"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["a"], 1);
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
        assert_eq!(t.send(HttpRequest::new("POST", "https://a/x")).await.unwrap().status, 404);
        assert_eq!(t.requests().len(), 5);
        assert_eq!(t.requests_to("https://a/x").len(), 3);
    }

    #[tokio::test]
    async fn memory_failures() {
        let t = MemoryTransport::new();
        t.fail("GET", "https://a/x", TransportError::Timeout);
        assert_eq!(t.send(HttpRequest::new("GET", "https://a/x")).await, Err(TransportError::Timeout));
    }

    #[tokio::test]
    async fn reqwest_transport_reports_connect_errors() {
        let t = ReqwestTransport::new(TransportConfig {
            timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(1),
            max_body_bytes: 1024,
        })
        .unwrap();
        let err = t.send(HttpRequest::new("GET", "http://127.0.0.1:9/")).await.unwrap_err();
        assert!(matches!(err, TransportError::Connect(_) | TransportError::Timeout), "{err:?}");
        let err = t.send(HttpRequest::new("GET", "not a url")).await.unwrap_err();
        assert!(matches!(err, TransportError::Invalid(_)), "{err:?}");
        let err = t.send(HttpRequest::new("BAD METHOD", "http://127.0.0.1:9/")).await.unwrap_err();
        assert!(matches!(err, TransportError::Invalid(_)), "{err:?}");
    }
}
