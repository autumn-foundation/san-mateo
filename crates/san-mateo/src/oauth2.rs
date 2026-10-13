//! `OAuth2`: authorize URL, PKCE (RFC 7636), token requests and token reply
//! parsing. This module does no I/O. See ADR 0002.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::clock::{from_ms, to_ms};
use crate::error::{Error, Result};
use crate::policy::expires_at_ms;
use crate::provider::{BodyFormat, Provider};
use crate::secret::Secret;
use crate::transport::HttpRequest;

/// The `User-Agent` of token requests.
pub const USER_AGENT: &str = concat!("san-mateo/", env!("CARGO_PKG_VERSION"));

/// Returns the S256 code challenge: base64url(sha256(verifier)), no padding.
#[must_use]
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
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

/// Sets `key` to `value`. An existing key keeps its position.
fn set(list: &mut Vec<(String, String)>, key: &str, value: &str) {
    match list.iter_mut().find(|(k, _)| k == key) {
        Some(item) => value.clone_into(&mut item.1),
        None => list.push((key.to_owned(), value.to_owned())),
    }
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
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
    url::Url::parse(req.authorization_url)
        .map_err(|_| Error::Template("the authorization URL is not a valid URL".to_owned()))?;
    let mut list = pairs(&[
        ("response_type", "code"),
        ("client_id", req.client_id),
        ("redirect_uri", req.redirect_uri),
    ]);
    if !req.scopes.is_empty() {
        list.push(("scope".to_owned(), req.scopes.join(req.scope_separator)));
    }
    list.push(("state".to_owned(), req.state.to_owned()));
    if let Some(challenge) = req.code_challenge {
        list.push(("code_challenge".to_owned(), challenge.to_owned()));
        list.push(("code_challenge_method".to_owned(), "S256".to_owned()));
    }
    for (k, v) in req.params {
        set(&mut list, k, v);
    }
    let query = list
        .iter()
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect::<Vec<_>>()
        .join("&");
    let base = req.authorization_url;
    let sep = if !base.contains('?') {
        "?"
    } else if base.ends_with('?') || base.ends_with('&') {
        ""
    } else {
        "&"
    };
    Ok(format!("{base}{sep}{query}"))
}

/// Percent-encodes like `encodeURIComponent`.
#[must_use]
pub fn encode_component(s: &str) -> String {
    crate::template::encode_component(s)
}

/// Returns the parameters of a code exchange.
#[must_use]
pub fn code_exchange_params(
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    token_params: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut list = pairs(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ]);
    if let Some(v) = code_verifier {
        list.push(("code_verifier".to_owned(), v.to_owned()));
    }
    for (k, v) in token_params {
        set(&mut list, k, v);
    }
    list
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
    let mut list = pairs(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ]);
    if refresh_params.is_empty() {
        for (k, v) in token_params
            .iter()
            .filter(|(k, _)| k.as_str() != "grant_type")
        {
            set(&mut list, k, v);
        }
    } else {
        for (k, v) in refresh_params {
            set(&mut list, k, v);
        }
    }
    list
}

/// Returns the parameters of a client credentials request.
#[must_use]
pub fn client_credentials_params(
    scopes: &[String],
    scope_separator: &str,
    token_params: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut list = pairs(&[("grant_type", "client_credentials")]);
    if !scopes.is_empty() {
        list.push(("scope".to_owned(), scopes.join(scope_separator)));
    }
    for (k, v) in token_params {
        set(&mut list, k, v);
    }
    list
}

/// Builds a token endpoint request.
///
/// The body is a form or JSON (`body_format`). The client authenticates in
/// the body, or with Basic auth (see [`Provider::uses_basic_client_auth`]).
#[must_use]
pub fn token_request(
    provider: &Provider,
    url: &str,
    client_id: &str,
    client_secret: &str,
    mut params: Vec<(String, String)>,
) -> HttpRequest {
    let mut req = HttpRequest::new("POST", url)
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT);
    if provider.uses_basic_client_auth() {
        let id: String = url::form_urlencoded::byte_serialize(client_id.as_bytes()).collect();
        let secret: String =
            url::form_urlencoded::byte_serialize(client_secret.as_bytes()).collect();
        req = req.header(
            "authorization",
            format!("Basic {}", STANDARD.encode(format!("{id}:{secret}"))),
        );
    } else {
        params.push(("client_id".to_owned(), client_id.to_owned()));
        if !client_secret.is_empty() {
            params.push(("client_secret".to_owned(), client_secret.to_owned()));
        }
    }
    match body_format(provider) {
        BodyFormat::Form => {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(params.iter())
                .finish();
            req.header("content-type", "application/x-www-form-urlencoded")
                .body(body.into_bytes())
        }
        BodyFormat::Json => {
            let obj: serde_json::Map<String, Value> = params
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            req.header("content-type", "application/json")
                .body(Value::Object(obj).to_string().into_bytes())
        }
    }
}

/// A parsed token reply.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Parses a reply body as a JSON object, or as a form.
fn parse_body(body: &[u8]) -> Option<serde_json::Map<String, Value>> {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        return v.as_object().cloned();
    }
    let text = std::str::from_utf8(body).ok()?;
    if !text.contains('=') {
        return None;
    }
    Some(
        url::form_urlencoded::parse(text.as_bytes())
            .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
            .collect(),
    )
}

fn non_empty_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Reads a non-negative number from a number or a numeric string. A
/// fraction becomes a whole number (`"3599.0"` is 3599).
fn as_count(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| whole(n.as_f64()?)),
        Value::String(s) => {
            let s = s.trim();
            s.parse().ok().or_else(|| whole(s.parse().ok()?))
        }
        _ => None,
    }
}

/// Cuts a finite, non-negative number to a whole number.
fn whole(f: f64) -> Option<u64> {
    // The check keeps the value inside `u64`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (f.is_finite() && (0.0..1.8e19).contains(&f)).then_some(f as u64)
}

/// Returns `true` for a zero number or a zero numeric string.
fn is_zero(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.trim().parse::<f64>().is_ok_and(|f| f == 0.0),
        _ => false,
    }
}

/// Returns `true` when the reply has an `error` that is a non-empty string or
/// an object. A `null`, `""` or `false` error is not an error.
fn has_error(obj: &serde_json::Map<String, Value>) -> bool {
    match obj.get("error") {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Object(_)) => true,
        _ => false,
    }
}

/// Returns the error text of a provider reply, if any. It has the provider
/// error code and description only.
#[must_use]
pub fn reply_error(body: &[u8]) -> Option<String> {
    let obj = parse_body(body)?;
    let error = match obj.get("error")? {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Object(o) => non_empty_str(o.get("code"))
            .or_else(|| non_empty_str(o.get("type")))?
            .to_owned(),
        _ => return None,
    };
    let description = non_empty_str(obj.get("error_description")).unwrap_or("");
    Some(if description.is_empty() {
        error
    } else {
        format!("{error}: {description}")
    })
}

/// Parses a token reply body.
///
/// - The body is JSON. A form body (`a=1&b=2`) also works.
/// - The token is `access_token` at the root. When it is missing, the token
///   comes from `alternate_path` (an object with `access_token`, or a string).
/// - The expiry comes from `expires_at` (see [`crate::clock::parse_instant`]),
///   else from `expires_in` (seconds, or ms when `expires_in_ms`). A zero
///   value means no expiry.
///
/// # Errors
///
/// [`Error::Provider`] with `code` when the reply has an `error` (a
/// non-empty string or an object), does not parse, or has no token. The
/// message has the provider error code and description only.
pub fn parse_token_reply(
    body: &[u8],
    alternate_path: Option<&str>,
    expires_in_ms: bool,
    now: DateTime<Utc>,
    code: &'static str,
) -> Result<TokenReply> {
    let raw = parse_body(body)
        .ok_or_else(|| provider_error(code, "the token reply is not a JSON object"))?;
    if has_error(&raw) {
        let msg = reply_error(body).unwrap_or_else(|| "the provider gave an error".to_owned());
        return Err(provider_error(code, msg));
    }
    let raw = Value::Object(raw);
    let (token, holder) = if let Some(t) = non_empty_str(raw.get("access_token")) {
        (t.to_owned(), &raw)
    } else {
        let alt = alternate_path
            .and_then(|p| crate::template::lookup(&raw, p))
            .ok_or_else(|| provider_error(code, "the token reply has no access token"))?;
        match alt {
            Value::String(s) if !s.is_empty() => (s.clone(), &raw),
            Value::Object(o) => {
                let t = non_empty_str(o.get("access_token"))
                    .ok_or_else(|| provider_error(code, "the token reply has no access token"))?;
                (t.to_owned(), alt)
            }
            _ => return Err(provider_error(code, "the token reply has no access token")),
        }
    };
    let refresh_token = non_empty_str(holder.get("refresh_token")).map(Secret::new);
    let expires_at = holder
        .get("expires_at")
        .filter(|v| !is_zero(v))
        .and_then(crate::clock::parse_instant)
        .or_else(|| {
            let n = as_count(holder.get("expires_in")?)?;
            (n > 0).then(|| from_ms(expires_at_ms(to_ms(now), n, expires_in_ms)))
        });
    Ok(TokenReply {
        access_token: Secret::new(token),
        refresh_token,
        expires_at,
        raw,
    })
}

/// Copies the values at `paths` from the token reply. Only strings, numbers
/// and bools are copied. The key is the full dotted path.
#[must_use]
pub fn extract_metadata(raw: &Value, paths: &[String]) -> serde_json::Map<String, Value> {
    paths
        .iter()
        .filter_map(|p| match crate::template::lookup(raw, p)? {
            v @ (Value::String(_) | Value::Number(_) | Value::Bool(_)) => {
                Some((p.clone(), v.clone()))
            }
            _ => None,
        })
        .collect()
}

fn body_format(provider: &Provider) -> BodyFormat {
    provider.body_format.unwrap_or_default()
}

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
    use crate::provider::TokenAuthMethod;
    use chrono::TimeZone;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
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
        assert_eq!(
            url,
            "https://auth.example.com/authorize?response_type=code&client_id=cid&redirect_uri=https%3A%2F%2Fcb&state=st"
        );
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
            vec![
                ("grant_type".to_owned(), "client_credentials".to_owned()),
                ("scope".to_owned(), "a b".to_owned())
            ]
        );
        assert_eq!(
            client_credentials_params(&[], " ", &BTreeMap::new()),
            vec![("grant_type".to_owned(), "client_credentials".to_owned())]
        );
    }

    #[test]
    fn token_request_form_and_body_auth() {
        let p = Provider::default();
        let r = token_request(
            &p,
            "https://t/token",
            "id",
            "s&cret",
            vec![("code".to_owned(), "a b".to_owned())],
        );
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://t/token");
        assert_eq!(
            r.header_value("content-type"),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(r.header_value("accept"), Some("application/json"));
        assert_eq!(r.header_value("user-agent"), Some(USER_AGENT));
        assert_eq!(r.header_value("authorization"), None);
        assert_eq!(
            String::from_utf8(r.body).unwrap(),
            "code=a+b&client_id=id&client_secret=s%26cret"
        );
    }

    #[test]
    fn token_request_json_and_basic_auth() {
        let p = Provider {
            body_format: Some(BodyFormat::Json),
            token_request_auth_method: Some(TokenAuthMethod::Basic),
            ..Provider::default()
        };
        let r = token_request(
            &p,
            "https://t/token",
            "id",
            "sec:ret",
            vec![("code".to_owned(), "c".to_owned())],
        );
        assert_eq!(r.header_value("content-type"), Some("application/json"));
        // RFC 6749 2.3.1: form-encode the id and the secret, then base64.
        assert_eq!(
            r.header_value("authorization"),
            Some("Basic aWQ6c2VjJTNBcmV0")
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&r.body).unwrap(),
            json!({"code": "c"})
        );
    }

    #[test]
    fn header_authorization_method_uses_basic_auth() {
        let c = crate::provider::Catalog::from_yaml_str(
            "h:\n    auth_mode: OAUTH2\n    authorization_method: header\nb:\n    auth_mode: OAUTH2\n    authorization_method: body\n",
        )
        .unwrap();
        let p = c.get("h").unwrap();
        let none = BTreeMap::new();
        for params in [
            code_exchange_params("c1", "https://cb", None, &none),
            refresh_params("rt", &none, &none),
            client_credentials_params(&[], " ", &none),
        ] {
            let r = token_request(&p, "https://t/token", "id", "sec", params);
            assert_eq!(r.header_value("authorization"), Some("Basic aWQ6c2Vj"));
            let body = String::from_utf8(r.body).unwrap();
            assert!(
                !body.contains("client_id") && !body.contains("client_secret"),
                "{body}"
            );
        }
        let b = c.get("b").unwrap();
        let r = token_request(&b, "https://t/token", "id", "sec", Vec::new());
        assert_eq!(r.header_value("authorization"), None);
        assert_eq!(
            String::from_utf8(r.body).unwrap(),
            "client_id=id&client_secret=sec"
        );
    }

    #[test]
    fn zero_expiry_means_no_expiry() {
        for body in [
            r#"{"access_token":"at","expires_in":0}"#,
            r#"{"access_token":"at","expires_in":"0"}"#,
            r#"{"access_token":"at","expires_at":0}"#,
            r#"{"access_token":"at","expires_at":"0"}"#,
            r#"{"access_token":"at","expires_at":0,"expires_in":0}"#,
        ] {
            let r = parse_token_reply(body.as_bytes(), None, false, now(), "x").unwrap();
            assert_eq!(r.expires_at, None, "{body}");
        }
        // A zero `expires_at` gives way to `expires_in`.
        let r = parse_token_reply(
            br#"{"access_token":"at","expires_at":0,"expires_in":60}"#,
            None,
            false,
            now(),
            "x",
        )
        .unwrap();
        assert_eq!(r.expires_at, Some(now() + chrono::Duration::seconds(60)));
    }

    #[test]
    fn decimal_expires_in_strings() {
        for (v, secs) in [
            (r#""3599.0""#, 3599),
            (r#""3599.9""#, 3599),
            (r#"" 60 ""#, 60),
            ("3599.5", 3599),
        ] {
            let body = format!(r#"{{"access_token":"at","expires_in":{v}}}"#);
            let r = parse_token_reply(body.as_bytes(), None, false, now(), "x").unwrap();
            assert_eq!(
                r.expires_at,
                Some(now() + chrono::Duration::seconds(secs)),
                "{v}"
            );
        }
        let r = parse_token_reply(
            br#"{"access_token":"at","expires_in":"-5"}"#,
            None,
            false,
            now(),
            "x",
        )
        .unwrap();
        assert_eq!(r.expires_at, None);
    }

    #[test]
    fn only_a_real_error_field_fails() {
        for body in [
            r#"{"access_token":"at","error":null}"#,
            r#"{"access_token":"at","error":""}"#,
            r#"{"access_token":"at","error":false}"#,
        ] {
            let r = parse_token_reply(body.as_bytes(), None, false, now(), "x").unwrap();
            assert_eq!(r.access_token.expose(), "at", "{body}");
        }
        for body in [
            r#"{"access_token":"at","error":"invalid_grant"}"#,
            r#"{"access_token":"at","error":{"code":"E1"}}"#,
            r#"{"access_token":"at","error":{}}"#,
        ] {
            assert!(
                parse_token_reply(body.as_bytes(), None, false, now(), "x").is_err(),
                "{body}"
            );
        }
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
        let r = parse_token_reply(
            br#"{"access_token":"at","expires_in":"5000"}"#,
            None,
            true,
            now(),
            "x",
        )
        .unwrap();
        assert_eq!(
            r.expires_at,
            Some(now() + chrono::Duration::milliseconds(5000))
        );
        // expires_at as epoch seconds and as an ISO string.
        let r = parse_token_reply(
            br#"{"access_token":"at","expires_at":1700003600}"#,
            None,
            false,
            now(),
            "x",
        )
        .unwrap();
        assert_eq!(
            r.expires_at,
            Some(Utc.timestamp_opt(1_700_003_600, 0).unwrap())
        );
        let r = parse_token_reply(
            br#"{"access_token":"at","expires_at":"2023-11-15T00:00:00Z"}"#,
            None,
            false,
            now(),
            "x",
        )
        .unwrap();
        assert_eq!(
            r.expires_at,
            Some(Utc.timestamp_opt(1_700_006_400, 0).unwrap())
        );
        // No expiry.
        let r = parse_token_reply(br#"{"access_token":"at"}"#, None, false, now(), "x").unwrap();
        assert_eq!(r.expires_at, None);
        // A form body (GitHub with no Accept header).
        let r = parse_token_reply(
            b"access_token=gho_1&scope=repo&token_type=bearer",
            None,
            false,
            now(),
            "x",
        )
        .unwrap();
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
        assert_eq!(
            parse_token_reply(body, Some("authed_user"), false, now(), "x")
                .unwrap()
                .access_token
                .expose(),
            "xoxb"
        );
        let body = br#"{"data":{"token":"x"},"tok":"abc"}"#;
        assert_eq!(
            parse_token_reply(body, Some("tok"), false, now(), "x")
                .unwrap()
                .access_token
                .expose(),
            "abc"
        );
    }

    #[test]
    fn parse_errors_have_no_secrets() {
        let e = parse_token_reply(
            br#"{"error":"invalid_grant","error_description":"bad code"}"#,
            None,
            false,
            now(),
            "token_exchange_failed",
        )
        .unwrap_err();
        match e {
            Error::Provider { code, message, .. } => {
                assert_eq!(code, "token_exchange_failed");
                assert!(message.contains("invalid_grant") && message.contains("bad code"));
            }
            other => panic!("{other:?}"),
        }
        let e = parse_token_reply(
            br#"{"refresh_token":"secret-rt"}"#,
            None,
            false,
            now(),
            "refresh_failed",
        )
        .unwrap_err();
        assert!(matches!(
            e,
            Error::Provider {
                code: "refresh_failed",
                ..
            }
        ));
        assert!(!e.to_string().contains("secret-rt"));
        assert!(parse_token_reply(b"\xff\xfe", None, false, now(), "x").is_err());
        assert!(parse_token_reply(b"[1,2]", None, false, now(), "x").is_err());
    }

    #[test]
    fn metadata_copies_scalars_only() {
        let raw = json!({"instance_url": "https://na1.example.com", "team": {"id": "T1", "obj": {"a": 1}}, "n": 5, "b": true});
        let m = extract_metadata(
            &raw,
            &[
                "instance_url".into(),
                "team.id".into(),
                "team.obj".into(),
                "n".into(),
                "b".into(),
                "missing".into(),
            ],
        );
        assert_eq!(m.len(), 4);
        assert_eq!(m["instance_url"], "https://na1.example.com");
        assert_eq!(m["team.id"], "T1");
        assert_eq!(m["n"], 5);
        assert_eq!(m["b"], true);
    }
}
