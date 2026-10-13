//! The `san-mateo:` producer: exchange to proxy call, and reply to exchange.

use std::collections::HashMap;
use std::sync::Arc;

use camel_api::{Body, CamelError, Exchange, Message, Value};
use san_mateo::{ProxyResponse, SanMateo};

use crate::options::EndpointOptions;

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
    pub(crate) async fn send(&self, exchange: Exchange) -> Result<Exchange, CamelError> {
        let _ = (&self.san, &self.options, exchange);
        todo!()
    }
}

/// Reads the connection id, the method and the path from the headers, else
/// from the endpoint options.
pub(crate) fn target(message: &Message, options: &EndpointOptions) -> Result<Target, CamelError> {
    let _ = (message, options);
    todo!()
}

/// Returns the `SanMateoProxy-<Name>` headers as `(<name>, value)` pairs,
/// sorted by name. Names are lower case.
pub(crate) fn forwarded_headers(
    headers: &HashMap<String, Value>,
) -> Result<Vec<(String, String)>, CamelError> {
    let _ = headers;
    todo!()
}

/// Returns the default content type for a body.
pub(crate) const fn default_content_type(body: &Body) -> Option<&'static str> {
    let _ = body;
    todo!()
}

/// Makes the camel body for a reply.
pub(crate) fn reply_body(content_type: Option<&str>, bytes: Vec<u8>) -> Body {
    let _ = (content_type, bytes);
    todo!()
}

/// Returns `true` for a reply header that must not reach the exchange.
pub(crate) fn is_secret_reply_header(name: &str) -> bool {
    let _ = name;
    todo!()
}

/// Puts a reply into the in message and clears the out message.
pub(crate) fn apply_reply(exchange: &mut Exchange, reply: ProxyResponse) {
    let _ = (exchange, reply);
    todo!()
}

/// Returns the URL for error messages: `san-mateo:<integration><path>`,
/// with no query.
pub(crate) fn error_url(integration: &str, path: &str) -> String {
    let _ = (integration, path);
    todo!()
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
