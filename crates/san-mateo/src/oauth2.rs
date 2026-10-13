//! OAuth2: authorize URL, PKCE (RFC 7636), token requests and token reply
//! parsing. This module does no I/O. See ADR 0002.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::provider::{BodyFormat, Provider, TokenAuthMethod};
use crate::secret::Secret;
use crate::transport::HttpRequest;

/// The `User-Agent` of token requests.
pub const USER_AGENT: &str = concat!("san-mateo/", env!("CARGO_PKG_VERSION"));

/// Returns the S256 code challenge: base64url(sha256(verifier)), no padding.
#[must_use]
pub fn pkce_challenge(verifier: &str) -> String {
    let _ = verifier;
    todo!()
}

/// The parts of an authorization request.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizeRequest<'a> {
    /// The rendered authorization URL. It can have a query.
    pub authorization_url: &'a str,
    /// The client id.
    pub client_id: &'a str,
    /// The callback URL.
    pub redirect_uri: &'a str,
    /// The scopes.
    pub scopes: &'a [String],
    /// The scope separator.
    pub scope_separator: &'a str,
    /// The `state` value.
    pub state: &'a str,
    /// The PKCE challenge, if PKCE is on.
    pub code_challenge: Option<&'a str>,
    /// Extra parameters. A parameter here replaces a standard one with the
    /// same name.
    pub params: &'a BTreeMap<String, String>,
}

/// Builds the authorization URL.
///
/// The query has `response_type=code`, `client_id`, `redirect_uri`, `scope`
/// (when there are scopes), `state`, the PKCE fields, then the extra
/// parameters. Values are percent-encoded (a space is `%20`).
///
/// # Errors
///
/// [`Error::Template`] when the authorization URL is not a valid URL.
pub fn authorize_url(req: &AuthorizeRequest<'_>) -> Result<String> {
    let _ = req;
    todo!()
}

/// Returns the parameters of a code exchange.
#[must_use]
pub fn code_exchange_params(
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    token_params: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let _ = (code, redirect_uri, code_verifier, token_params);
    todo!()
}

/// Returns the parameters of a refresh request.
///
/// `refresh_params` are used when the provider has them. Else the
/// `token_params` are used with no `grant_type`. The `grant_type` is
/// `refresh_token` unless `refresh_params` sets it.
#[must_use]
pub fn refresh_params(
    refresh_token: &str,
    refresh_params: &BTreeMap<String, String>,
    token_params: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let _ = (refresh_token, refresh_params, token_params);
    todo!()
}

/// Returns the parameters of a client credentials request.
#[must_use]
pub fn client_credentials_params(
    scopes: &[String],
    scope_separator: &str,
    token_params: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let _ = (scopes, scope_separator, token_params);
    todo!()
}

/// Builds a token endpoint request.
///
/// The body is a form or JSON (`body_format`). The client authenticates in
/// the body, or with Basic auth when `token_request_auth_method` is `basic`.
#[must_use]
pub fn token_request(
    provider: &Provider,
    url: &str,
    client_id: &str,
    client_secret: &str,
    params: Vec<(String, String)>,
) -> HttpRequest {
    let _ = (provider, url, client_id, client_secret, params);
    todo!()
}

/// A parsed token reply.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenReply {
    /// The access token.
    pub access_token: Secret,
    /// The refresh token, if any.
    pub refresh_token: Option<Secret>,
    /// The expiry instant, if any.
    pub expires_at: Option<DateTime<Utc>>,
    /// The reply as JSON.
    pub raw: Value,
}

/// Parses a token reply body.
///
/// - The body is JSON. A form body (`a=1&b=2`) also works.
/// - The token is `access_token` at the root. When it is missing, the token
///   comes from `alternate_path` (an object with `access_token`, or a string).
/// - The expiry comes from `expires_at` (see [`crate::clock::parse_instant`]),
///   else from `expires_in` (seconds, or ms when `expires_in_ms`).
///
/// # Errors
///
/// [`Error::Provider`] with `code` when the reply has an `error` field, does
/// not parse, or has no token. The message has the provider error code and
/// description only.
pub fn parse_token_reply(
    body: &[u8],
    alternate_path: Option<&str>,
    expires_in_ms: bool,
    now: DateTime<Utc>,
    code: &'static str,
) -> Result<TokenReply> {
    let _ = (body, alternate_path, expires_in_ms, now, code);
    todo!()
}

/// Copies the values at `paths` from the token reply. Only strings, numbers
/// and bools are copied. The key is the full dotted path.
#[must_use]
pub fn extract_metadata(raw: &Value, paths: &[String]) -> serde_json::Map<String, Value> {
    let _ = (raw, paths);
    todo!()
}

#[allow(dead_code)]
const fn uses_basic(provider: &Provider) -> bool {
    matches!(provider.token_request_auth_method, Some(TokenAuthMethod::Basic))
}

#[allow(dead_code)]
fn body_format(provider: &Provider) -> BodyFormat {
    provider.body_format.unwrap_or_default()
}

#[allow(dead_code)]
fn provider_error(code: &'static str, message: impl Into<String>) -> Error {
    Error::Provider {
        code,
        status: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-1mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_has_all_parts() {
        let scopes = vec!["read".to_owned(), "write all".to_owned()];
        let mut params = BTreeMap::new();
        params.insert("prompt".to_owned(), "consent".to_owned());
        params.insert("response_type".to_owned(), "code id_token".to_owned());
        let url = authorize_url(&AuthorizeRequest {
            authorization_url: "https://auth.example.com/authorize?tenant=t1",
            client_id: "cid",
            redirect_uri: "https://sm.example.com/oauth/callback",
            scopes: &scopes,
            scope_separator: ",",
            state: "st",
            code_challenge: Some("ch"),
            params: &params,
        })
        .unwrap();
        assert_eq!(
            url,
            "https://auth.example.com/authorize?tenant=t1&response_type=code%20id_token&client_id=cid&redirect_uri=https%3A%2F%2Fsm.example.com%2Foauth%2Fcallback&scope=read%2Cwrite%20all&state=st&code_challenge=ch&code_challenge_method=S256&prompt=consent"
        );
    }

    #[test]
    fn authorize_url_without_scopes_or_pkce() {
        let params = BTreeMap::new();
        let url = authorize_url(&AuthorizeRequest {
            authorization_url: "https://auth.example.com/authorize",
            client_id: "cid",
            redirect_uri: "https://cb",
            scopes: &[],
            scope_separator: " ",
            state: "st",
            code_challenge: None,
            params: &params,
        })
        .unwrap();
        assert_eq!(url, "https://auth.example.com/authorize?response_type=code&client_id=cid&redirect_uri=https%3A%2F%2Fcb&state=st");
        let bad = AuthorizeRequest {
            authorization_url: "not a url",
            client_id: "c",
            redirect_uri: "r",
            scopes: &[],
            scope_separator: " ",
            state: "s",
            code_challenge: None,
            params: &params,
        };
        assert!(matches!(authorize_url(&bad), Err(Error::Template(_))));
    }

    #[test]
    fn params_for_each_grant() {
        let mut tp = BTreeMap::new();
        tp.insert("grant_type".to_owned(), "authorization_code".to_owned());
        tp.insert("expires_in".to_owned(), "1800".to_owned());
        assert_eq!(
            code_exchange_params("c1", "https://cb", Some("v1"), &tp),
            vec![
                ("grant_type".to_owned(), "authorization_code".to_owned()),
                ("code".to_owned(), "c1".to_owned()),
                ("redirect_uri".to_owned(), "https://cb".to_owned()),
                ("code_verifier".to_owned(), "v1".to_owned()),
                ("expires_in".to_owned(), "1800".to_owned()),
            ]
        );
        assert_eq!(
            refresh_params("rt", &BTreeMap::new(), &tp),
            vec![
                ("grant_type".to_owned(), "refresh_token".to_owned()),
                ("refresh_token".to_owned(), "rt".to_owned()),
                ("expires_in".to_owned(), "1800".to_owned()),
            ]
        );
        let mut rp = BTreeMap::new();
        rp.insert("grant_type".to_owned(), "refresh_token".to_owned());
        rp.insert("audience".to_owned(), "x".to_owned());
        assert_eq!(
            refresh_params("rt", &rp, &tp),
            vec![
                ("grant_type".to_owned(), "refresh_token".to_owned()),
                ("refresh_token".to_owned(), "rt".to_owned()),
                ("audience".to_owned(), "x".to_owned()),
            ]
        );
        assert_eq!(
            client_credentials_params(&["a".to_owned(), "b".to_owned()], " ", &BTreeMap::new()),
            vec![("grant_type".to_owned(), "client_credentials".to_owned()), ("scope".to_owned(), "a b".to_owned())]
        );
        assert_eq!(
            client_credentials_params(&[], " ", &BTreeMap::new()),
            vec![("grant_type".to_owned(), "client_credentials".to_owned())]
        );
    }

    #[test]
    fn token_request_form_and_body_auth() {
        let p = Provider::default();
        let r = token_request(&p, "https://t/token", "id", "s&cret", vec![("code".to_owned(), "a b".to_owned())]);
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://t/token");
        assert_eq!(r.header_value("content-type"), Some("application/x-www-form-urlencoded"));
        assert_eq!(r.header_value("accept"), Some("application/json"));
        assert_eq!(r.header_value("user-agent"), Some(USER_AGENT));
        assert_eq!(r.header_value("authorization"), None);
        assert_eq!(String::from_utf8(r.body).unwrap(), "code=a+b&client_id=id&client_secret=s%26cret");
    }

    #[test]
    fn token_request_json_and_basic_auth() {
        let p = Provider {
            body_format: Some(BodyFormat::Json),
            token_request_auth_method: Some(TokenAuthMethod::Basic),
            ..Provider::default()
        };
        let r = token_request(&p, "https://t/token", "id", "sec:ret", vec![("code".to_owned(), "c".to_owned())]);
        assert_eq!(r.header_value("content-type"), Some("application/json"));
        // RFC 6749 2.3.1: form-encode the id and the secret, then base64.
        assert_eq!(r.header_value("authorization"), Some("Basic aWQ6c2VjJTNBcmV0"));
        assert_eq!(serde_json::from_slice::<Value>(&r.body).unwrap(), json!({"code": "c"}));
    }

    #[test]
    fn parse_standard_reply() {
        let body = br#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"token_type":"bearer"}"#;
        let r = parse_token_reply(body, None, false, now(), "token_exchange_failed").unwrap();
        assert_eq!(r.access_token.expose(), "at");
        assert_eq!(r.refresh_token.unwrap().expose(), "rt");
        assert_eq!(r.expires_at, Some(now() + chrono::Duration::seconds(3600)));
        assert_eq!(r.raw["token_type"], "bearer");
    }

    #[test]
    fn parse_quirky_replies() {
        // expires_in as a string, in ms.
        let r = parse_token_reply(br#"{"access_token":"at","expires_in":"5000"}"#, None, true, now(), "x").unwrap();
        assert_eq!(r.expires_at, Some(now() + chrono::Duration::milliseconds(5000)));
        // expires_at as epoch seconds and as an ISO string.
        let r = parse_token_reply(br#"{"access_token":"at","expires_at":1700003600}"#, None, false, now(), "x").unwrap();
        assert_eq!(r.expires_at, Some(Utc.timestamp_opt(1_700_003_600, 0).unwrap()));
        let r = parse_token_reply(br#"{"access_token":"at","expires_at":"2023-11-15T00:00:00Z"}"#, None, false, now(), "x").unwrap();
        assert_eq!(r.expires_at, Some(Utc.timestamp_opt(1_700_006_400, 0).unwrap()));
        // No expiry.
        let r = parse_token_reply(br#"{"access_token":"at"}"#, None, false, now(), "x").unwrap();
        assert_eq!(r.expires_at, None);
        // A form body (GitHub with no Accept header).
        let r = parse_token_reply(b"access_token=gho_1&scope=repo&token_type=bearer", None, false, now(), "x").unwrap();
        assert_eq!(r.access_token.expose(), "gho_1");
        assert_eq!(r.raw["scope"], "repo");
    }

    #[test]
    fn parse_alternate_path() {
        let body = br#"{"ok":true,"authed_user":{"access_token":"xoxp","refresh_token":"r2","expires_in":60}}"#;
        let r = parse_token_reply(body, Some("authed_user"), false, now(), "x").unwrap();
        assert_eq!(r.access_token.expose(), "xoxp");
        assert_eq!(r.refresh_token.unwrap().expose(), "r2");
        assert_eq!(r.expires_at, Some(now() + chrono::Duration::seconds(60)));
        // The root token wins when it exists.
        let body = br#"{"access_token":"xoxb","authed_user":{"access_token":"xoxp"}}"#;
        assert_eq!(parse_token_reply(body, Some("authed_user"), false, now(), "x").unwrap().access_token.expose(), "xoxb");
        let body = br#"{"data":{"token":"x"},"tok":"abc"}"#;
        assert_eq!(parse_token_reply(body, Some("tok"), false, now(), "x").unwrap().access_token.expose(), "abc");
    }

    #[test]
    fn parse_errors_have_no_secrets() {
        let e = parse_token_reply(br#"{"error":"invalid_grant","error_description":"bad code"}"#, None, false, now(), "token_exchange_failed").unwrap_err();
        match e {
            Error::Provider { code, message, .. } => {
                assert_eq!(code, "token_exchange_failed");
                assert!(message.contains("invalid_grant") && message.contains("bad code"));
            }
            other => panic!("{other:?}"),
        }
        let e = parse_token_reply(br#"{"refresh_token":"secret-rt"}"#, None, false, now(), "refresh_failed").unwrap_err();
        assert!(matches!(e, Error::Provider { code: "refresh_failed", .. }));
        assert!(!e.to_string().contains("secret-rt"));
        assert!(parse_token_reply(b"\xff\xfe", None, false, now(), "x").is_err());
        assert!(parse_token_reply(b"[1,2]", None, false, now(), "x").is_err());
    }

    #[test]
    fn metadata_copies_scalars_only() {
        let raw = json!({"instance_url": "https://na1.example.com", "team": {"id": "T1", "obj": {"a": 1}}, "n": 5, "b": true});
        let m = extract_metadata(&raw, &["instance_url".into(), "team.id".into(), "team.obj".into(), "n".into(), "b".into(), "missing".into()]);
        assert_eq!(m.len(), 4);
        assert_eq!(m["instance_url"], "https://na1.example.com");
        assert_eq!(m["team.id"], "T1");
        assert_eq!(m["n"], 5);
        assert_eq!(m["b"], true);
    }
}
