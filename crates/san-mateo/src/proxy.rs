//! Proxy helpers: upstream URL, SSRF checks, auth headers, header rules,
//! reply class and retry waits. The send loop is in [`crate::service`].
//! See ADR 0004.

use serde_json::Value;

use crate::credentials::Credentials;
use crate::error::{Error, Result};
use crate::policy::Outcome;
use crate::provider::{Provider, RetryHeaders};

/// Caller headers with these prefixes go upstream without the prefix.
pub const FORWARD_PREFIXES: &[&str] = &["san-mateo-proxy-", "nango-proxy-"];

/// Headers that never cross the proxy, in either direction.
pub const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

/// Proxy settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySettings {
    /// Extra hosts for `Base-Url-Override`. `*.example.com` allows sub-domains.
    pub allowed_hosts: Vec<String>,
    /// Allow private, loopback and link-local hosts. Only for tests and local
    /// development.
    pub allow_private_hosts: bool,
    /// Allow plain `http`. Only for tests and local development.
    pub allow_plain_http: bool,
    /// First backoff.
    pub initial_backoff_ms: u64,
    /// Largest wait before a retry.
    pub max_wait_ms: u64,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            allowed_hosts: Vec::new(),
            allow_private_hosts: false,
            allow_plain_http: false,
            initial_backoff_ms: 1_000,
            max_wait_ms: 60_000,
        }
    }
}

/// A call to send through the proxy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyRequest {
    /// Upper-case HTTP method.
    pub method: String,
    /// Path and query, for example `/v3/contacts?limit=10`.
    pub endpoint: String,
    /// Headers to send upstream.
    pub headers: Vec<(String, String)>,
    /// Body bytes.
    pub body: Vec<u8>,
    /// Retries after the first attempt.
    pub retries: u32,
    /// Status values to retry also for non-idempotent methods.
    pub retry_on: Vec<u16>,
    /// A base URL to use in place of `proxy.base_url`.
    pub base_url_override: Option<String>,
    /// Query parameters to add.
    pub params: Vec<(String, String)>,
}

impl ProxyRequest {
    /// Makes a request with no body.
    #[must_use]
    pub fn new(method: &str, endpoint: impl Into<String>) -> Self {
        Self {
            method: method.to_ascii_uppercase(),
            endpoint: endpoint.into(),
            ..Self::default()
        }
    }

    /// Makes a `GET` request.
    #[must_use]
    pub fn get(endpoint: impl Into<String>) -> Self {
        Self::new("GET", endpoint)
    }

    /// Adds a header. The name becomes lower case.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    /// Sets a JSON body and its content type.
    #[must_use]
    pub fn json(mut self, value: &Value) -> Self {
        self.body = value.to_string().into_bytes();
        self.header("content-type", "application/json")
    }

    /// Sets the number of retries.
    #[must_use]
    pub const fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Adds a query parameter.
    #[must_use]
    pub fn query(mut self, name: &str, value: impl Into<String>) -> Self {
        self.params.push((name.to_owned(), value.into()));
        self
    }
}

/// A reply from the proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyResponse {
    /// HTTP status.
    pub status: u16,
    /// Headers with no hop-by-hop headers.
    pub headers: Vec<(String, String)>,
    /// Body bytes.
    pub body: Vec<u8>,
}

impl ProxyResponse {
    /// Parses the body as JSON.
    ///
    /// # Errors
    ///
    /// [`Error::Upstream`] when the body is not valid JSON for `T`.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| Error::Upstream(format!("reply is not JSON: {e}")))
    }

    /// Returns `true` for a 2xx status.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// Returns the first value of a header.
    #[must_use]
    pub fn header_value(&self, name: &str) -> Option<&str> {
        crate::transport::header_value(&self.headers, name)
    }
}

/// Returns `true` for `GET`, `HEAD`, `OPTIONS`, `PUT` and `DELETE`.
#[must_use]
pub fn is_idempotent(method: &str) -> bool {
    let _ = method;
    todo!()
}

/// Joins a base URL and an endpoint. The endpoint can have a query. An
/// endpoint that starts with the base URL loses that prefix.
///
/// # Errors
///
/// [`Error::Invalid`] when the result is not a valid URL.
pub fn join_url(base: &str, endpoint: &str) -> Result<String> {
    let _ = (base, endpoint);
    todo!()
}

/// Adds query parameters to a URL.
///
/// # Errors
///
/// [`Error::Invalid`] when the URL is not valid.
pub fn add_query(url: &str, params: &[(String, String)]) -> Result<String> {
    let _ = (url, params);
    todo!()
}

/// Checks an upstream URL before a send.
///
/// - The scheme must be `https` (or `http` when `allow_plain_http`).
/// - A private, loopback, link-local or unspecified IP literal, or
///   `localhost`, is rejected unless `allow_private_hosts`.
/// - With `base_url_override`, the host must be `provider_host` or a host in
///   `allowed_hosts`.
///
/// # Errors
///
/// [`Error::ForbiddenHost`] or [`Error::Invalid`].
pub fn check_upstream(
    url: &str,
    provider_host: Option<&str>,
    base_url_override: bool,
    settings: &ProxySettings,
) -> Result<()> {
    let _ = (url, provider_host, base_url_override, settings);
    todo!()
}

/// Returns the host of a URL, in lower case.
#[must_use]
pub fn host_of(url: &str) -> Option<String> {
    let _ = url;
    todo!()
}

/// Returns the auth headers for the credentials.
///
/// OAuth2 modes add `Authorization: Bearer`. Basic adds `Authorization:
/// Basic`. API key and no-auth add nothing: the provider headers carry the
/// key. A provider `authorization` header replaces the default header.
#[must_use]
pub fn auth_headers(credentials: &Credentials, provider: &Provider) -> Vec<(String, String)> {
    let _ = (credentials, provider);
    todo!()
}

/// Renders the provider `proxy.headers` with `vars`.
///
/// # Errors
///
/// [`Error::Template`] when a value is missing.
pub fn provider_headers(provider: &Provider, vars: &Value) -> Result<Vec<(String, String)>> {
    let _ = (provider, vars);
    todo!()
}

/// Renders the provider `proxy.query` with `vars`.
///
/// # Errors
///
/// [`Error::Template`] when a value is missing.
pub fn provider_query(provider: &Provider, vars: &Value) -> Result<Vec<(String, String)>> {
    let _ = (provider, vars);
    todo!()
}

/// Returns the caller headers that go upstream: the headers with a
/// [`FORWARD_PREFIXES`] prefix, with no prefix and in lower case.
#[must_use]
pub fn forwarded_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let _ = headers;
    todo!()
}

/// Removes hop-by-hop headers.
#[must_use]
pub fn strip_hop_by_hop(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    let _ = headers;
    todo!()
}

/// Merges headers. A header in `over` replaces all headers with that name
/// in `base`. Names are lower case.
#[must_use]
pub fn merge_headers(base: Vec<(String, String)>, over: Vec<(String, String)>) -> Vec<(String, String)> {
    let _ = (base, over);
    todo!()
}

/// Classifies a reply. Returns the outcome and `true` when the caller or
/// the provider opts in to a retry for this status.
#[must_use]
pub fn classify(
    status: u16,
    headers: &[(String, String)],
    retry: Option<&RetryHeaders>,
    retry_on: &[u16],
) -> (Outcome, bool) {
    let _ = (status, headers, retry, retry_on);
    todo!()
}

/// Returns `true` when `status` matches a provider `error_code` entry:
/// `"429"`, `"5xx"` or `"500-502"`.
#[must_use]
pub fn error_code_matches(entry: &str, status: u16) -> bool {
    let _ = (entry, status);
    todo!()
}

/// Returns the wait that the reply headers ask for, in ms.
///
/// It reads `Retry-After` (seconds or an HTTP date), the provider `after`
/// headers (seconds) and the provider `at` headers (an instant). The longest
/// wait wins.
#[must_use]
pub fn header_wait_ms(headers: &[(String, String)], retry: Option<&RetryHeaders>, now_ms: u64) -> Option<u64> {
    let _ = (headers, retry, now_ms);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::OAuth2Credentials;
    use serde_json::json;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    #[test]
    fn idempotent_methods() {
        for m in ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "get"] {
            assert!(is_idempotent(m), "{m}");
        }
        for m in ["POST", "PATCH", "CONNECT"] {
            assert!(!is_idempotent(m), "{m}");
        }
    }

    #[test]
    fn url_joins() {
        assert_eq!(join_url("https://api.example.com", "/v1/x").unwrap(), "https://api.example.com/v1/x");
        assert_eq!(join_url("https://api.example.com/", "v1/x?a=1").unwrap(), "https://api.example.com/v1/x?a=1");
        assert_eq!(join_url("https://api.example.com/api/1.0", "/tasks").unwrap(), "https://api.example.com/api/1.0/tasks");
        assert_eq!(join_url("https://api.example.com/api", "https://api.example.com/api/tasks").unwrap(), "https://api.example.com/api/tasks");
        assert_eq!(join_url("https://api.example.com", "").unwrap(), "https://api.example.com/");
        assert!(join_url("not a url", "/x").is_err());
        assert_eq!(add_query("https://a.example.com/x?y=1", &h(&[("p", "a b"), ("q", "&")])).unwrap(), "https://a.example.com/x?y=1&p=a+b&q=%26");
        assert_eq!(add_query("https://a.example.com/x", &[]).unwrap(), "https://a.example.com/x");
    }

    #[test]
    fn ssrf_rules() {
        let s = ProxySettings::default();
        assert!(check_upstream("https://api.example.com/x", Some("api.example.com"), false, &s).is_ok());
        for bad in [
            "http://api.example.com/x",
            "https://127.0.0.1/x",
            "https://10.0.0.5/x",
            "https://192.168.1.1/x",
            "https://172.16.0.1/x",
            "https://169.254.169.254/latest/meta-data",
            "https://0.0.0.0/x",
            "https://[::1]/x",
            "https://[fe80::1]/x",
            "https://[fc00::1]/x",
            "https://[::ffff:127.0.0.1]/x",
            "https://localhost/x",
            "ftp://api.example.com/x",
        ] {
            assert!(check_upstream(bad, None, false, &s).is_err(), "{bad}");
        }
        let dev = ProxySettings { allow_private_hosts: true, allow_plain_http: true, ..ProxySettings::default() };
        assert!(check_upstream("http://127.0.0.1:8080/x", None, false, &dev).is_ok());
        assert!(check_upstream("not a url", None, false, &dev).is_err());
    }

    #[test]
    fn override_host_rules() {
        let s = ProxySettings { allowed_hosts: vec!["files.example.org".into(), "*.cdn.example.net".into()], ..ProxySettings::default() };
        assert!(check_upstream("https://api.example.com/x", Some("api.example.com"), true, &s).is_ok());
        assert!(check_upstream("https://files.example.org/x", Some("api.example.com"), true, &s).is_ok());
        assert!(check_upstream("https://eu.cdn.example.net/x", Some("api.example.com"), true, &s).is_ok());
        assert!(matches!(check_upstream("https://cdn.example.net/x", Some("api.example.com"), true, &s), Err(Error::ForbiddenHost(_))));
        assert!(matches!(check_upstream("https://evil.example/x", Some("api.example.com"), true, &s), Err(Error::ForbiddenHost(_))));
        assert!(matches!(check_upstream("https://evil.example/x", None, true, &s), Err(Error::ForbiddenHost(_))));
        assert_eq!(host_of("https://API.Example.com:8443/x").as_deref(), Some("api.example.com"));
        assert_eq!(host_of("nope"), None);
    }

    fn provider(headers: &[(&str, &str)]) -> Provider {
        let mut p = Provider::default();
        let mut proxy = crate::provider::ProxyConfig::default();
        for (k, v) in headers {
            proxy.headers.insert((*k).to_owned(), (*v).to_owned());
        }
        proxy.query.insert("key".into(), "${apiKey}".into());
        p.proxy = Some(proxy);
        p
    }

    #[test]
    fn auth_headers_by_mode() {
        let p = provider(&[]);
        let oauth = Credentials::OAuth2(OAuth2Credentials { access_token: "at".into(), refresh_token: None, expires_at: None, raw: Value::Null });
        assert_eq!(auth_headers(&oauth, &p), h(&[("authorization", "Bearer at")]));
        let basic = Credentials::Basic { username: "u".into(), password: "p".into() };
        assert_eq!(auth_headers(&basic, &p), h(&[("authorization", "Basic dTpw")]));
        assert!(auth_headers(&Credentials::ApiKey { api_key: "k".into() }, &p).is_empty());
        assert!(auth_headers(&Credentials::Unauthenticated, &p).is_empty());
        let custom = provider(&[("Authorization", "Token ${apiKey}")]);
        assert!(auth_headers(&oauth, &custom).is_empty());
    }

    #[test]
    fn provider_headers_and_query_render() {
        let p = provider(&[("authorization", "Bearer ${apiKey}"), ("x-version", "2024")]);
        let vars = json!({"apiKey": "k1"});
        assert_eq!(provider_headers(&p, &vars).unwrap(), h(&[("authorization", "Bearer k1"), ("x-version", "2024")]));
        assert_eq!(provider_query(&p, &vars).unwrap(), h(&[("key", "k1")]));
        assert!(provider_headers(&p, &json!({})).is_err());
        assert!(provider_headers(&Provider::default(), &vars).unwrap().is_empty());
    }

    #[test]
    fn header_rules() {
        let input = h(&[
            ("San-Mateo-Proxy-X-Trace", "1"),
            ("nango-proxy-accept", "application/json"),
            ("authorization", "Bearer caller-secret"),
            ("connection-id", "c1"),
        ]);
        assert_eq!(forwarded_headers(&input), h(&[("x-trace", "1"), ("accept", "application/json")]));
        let out = strip_hop_by_hop(h(&[("Connection", "close"), ("transfer-encoding", "chunked"), ("x-a", "1"), ("Host", "h")]));
        assert_eq!(out, h(&[("x-a", "1")]));
        let merged = merge_headers(h(&[("a", "1"), ("b", "2"), ("b", "3")]), h(&[("B", "9"), ("c", "4")]));
        assert_eq!(merged, h(&[("a", "1"), ("b", "9"), ("c", "4")]));
    }

    #[test]
    fn classes() {
        let none: &[(String, String)] = &[];
        assert_eq!(classify(200, none, None, &[]), (Outcome::Success, false));
        assert_eq!(classify(302, none, None, &[]), (Outcome::Success, false));
        assert_eq!(classify(429, none, None, &[]), (Outcome::RateLimited, false));
        assert_eq!(classify(503, none, None, &[]), (Outcome::ServerError, false));
        assert_eq!(classify(404, none, None, &[]), (Outcome::ClientError, false));
        assert_eq!(classify(500, none, None, &[500]), (Outcome::ServerError, true));
        let gh = RetryHeaders { remaining: Some("x-ratelimit-remaining".into()), error_code: vec!["403".into(), "5xx".into()], ..RetryHeaders::default() };
        assert_eq!(classify(403, &h(&[("x-ratelimit-remaining", "0")]), Some(&gh), &[]), (Outcome::RateLimited, true));
        assert_eq!(classify(403, &h(&[("x-ratelimit-remaining", "10")]), Some(&gh), &[]), (Outcome::ClientError, true));
        assert_eq!(classify(502, none, Some(&gh), &[]), (Outcome::ServerError, true));
        // A success with no calls left is still a success.
        assert_eq!(classify(200, &h(&[("x-ratelimit-remaining", "0")]), Some(&gh), &[]), (Outcome::Success, false));
    }

    #[test]
    fn error_code_entries() {
        assert!(error_code_matches("429", 429));
        assert!(!error_code_matches("429", 430));
        assert!(error_code_matches("5xx", 503));
        assert!(error_code_matches("4XX", 404));
        assert!(!error_code_matches("5xx", 404));
        assert!(error_code_matches("500-502", 501));
        assert!(!error_code_matches("500-502", 503));
        assert!(!error_code_matches("junk", 500));
    }

    #[test]
    fn waits_from_headers() {
        let none: &[(String, String)] = &[];
        assert_eq!(header_wait_ms(none, None, 0), None);
        assert_eq!(header_wait_ms(&h(&[("retry-after", "3")]), None, 0), Some(3_000));
        assert_eq!(header_wait_ms(&h(&[("retry-after", "Tue, 14 Nov 2023 22:13:25 GMT")]), None, 1_700_000_000_000), Some(5_000));
        let r = RetryHeaders { at: vec!["x-reset".into()], after: vec!["x-wait".into()], ..RetryHeaders::default() };
        assert_eq!(header_wait_ms(&h(&[("x-reset", "1700000010")]), Some(&r), 1_700_000_000_000), Some(10_000));
        assert_eq!(header_wait_ms(&h(&[("x-reset", "1700000010000")]), Some(&r), 1_700_000_000_000), Some(10_000));
        assert_eq!(header_wait_ms(&h(&[("x-reset", "1699999990")]), Some(&r), 1_700_000_000_000), Some(0));
        assert_eq!(header_wait_ms(&h(&[("x-wait", "2.5")]), Some(&r), 0), Some(2_500));
        // The longest wait wins.
        assert_eq!(header_wait_ms(&h(&[("x-wait", "2"), ("retry-after", "7")]), Some(&r), 0), Some(7_000));
        assert_eq!(header_wait_ms(&h(&[("x-wait", "soon")]), Some(&r), 0), None);
    }
}
