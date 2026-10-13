//! Proxy helpers: upstream URL, SSRF checks, auth headers, header rules,
//! reply class and retry waits. The send loop is in [`crate::service`].
//! See ADR 0004.

use std::net::IpAddr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use crate::credentials::Credentials;
use crate::error::{Error, Result};
use crate::policy::Outcome;
use crate::provider::{Provider, RetryHeaders};
use crate::transport::header_value;

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
    /// Extra hosts for `Base-Url-Override` and for end-user host values.
    /// `*.example.com` allows sub-domains.
    pub allowed_hosts: Vec<String>,
    /// Allow internal hosts (see [`check_upstream`]). Only for tests and
    /// local development.
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
        serde_json::from_slice(&self.body)
            .map_err(|e| Error::Upstream(format!("reply is not JSON: {e}")))
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
    ["GET", "HEAD", "OPTIONS", "PUT", "DELETE"]
        .iter()
        .any(|m| m.eq_ignore_ascii_case(method))
}

/// Joins a base URL and an endpoint. The endpoint can have a query. An
/// endpoint that starts with the base URL loses that prefix.
///
/// # Errors
///
/// [`Error::Invalid`] when the result is not a valid URL.
pub fn join_url(base: &str, endpoint: &str) -> Result<String> {
    let rest = if !base.is_empty() && endpoint.starts_with(base) {
        &endpoint[base.len()..]
    } else {
        endpoint
    };
    let joined = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        rest.trim_start_matches('/')
    );
    url::Url::parse(&joined)
        .map_err(|_| Error::Invalid("the upstream URL is not valid".to_owned()))?;
    Ok(joined)
}

/// Adds query parameters to a URL.
///
/// # Errors
///
/// [`Error::Invalid`] when the URL is not valid.
pub fn add_query(url: &str, params: &[(String, String)]) -> Result<String> {
    url::Url::parse(url).map_err(|_| Error::Invalid("the upstream URL is not valid".to_owned()))?;
    if params.is_empty() {
        return Ok(url.to_owned());
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter())
        .finish();
    let sep = if url.contains('?') { "&" } else { "?" };
    Ok(format!("{url}{sep}{query}"))
}

/// Returns `true` for an IP that is not a public unicast address: private,
/// loopback, link-local, unspecified, shared (CGNAT), benchmark, multicast or
/// reserved. An IPv6 address with an IPv4 address in it (mapped, compatible,
/// NAT64 or 6to4) uses the rule of that IPv4 address.
fn is_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let embedded = |hi: u16, lo: u16| {
                let [h1, h2] = hi.to_be_bytes();
                let [l1, l2] = lo.to_be_bytes();
                is_internal_ip(IpAddr::V4(std::net::Ipv4Addr::new(h1, h2, l1, l2)))
            };
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_internal_ip(IpAddr::V4(mapped));
            }
            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }
            match s {
                // IPv4-compatible (::a.b.c.d), NAT64 (64:ff9b::/96) and 6to4
                // (2002::/16).
                [0, 0, 0, 0, 0, 0, hi, lo]
                | [0x64, 0xff9b, 0, 0, 0, 0, hi, lo]
                | [0x2002, hi, lo, ..] => embedded(hi, lo),
                // Local NAT64 (64:ff9b:1::/48).
                [0x64, 0xff9b, 1, ..] => true,
                [first, ..] => {
                    (first & 0xfe00) == 0xfc00
                        || (first & 0xffc0) == 0xfe80
                        || (first & 0xffc0) == 0xfec0
                        || (first & 0xff00) == 0xff00
                }
            }
        }
    }
}

pub(crate) fn host_allowed(host: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|entry| {
        let entry = entry.to_ascii_lowercase();
        entry.strip_prefix("*.").map_or_else(
            || host == entry,
            |suffix| host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
        )
    })
}

/// Checks an upstream URL before a send.
///
/// - The scheme must be `https` (or `http` when `allow_plain_http`).
/// - The host must not be internal, unless `allow_private_hosts`. Internal
///   hosts are `localhost`, `*.localhost`, and IP literals that are not
///   public unicast addresses (also inside IPv6 forms).
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
    let parsed = url::Url::parse(url)
        .map_err(|_| Error::Invalid("the upstream URL is not valid".to_owned()))?;
    match parsed.scheme() {
        "https" => {}
        "http" if settings.allow_plain_http => {}
        other => {
            return Err(Error::ForbiddenHost(format!(
                "the scheme {other} is not allowed"
            )));
        }
    }
    let host = parsed
        .host_str()
        .map(|h| {
            h.trim_start_matches('[')
                .trim_end_matches(']')
                .to_ascii_lowercase()
        })
        .ok_or_else(|| Error::Invalid("the upstream URL has no host".to_owned()))?;
    let internal = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => is_internal_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_internal_ip(IpAddr::V6(ip)),
        _ => {
            let name = host.trim_end_matches('.');
            name == "localhost" || name.ends_with(".localhost")
        }
    };
    if internal && !settings.allow_private_hosts {
        return Err(Error::ForbiddenHost(format!("the host {host} is internal")));
    }
    if base_url_override {
        let same = provider_host.is_some_and(|p| p.eq_ignore_ascii_case(&host));
        if !same && !host_allowed(&host, &settings.allowed_hosts) {
            return Err(Error::ForbiddenHost(format!(
                "the host {host} is not allowed"
            )));
        }
    }
    Ok(())
}

/// Returns the host of a URL, in lower case.
#[must_use]
pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

fn has_provider_auth_header(provider: &Provider) -> bool {
    provider.proxy.as_ref().is_some_and(|p| {
        p.headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("authorization"))
    })
}

/// Returns the auth headers for the credentials.
///
/// `OAuth2` modes add `Authorization: Bearer`. Basic adds `Authorization:
/// Basic`. API key and no-auth add nothing: the provider headers carry the
/// key. A provider `authorization` header replaces the default header.
#[must_use]
pub fn auth_headers(credentials: &Credentials, provider: &Provider) -> Vec<(String, String)> {
    if has_provider_auth_header(provider) {
        return Vec::new();
    }
    let value = match credentials {
        Credentials::OAuth2(_) | Credentials::OAuth2ClientCredentials(_) => {
            credentials.bearer_token().map(|t| format!("Bearer {t}"))
        }
        Credentials::Basic { username, password } => Some(format!(
            "Basic {}",
            STANDARD.encode(format!("{username}:{}", password.expose()))
        )),
        Credentials::ApiKey { .. } | Credentials::Unauthenticated => None,
    };
    value
        .map(|v| ("authorization".to_owned(), v))
        .into_iter()
        .collect()
}

/// Returns `true` when `template` reads a value that is not in `vars`, is
/// `null` or is empty.
///
/// # Errors
///
/// [`Error::Template`] when the syntax is wrong.
fn lacks_value(template: &str, vars: &Value) -> Result<bool> {
    Ok(crate::template::paths(template)?
        .iter()
        .any(|p| match crate::template::lookup(vars, p) {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.is_empty(),
            Some(_) => false,
        }))
}

/// Renders each template of `map`. As in Nango, it skips an entry with a
/// missing value. `lower` makes the names lower case.
fn render_map(
    map: &std::collections::BTreeMap<String, String>,
    vars: &Value,
    lower: bool,
) -> Result<Vec<(String, String)>> {
    let mut out = Vec::with_capacity(map.len());
    for (k, v) in map {
        match crate::template::render(v, vars) {
            Ok(value) if lower => out.push((k.to_ascii_lowercase(), value)),
            Ok(value) => out.push((k.clone(), value)),
            Err(e) => {
                if !lacks_value(v, vars)? {
                    return Err(e);
                }
            }
        }
    }
    Ok(out)
}

/// Renders the provider `proxy.headers` with `vars`. It skips a header with
/// a missing value.
///
/// # Errors
///
/// [`Error::Template`] when the syntax is wrong or a value is not text.
pub fn provider_headers(provider: &Provider, vars: &Value) -> Result<Vec<(String, String)>> {
    provider
        .proxy
        .as_ref()
        .map_or_else(|| Ok(Vec::new()), |p| render_map(&p.headers, vars, true))
}

/// Renders the provider `proxy.query` with `vars`. It skips a parameter with
/// a missing value. Names keep their case.
///
/// # Errors
///
/// [`Error::Template`] when the syntax is wrong or a value is not text.
pub fn provider_query(provider: &Provider, vars: &Value) -> Result<Vec<(String, String)>> {
    provider
        .proxy
        .as_ref()
        .map_or_else(|| Ok(Vec::new()), |p| render_map(&p.query, vars, false))
}

fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.iter().any(|h| h.eq_ignore_ascii_case(name))
}

/// Returns the caller headers that go upstream: the headers with a
/// [`FORWARD_PREFIXES`] prefix, with no prefix and in lower case.
#[must_use]
pub fn forwarded_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            let k = k.to_ascii_lowercase();
            let name = FORWARD_PREFIXES.iter().find_map(|p| k.strip_prefix(p))?;
            (!name.is_empty() && !is_hop_by_hop(name)).then(|| (name.to_owned(), v.clone()))
        })
        .collect()
}

/// Removes hop-by-hop headers. Names become lower case.
#[must_use]
pub fn strip_hop_by_hop(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter(|(k, _)| !is_hop_by_hop(k))
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

/// Merges headers. A header in `over` replaces all headers with that name
/// in `base`. Names are lower case.
#[must_use]
pub fn merge_headers(
    base: Vec<(String, String)>,
    over: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let over: Vec<(String, String)> = over
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect();
    let mut out: Vec<(String, String)> = base
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .filter(|(k, _)| !over.iter().any(|(o, _)| o == k))
        .collect();
    out.extend(over);
    out
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
    if status < 400 {
        return (Outcome::Success, false);
    }
    let opted_in = retry_on.contains(&status)
        || retry.is_some_and(|r| r.error_code.iter().any(|e| error_code_matches(e, status)));
    let none_left = retry
        .and_then(|r| r.remaining.as_deref())
        .and_then(|name| header_value(headers, name))
        .is_some_and(|v| v.trim() == "0");
    // The quota header counts only for 4xx: after a 5xx, the provider
    // possibly did the call.
    let outcome = if status == 429 || (none_left && status < 500) {
        Outcome::RateLimited
    } else if status >= 500 {
        Outcome::ServerError
    } else {
        Outcome::ClientError
    };
    (outcome, opted_in)
}

/// Returns `true` when `status` matches a provider `error_code` entry:
/// `"429"`, `"5xx"` or `"500-502"`.
#[must_use]
pub fn error_code_matches(entry: &str, status: u16) -> bool {
    let e = entry.trim().to_ascii_lowercase();
    if let Some(class) = e.strip_suffix("xx") {
        return class.len() == 1 && class.parse::<u16>().is_ok_and(|c| status / 100 == c);
    }
    if let Some((a, b)) = e.split_once('-') {
        return match (a.trim().parse::<u16>(), b.trim().parse::<u16>()) {
            (Ok(a), Ok(b)) => (a..=b).contains(&status),
            _ => false,
        };
    }
    e.parse::<u16>().is_ok_and(|c| c == status)
}

/// Seconds as text to ms. Negative or bad values give `None`.
fn secs_text_to_ms(v: &str) -> Option<u64> {
    let secs: f64 = v.trim().parse().ok()?;
    if !secs.is_finite() || secs < 0.0 || secs > 1.0e12 {
        return None;
    }
    // The range check keeps the value inside `u64`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((secs * 1000.0).round() as u64)
}

/// Returns the wait that the reply headers ask for, in ms.
///
/// It reads `Retry-After` (seconds or an HTTP date), the provider `after`
/// headers (seconds) and the provider `at` headers (an instant). The longest
/// wait wins.
#[must_use]
pub fn header_wait_ms(
    headers: &[(String, String)],
    retry: Option<&RetryHeaders>,
    now_ms: u64,
) -> Option<u64> {
    let until = |v: &str| {
        crate::clock::parse_instant_str(v)
            .map(|t| crate::policy::wait_until_ms(now_ms, crate::clock::to_ms(t)))
    };
    let mut waits = Vec::new();
    if let Some(v) = header_value(headers, "retry-after") {
        waits.extend(secs_text_to_ms(v).or_else(|| until(v)));
    }
    if let Some(r) = retry {
        for name in &r.after {
            waits.extend(header_value(headers, name).and_then(secs_text_to_ms));
        }
        for name in &r.at {
            waits.extend(header_value(headers, name).and_then(until));
        }
    }
    waits.into_iter().max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::OAuth2Credentials;
    use serde_json::json;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
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
        assert_eq!(
            join_url("https://api.example.com", "/v1/x").unwrap(),
            "https://api.example.com/v1/x"
        );
        assert_eq!(
            join_url("https://api.example.com/", "v1/x?a=1").unwrap(),
            "https://api.example.com/v1/x?a=1"
        );
        assert_eq!(
            join_url("https://api.example.com/api/1.0", "/tasks").unwrap(),
            "https://api.example.com/api/1.0/tasks"
        );
        assert_eq!(
            join_url(
                "https://api.example.com/api",
                "https://api.example.com/api/tasks"
            )
            .unwrap(),
            "https://api.example.com/api/tasks"
        );
        assert_eq!(
            join_url("https://api.example.com", "").unwrap(),
            "https://api.example.com/"
        );
        assert!(join_url("not a url", "/x").is_err());
        assert_eq!(
            add_query(
                "https://a.example.com/x?y=1",
                &h(&[("p", "a b"), ("q", "&")])
            )
            .unwrap(),
            "https://a.example.com/x?y=1&p=a+b&q=%26"
        );
        assert_eq!(
            add_query("https://a.example.com/x", &[]).unwrap(),
            "https://a.example.com/x"
        );
    }

    #[test]
    fn ssrf_rules() {
        let s = ProxySettings::default();
        assert!(
            check_upstream(
                "https://api.example.com/x",
                Some("api.example.com"),
                false,
                &s
            )
            .is_ok()
        );
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
            // Review findings: more internal forms.
            "https://localhost./x",
            "https://api.localhost./x",
            "https://0.1.2.3/x",
            "https://198.18.0.1/x",
            "https://224.0.0.1/x",
            "https://240.0.0.1/x",
            "https://[::7f00:1]/x",
            "https://[64:ff9b::a00:1]/x",
            "https://[2002:7f00:1::]/x",
            "https://[ff02::1]/x",
            "https://[fec0::1]/x",
            "https://100.64.0.1/x",
            "https://100.127.255.254/x",
            "https://198.19.255.1/x",
        ] {
            assert!(check_upstream(bad, None, false, &s).is_err(), "{bad}");
        }
        for internal in [
            "https://10.0.0.5/x",
            "https://[::1]/x",
            "https://localhost./x",
        ] {
            assert!(
                matches!(
                    check_upstream(internal, None, false, &s),
                    Err(Error::ForbiddenHost(_))
                ),
                "{internal}"
            );
        }
        // Public neighbours of the internal ranges stay open.
        for good in [
            "https://8.8.8.8/x",
            "https://[2001:4860:4860::8888]/x",
            "https://[64:ff9b::808:808]/x",
            "https://100.63.255.255/x",
            "https://100.128.0.1/x",
            "https://198.17.255.255/x",
            "https://198.20.0.1/x",
            "https://223.255.255.255/x",
            "https://1.0.0.1/x",
        ] {
            assert!(check_upstream(good, None, false, &s).is_ok(), "{good}");
        }
        let dev = ProxySettings {
            allow_private_hosts: true,
            allow_plain_http: true,
            ..ProxySettings::default()
        };
        assert!(check_upstream("http://127.0.0.1:8080/x", None, false, &dev).is_ok());
        assert!(check_upstream("not a url", None, false, &dev).is_err());
    }

    #[test]
    fn override_host_rules() {
        let s = ProxySettings {
            allowed_hosts: vec!["files.example.org".into(), "*.cdn.example.net".into()],
            ..ProxySettings::default()
        };
        assert!(
            check_upstream(
                "https://api.example.com/x",
                Some("api.example.com"),
                true,
                &s
            )
            .is_ok()
        );
        assert!(
            check_upstream(
                "https://files.example.org/x",
                Some("api.example.com"),
                true,
                &s
            )
            .is_ok()
        );
        assert!(
            check_upstream(
                "https://eu.cdn.example.net/x",
                Some("api.example.com"),
                true,
                &s
            )
            .is_ok()
        );
        assert!(matches!(
            check_upstream(
                "https://cdn.example.net/x",
                Some("api.example.com"),
                true,
                &s
            ),
            Err(Error::ForbiddenHost(_))
        ));
        assert!(matches!(
            check_upstream("https://evil.example/x", Some("api.example.com"), true, &s),
            Err(Error::ForbiddenHost(_))
        ));
        assert!(matches!(
            check_upstream("https://evil.example/x", None, true, &s),
            Err(Error::ForbiddenHost(_))
        ));
        assert_eq!(
            host_of("https://API.Example.com:8443/x").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(host_of("nope"), None);
        // A wildcard matches only hosts under its suffix, whatever the length.
        for foreign in [
            "https://attacker.example.org/x",
            "https://cdn.example.net.attacker.org/x",
            "https://xcdn.example.net/x",
        ] {
            assert!(
                matches!(
                    check_upstream(foreign, Some("api.example.com"), true, &s),
                    Err(Error::ForbiddenHost(_))
                ),
                "{foreign}"
            );
        }
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
        let oauth = Credentials::OAuth2(OAuth2Credentials {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
            raw: Value::Null,
        });
        assert_eq!(
            auth_headers(&oauth, &p),
            h(&[("authorization", "Bearer at")])
        );
        let basic = Credentials::Basic {
            username: "u".into(),
            password: "p".into(),
        };
        assert_eq!(
            auth_headers(&basic, &p),
            h(&[("authorization", "Basic dTpw")])
        );
        assert!(
            auth_headers(
                &Credentials::ApiKey {
                    api_key: "k".into()
                },
                &p
            )
            .is_empty()
        );
        assert!(auth_headers(&Credentials::Unauthenticated, &p).is_empty());
        let custom = provider(&[("Authorization", "Token ${apiKey}")]);
        assert!(auth_headers(&oauth, &custom).is_empty());
    }

    #[test]
    fn provider_headers_and_query_render() {
        let p = provider(&[("authorization", "Bearer ${apiKey}"), ("x-version", "2024")]);
        let vars = json!({"apiKey": "k1"});
        assert_eq!(
            provider_headers(&p, &vars).unwrap(),
            h(&[("authorization", "Bearer k1"), ("x-version", "2024")])
        );
        assert_eq!(provider_query(&p, &vars).unwrap(), h(&[("key", "k1")]));
        assert!(
            provider_headers(&Provider::default(), &vars)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn provider_query_names_keep_their_case() {
        let mut p = provider(&[("X-Version", "2024")]);
        if let Some(proxy) = p.proxy.as_mut() {
            proxy.query.insert("apiKey".into(), "${apiKey}".into());
        }
        let vars = json!({"apiKey": "k1"});
        assert_eq!(
            provider_query(&p, &vars).unwrap(),
            h(&[("apiKey", "k1"), ("key", "k1")])
        );
        assert_eq!(
            provider_headers(&p, &vars).unwrap(),
            h(&[("x-version", "2024")])
        );
    }

    #[test]
    fn provider_templates_skip_entries_with_missing_values() {
        let mut p = provider(&[
            (
                "x-login-customer-id",
                "${connectionConfig.login_customer_id}",
            ),
            ("x-either", "${connectionConfig.a} || ${connectionConfig.b}"),
            ("x-empty", "${connectionConfig.empty}"),
            ("x-version", "2024"),
        ]);
        if let Some(proxy) = p.proxy.as_mut() {
            proxy
                .query
                .insert("account".into(), "${connectionConfig.account}".into());
            proxy.query.insert("fixed".into(), "1".into());
        }
        let vars = json!({"connectionConfig": {"b": "B", "empty": ""}});
        assert_eq!(
            provider_headers(&p, &vars).unwrap(),
            h(&[("x-either", "B"), ("x-version", "2024")])
        );
        assert_eq!(provider_query(&p, &vars).unwrap(), h(&[("fixed", "1")]));
        // A syntax error or a value that is not text still fails.
        for bad in [
            "${connectionConfig.b",
            "${nope(${connectionConfig.b})}",
            "${connectionConfig}",
        ] {
            let p = provider(&[("x-bad", bad)]);
            assert!(provider_headers(&p, &vars).is_err(), "{bad}");
        }
    }

    #[test]
    fn header_rules() {
        let input = h(&[
            ("San-Mateo-Proxy-X-Trace", "1"),
            ("nango-proxy-accept", "application/json"),
            ("authorization", "Bearer caller-secret"),
            ("connection-id", "c1"),
        ]);
        assert_eq!(
            forwarded_headers(&input),
            h(&[("x-trace", "1"), ("accept", "application/json")])
        );
        let out = strip_hop_by_hop(h(&[
            ("Connection", "close"),
            ("transfer-encoding", "chunked"),
            ("x-a", "1"),
            ("Host", "h"),
        ]));
        assert_eq!(out, h(&[("x-a", "1")]));
        let merged = merge_headers(
            h(&[("a", "1"), ("b", "2"), ("b", "3")]),
            h(&[("B", "9"), ("c", "4")]),
        );
        assert_eq!(merged, h(&[("a", "1"), ("b", "9"), ("c", "4")]));
    }

    #[test]
    fn classes() {
        let none: &[(String, String)] = &[];
        assert_eq!(classify(200, none, None, &[]), (Outcome::Success, false));
        assert_eq!(classify(302, none, None, &[]), (Outcome::Success, false));
        assert_eq!(
            classify(429, none, None, &[]),
            (Outcome::RateLimited, false)
        );
        assert_eq!(
            classify(503, none, None, &[]),
            (Outcome::ServerError, false)
        );
        assert_eq!(
            classify(404, none, None, &[]),
            (Outcome::ClientError, false)
        );
        assert_eq!(
            classify(500, none, None, &[500]),
            (Outcome::ServerError, true)
        );
        // A quota header with no quota left does not make a 5xx a rate limit.
        let quota = RetryHeaders {
            remaining: Some("x-ratelimit-remaining".into()),
            ..RetryHeaders::default()
        };
        assert_eq!(
            classify(
                503,
                &h(&[("x-ratelimit-remaining", "0")]),
                Some(&quota),
                &[]
            ),
            (Outcome::ServerError, false)
        );
        let gh = RetryHeaders {
            remaining: Some("x-ratelimit-remaining".into()),
            error_code: vec!["403".into(), "5xx".into()],
            ..RetryHeaders::default()
        };
        assert_eq!(
            classify(403, &h(&[("x-ratelimit-remaining", "0")]), Some(&gh), &[]),
            (Outcome::RateLimited, true)
        );
        assert_eq!(
            classify(403, &h(&[("x-ratelimit-remaining", "10")]), Some(&gh), &[]),
            (Outcome::ClientError, true)
        );
        assert_eq!(
            classify(502, none, Some(&gh), &[]),
            (Outcome::ServerError, true)
        );
        // A success with no calls left is still a success.
        assert_eq!(
            classify(200, &h(&[("x-ratelimit-remaining", "0")]), Some(&gh), &[]),
            (Outcome::Success, false)
        );
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
        assert_eq!(
            header_wait_ms(&h(&[("retry-after", "3")]), None, 0),
            Some(3_000)
        );
        assert_eq!(
            header_wait_ms(
                &h(&[("retry-after", "Tue, 14 Nov 2023 22:13:25 GMT")]),
                None,
                1_700_000_000_000
            ),
            Some(5_000)
        );
        let r = RetryHeaders {
            at: vec!["x-reset".into()],
            after: vec!["x-wait".into()],
            ..RetryHeaders::default()
        };
        assert_eq!(
            header_wait_ms(
                &h(&[("x-reset", "1700000010")]),
                Some(&r),
                1_700_000_000_000
            ),
            Some(10_000)
        );
        assert_eq!(
            header_wait_ms(
                &h(&[("x-reset", "1700000010000")]),
                Some(&r),
                1_700_000_000_000
            ),
            Some(10_000)
        );
        assert_eq!(
            header_wait_ms(
                &h(&[("x-reset", "1699999990")]),
                Some(&r),
                1_700_000_000_000
            ),
            Some(0)
        );
        assert_eq!(
            header_wait_ms(&h(&[("x-wait", "2.5")]), Some(&r), 0),
            Some(2_500)
        );
        // The longest wait wins.
        assert_eq!(
            header_wait_ms(&h(&[("x-wait", "2"), ("retry-after", "7")]), Some(&r), 0),
            Some(7_000)
        );
        assert_eq!(header_wait_ms(&h(&[("x-wait", "soon")]), Some(&r), 0), None);
    }
}
