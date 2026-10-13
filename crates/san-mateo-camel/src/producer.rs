//! The `san-mateo:` producer: exchange to proxy call, and reply to exchange.

use std::collections::HashMap;
use std::sync::Arc;

use camel_api::{Body, CamelError, Exchange, Message, Value};
use san_mateo::transport::is_secret_header;
use san_mateo::{ConnectionKey, ProxyRequest, ProxyResponse, SanMateo};

use crate::error::{status_error, to_camel_error};
use crate::options::{EndpointOptions, METHODS, check_path, normalize_method};
use crate::{
    HEADER_CONNECTION_ID, HEADER_METHOD, HEADER_PATH, HEADER_PROXY_PREFIX, HEADER_RESPONSE_CODE,
    HEADER_RESPONSE_PREFIX, SCHEME,
};

/// Sends exchanges through the San Mateo proxy.
#[derive(Debug, Clone)]
pub(crate) struct Producer {
    san: SanMateo,
    options: Arc<EndpointOptions>,
}

/// The call target of one exchange, after the header overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) connection_id: String,
    pub(crate) method: String,
    pub(crate) path: String,
}

impl Producer {
    /// Makes a producer for one endpoint.
    pub(crate) const fn new(san: SanMateo, options: Arc<EndpointOptions>) -> Self {
        Self { san, options }
    }

    /// Sends one exchange and puts the reply into it.
    pub(crate) async fn send(&self, mut exchange: Exchange) -> Result<Exchange, CamelError> {
        let options = &self.options;
        let target = target(&exchange.input, options)?;
        let mut request = ProxyRequest::new(&target.method, target.path.clone());
        request.headers = forwarded_headers(&exchange.input.headers)?;
        let body = std::mem::take(&mut exchange.input.body);
        if let Some(content_type) = default_content_type(&body)
            && !request.headers.iter().any(|(k, _)| k == "content-type")
        {
            request = request.header("content-type", content_type);
        }
        // `into_bytes` enforces the limit, also for a stream body.
        request.body = body.into_bytes(options.max_body_size).await?.to_vec();
        request.retries = options.retries;
        request.retry_on.clone_from(&options.retry_on);
        request.base_url_override.clone_from(&options.base_url_override);

        let url = error_url(&options.integration, &target.path);
        let key = ConnectionKey::new(&options.integration, &target.connection_id);
        let reply = self
            .san
            .proxy(&options.environment, &key, request)
            .await
            .map_err(|e| to_camel_error(&e, &target.method, &url))?;
        tracing::debug!(
            integration = %options.integration,
            method = %target.method,
            status = reply.status,
            "san-mateo proxy call done"
        );
        if options.throw_exception_on_failure && !reply.is_success() {
            return Err(status_error(&target.method, &url, reply.status));
        }
        apply_reply(&mut exchange, reply);
        Ok(exchange)
    }
}

/// Returns a header value, matched first by exact name and then with no
/// regard to case. A null value counts as no value.
fn header<'a>(message: &'a Message, name: &str) -> Option<&'a Value> {
    message
        .header(name)
        .or_else(|| message.header_ic(name))
        .filter(|v| !v.is_null())
}

fn bad_header(name: &str, problem: &str) -> CamelError {
    CamelError::ValidationError(format!("the {name} header {problem}"))
}

/// Reads the connection id, the method and the path from the headers, else
/// from the endpoint options.
pub(crate) fn target(message: &Message, options: &EndpointOptions) -> Result<Target, CamelError> {
    let connection_id = match header(message, HEADER_CONNECTION_ID) {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(_) => return Err(bad_header(HEADER_CONNECTION_ID, "must be a non-empty string")),
        None => options.connection_id.clone().ok_or_else(|| {
            CamelError::ValidationError(format!(
                "no connection id: set the connectionId option or the {HEADER_CONNECTION_ID} header"
            ))
        })?,
    };
    let method = match header(message, HEADER_METHOD) {
        Some(Value::String(s)) => normalize_method(s)
            .ok_or_else(|| {
                bad_header(
                    HEADER_METHOD,
                    &format!("must be one of {}", METHODS.join(", ")),
                )
            })?
            .to_owned(),
        Some(_) => return Err(bad_header(HEADER_METHOD, "must be a string")),
        None => options.method.clone(),
    };
    let path = match header(message, HEADER_PATH) {
        Some(Value::String(s)) => {
            check_path(s).map_err(|m| bad_header(HEADER_PATH, &m))?;
            s.clone()
        }
        Some(_) => return Err(bad_header(HEADER_PATH, "must be a string")),
        None => options.path.clone().ok_or_else(|| {
            CamelError::ValidationError(format!(
                "no upstream path: set the path option or the {HEADER_PATH} header"
            ))
        })?,
    };
    Ok(Target {
        connection_id,
        method,
        path,
    })
}

/// Removes an ASCII prefix. The match ignores case.
fn strip_prefix_ic<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| value.get(prefix.len()..))
        .flatten()
}

/// Returns `true` for a valid HTTP header name (an RFC 9110 token).
fn is_token(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Returns the `SanMateoProxy-<Name>` headers as `(<name>, value)` pairs,
/// sorted by name. Names are lower case.
pub(crate) fn forwarded_headers(
    headers: &HashMap<String, Value>,
) -> Result<Vec<(String, String)>, CamelError> {
    let mut out = Vec::new();
    for (key, value) in headers {
        let Some(name) = strip_prefix_ic(key, HEADER_PROXY_PREFIX) else {
            continue;
        };
        if !is_token(name) {
            return Err(CamelError::ValidationError(format!(
                "a {HEADER_PROXY_PREFIX}<Name> header needs a valid HTTP header name, got '{key}'"
            )));
        }
        let text = match value {
            Value::Null => continue,
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Array(_) | Value::Object(_) => {
                return Err(bad_header(key, "must be a string, a number or a boolean"));
            }
        };
        // A control character could split the header. The message names
        // the header but never shows the value.
        if text.chars().any(|c| c.is_control() && c != '\t') {
            return Err(bad_header(key, "must not have control characters"));
        }
        out.push((name.to_ascii_lowercase(), text));
    }
    out.sort();
    Ok(out)
}

/// Returns the default content type for a body.
pub(crate) const fn default_content_type(body: &Body) -> Option<&'static str> {
    match body {
        Body::Json(_) => Some("application/json"),
        _ => None,
    }
}

/// Returns the media type in lower case, with no parameters.
fn media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Makes the camel body for a reply.
pub(crate) fn reply_body(content_type: Option<&str>, bytes: Vec<u8>) -> Body {
    if bytes.is_empty() {
        return Body::Empty;
    }
    let Some(media) = content_type.map(media_type) else {
        return Body::Bytes(bytes.into());
    };
    let json = media == "application/json" || media.ends_with("+json");
    if json && let Ok(value) = serde_json::from_slice(&bytes) {
        return Body::Json(value);
    }
    let text = json
        || media.starts_with("text/")
        || media == "application/xml"
        || media.ends_with("+xml");
    if !text {
        return Body::Bytes(bytes.into());
    }
    match String::from_utf8(bytes) {
        Ok(s) => Body::Text(s),
        Err(e) => Body::Bytes(e.into_bytes().into()),
    }
}

/// Parts of reply header names that hold secrets.
const SECRET_NAME_PARTS: &[&str] = &["cookie", "token", "secret", "password", "api-key", "apikey"];

/// Returns `true` for a reply header that must not reach the exchange.
pub(crate) fn is_secret_reply_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    is_secret_header(&name) || SECRET_NAME_PARTS.iter().any(|p| name.contains(p))
}

/// Puts a reply into the in message and clears the out message.
pub(crate) fn apply_reply(exchange: &mut Exchange, reply: ProxyResponse) {
    let message = &mut exchange.input;
    // Headers of an earlier call in the same route must not mix in.
    message
        .headers
        .retain(|k, _| strip_prefix_ic(k, HEADER_RESPONSE_PREFIX).is_none());
    let mut copied: Vec<(String, String)> = Vec::new();
    for (name, value) in &reply.headers {
        if is_secret_reply_header(name) {
            continue;
        }
        let name = name.to_ascii_lowercase();
        match copied.iter_mut().find(|(k, _)| *k == name) {
            Some((_, joined)) => {
                joined.push_str(", ");
                joined.push_str(value);
            }
            None => copied.push((name, value.clone())),
        }
    }
    for (name, value) in copied {
        message.set_header(format!("{HEADER_RESPONSE_PREFIX}{name}"), Value::String(value));
    }
    message.set_header(HEADER_RESPONSE_CODE, Value::from(reply.status));
    let content_type = reply.header_value("content-type").map(str::to_owned);
    message.body = reply_body(content_type.as_deref(), reply.body);
    exchange.output = None;
}

/// Returns the URL for error messages: `san-mateo:<integration><path>`,
/// with no query.
pub(crate) fn error_url(integration: &str, path: &str) -> String {
    let path = path.split_once('?').map_or(path, |(p, _)| p);
    format!("{SCHEME}:{integration}{path}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;
    use crate::{
        HEADER_CONNECTION_ID, HEADER_METHOD, HEADER_PATH, HEADER_RESPONSE_CODE,
        HEADER_RESPONSE_PREFIX,
    };

    fn options() -> EndpointOptions {
        EndpointOptions::parse(
            "san-mateo:crm?connectionId=c1&method=GET&path=/v3/contacts",
            Some("dev"),
            1024,
        )
        .unwrap()
    }

    fn validation(r: Result<impl std::fmt::Debug, CamelError>) -> String {
        match r {
            Err(CamelError::ValidationError(m)) => m,
            other => panic!("expected ValidationError, got {other:?}"),
        }
    }

    fn headers(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn target_from_options() {
        let t = target(&Message::default(), &options()).unwrap();
        assert_eq!(
            t,
            Target {
                connection_id: "c1".to_owned(),
                method: "GET".to_owned(),
                path: "/v3/contacts".to_owned(),
            }
        );
    }

    #[test]
    fn headers_replace_the_options() {
        let mut m = Message::default();
        m.set_header(HEADER_CONNECTION_ID, "c2");
        m.set_header(HEADER_METHOD, "patch");
        m.set_header(HEADER_PATH, "/v3/contacts/7?archived=true");
        let t = target(&m, &options()).unwrap();
        assert_eq!(t.connection_id, "c2");
        assert_eq!(t.method, "PATCH");
        assert_eq!(t.path, "/v3/contacts/7?archived=true");
        // The match ignores case. A number is a valid connection id.
        let mut m = Message::default();
        m.set_header("sanmateoconnectionid", json!(42));
        assert_eq!(target(&m, &options()).unwrap().connection_id, "42");
        // A null header counts as no header.
        let mut m = Message::default();
        m.set_header(HEADER_PATH, json!(null));
        assert_eq!(target(&m, &options()).unwrap().path, "/v3/contacts");
    }

    #[test]
    fn bad_target_headers_fail_validation() {
        let no_defaults =
            EndpointOptions::parse("san-mateo:crm", Some("dev"), 1024).unwrap();
        assert!(validation(target(&Message::default(), &no_defaults)).contains("connection"));
        let mut m = Message::default();
        m.set_header(HEADER_CONNECTION_ID, "c1");
        assert!(validation(target(&m, &no_defaults)).contains("path"));
        for (name, value) in [
            (HEADER_CONNECTION_ID, json!("")),
            (HEADER_CONNECTION_ID, json!({"a": 1})),
            (HEADER_METHOD, json!("TRACE")),
            (HEADER_METHOD, json!(1)),
            (HEADER_PATH, json!("no-slash")),
            (HEADER_PATH, json!("/a\r\nb")),
        ] {
            let mut m = Message::default();
            m.set_header(name, value.clone());
            let msg = validation(target(&m, &options()));
            assert!(msg.contains(name), "{name}={value}: {msg}");
        }
    }

    #[test]
    fn proxy_headers_go_upstream_without_the_prefix() {
        let h = headers(&[
            ("SanMateoProxy-X-Trace", json!("t-1")),
            ("sanmateoproxy-accept", json!("application/json")),
            ("SanMateoProxy-X-Count", json!(3)),
            ("SanMateoProxy-X-Flag", json!(true)),
            ("SanMateoProxy-X-Null", json!(null)),
            ("SanMateoPath", json!("/x")),
            ("Other", json!("o")),
        ]);
        assert_eq!(
            forwarded_headers(&h).unwrap(),
            vec![
                ("accept".to_owned(), "application/json".to_owned()),
                ("x-count".to_owned(), "3".to_owned()),
                ("x-flag".to_owned(), "true".to_owned()),
                ("x-trace".to_owned(), "t-1".to_owned()),
            ]
        );
        assert!(forwarded_headers(&HashMap::new()).unwrap().is_empty());
    }

    #[test]
    fn bad_proxy_headers_fail_validation() {
        for (name, value) in [
            ("SanMateoProxy-", json!("v")),
            ("SanMateoProxy-Bad Name", json!("v")),
            ("SanMateoProxy-Bad:Name", json!("v")),
            ("SanMateoProxy-X-Inject", json!("a\r\nX-Evil: 1")),
            ("SanMateoProxy-X-Zero", json!("a\u{0}")),
            ("SanMateoProxy-X-Object", json!({"a": 1})),
            ("SanMateoProxy-X-Array", json!([1])),
        ] {
            let msg = validation(forwarded_headers(&headers(&[(name, value)])));
            assert!(msg.contains("SanMateoProxy-"), "{name}: {msg}");
            assert!(!msg.contains("X-Evil"), "no header values in errors: {msg}");
        }
    }

    #[test]
    fn content_types() {
        assert_eq!(
            default_content_type(&Body::Json(json!({}))),
            Some("application/json")
        );
        assert_eq!(default_content_type(&Body::Text("t".into())), None);
        assert_eq!(default_content_type(&Body::Bytes(vec![1].into())), None);
        assert_eq!(default_content_type(&Body::Empty), None);
    }

    #[test]
    fn reply_bodies() {
        let json_body = br#"{"a":1}"#.to_vec();
        assert_eq!(
            reply_body(Some("application/json; charset=utf-8"), json_body.clone()),
            Body::Json(json!({"a": 1}))
        );
        assert_eq!(
            reply_body(Some("application/hal+json"), json_body.clone()),
            Body::Json(json!({"a": 1}))
        );
        assert_eq!(
            reply_body(Some("Application/JSON"), json_body.clone()),
            Body::Json(json!({"a": 1}))
        );
        assert_eq!(
            reply_body(Some("application/json"), b"not json".to_vec()),
            Body::Text("not json".to_owned()),
            "a bad JSON reply stays text"
        );
        assert_eq!(
            reply_body(Some("text/plain"), b"hi".to_vec()),
            Body::Text("hi".to_owned())
        );
        assert_eq!(
            reply_body(Some("application/xml"), b"<a/>".to_vec()),
            Body::Text("<a/>".to_owned())
        );
        assert_eq!(
            reply_body(Some("application/atom+xml"), b"<a/>".to_vec()),
            Body::Text("<a/>".to_owned())
        );
        assert_eq!(
            reply_body(Some("text/plain"), vec![0xff, 0xfe]),
            Body::Bytes(vec![0xff, 0xfe].into()),
            "bad UTF-8 stays bytes"
        );
        assert_eq!(
            reply_body(Some("application/octet-stream"), b"{}".to_vec()),
            Body::Bytes(b"{}".to_vec().into())
        );
        assert_eq!(
            reply_body(None, json_body.clone()),
            Body::Bytes(json_body.into())
        );
        assert_eq!(reply_body(Some("application/json"), Vec::new()), Body::Empty);
    }

    #[test]
    fn secret_reply_headers() {
        for name in [
            "set-cookie",
            "Set-Cookie",
            "set-cookie2",
            "cookie",
            "authorization",
            "proxy-authorization",
            "x-api-key",
            "x-auth-token",
            "x-csrf-token",
            "x-client-secret",
            "x-password",
        ] {
            assert!(is_secret_reply_header(name), "{name}");
        }
        for name in ["content-type", "x-request-id", "link", "retry-after", "etag"] {
            assert!(!is_secret_reply_header(name), "{name}");
        }
    }

    #[test]
    fn reply_goes_into_the_in_message() {
        let mut ex = Exchange::new(Message::new("request"));
        ex.input.set_header("SanMateoResponse-x-old", "stale");
        ex.input.set_header("SanMateoProxy-X-Trace", "t-1");
        ex.output = Some(Message::new("old out"));
        apply_reply(
            &mut ex,
            ProxyResponse {
                status: 201,
                headers: vec![
                    ("content-type".to_owned(), "application/json".to_owned()),
                    ("x-request-id".to_owned(), "r1".to_owned()),
                    ("link".to_owned(), "<a>; rel=next".to_owned()),
                    ("link".to_owned(), "<b>; rel=last".to_owned()),
                    ("set-cookie".to_owned(), "session=secret".to_owned()),
                    ("x-auth-token".to_owned(), "secret".to_owned()),
                ],
                body: br#"{"id":7}"#.to_vec(),
            },
        );
        assert!(ex.output.is_none(), "the out message is cleared");
        let m = &ex.input;
        assert_eq!(m.body, Body::Json(json!({"id": 7})));
        assert_eq!(m.header(HEADER_RESPONSE_CODE), Some(&json!(201)));
        assert_eq!(
            m.header("SanMateoResponse-x-request-id"),
            Some(&json!("r1"))
        );
        assert_eq!(
            m.header("SanMateoResponse-content-type"),
            Some(&json!("application/json"))
        );
        assert_eq!(
            m.header("SanMateoResponse-link"),
            Some(&json!("<a>; rel=next, <b>; rel=last"))
        );
        assert_eq!(m.header("SanMateoResponse-x-old"), None, "stale headers go");
        assert_eq!(m.header("SanMateoResponse-set-cookie"), None);
        assert_eq!(m.header("SanMateoResponse-x-auth-token"), None);
        assert_eq!(
            m.header("SanMateoProxy-X-Trace"),
            Some(&json!("t-1")),
            "request headers stay"
        );
        let secrets = m
            .headers
            .values()
            .filter(|v| v.as_str().is_some_and(|s| s.contains("secret")))
            .count();
        assert_eq!(secrets, 0);
        assert!(
            m.headers
                .keys()
                .filter(|k| k.starts_with(HEADER_RESPONSE_PREFIX))
                .count()
                == 3
        );
    }

    #[test]
    fn error_urls_have_no_query() {
        assert_eq!(
            error_url("crm", "/v3/contacts?api_key=secret"),
            "san-mateo:crm/v3/contacts"
        );
        assert_eq!(error_url("crm", "/"), "san-mateo:crm/");
    }
}
