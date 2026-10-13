//! HTML pages: the hosted connect page and the OAuth result page.
//!
//! Each page escapes all text. A Content-Security-Policy with a nonce allows
//! only the inline script of the page.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use axum::extract::{RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::{query_get, query_pairs};
use crate::error::Error;
use crate::provider::{AuthMode, Provider};
use crate::service::SanMateo;

/// Escapes text for HTML content and attribute values.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Makes JSON safe inside a `<script>` element.
fn script_json(v: &serde_json::Value) -> String {
    v.to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

const STYLE: &str = "body{font-family:system-ui,sans-serif;max-width:40rem;margin:2rem auto;padding:0 1rem;color:#222}\
section{border:1px solid #ddd;border-radius:8px;padding:1rem;margin:1rem 0}\
label{display:block;margin:.5rem 0}input{width:100%;padding:.4rem;box-sizing:border-box}\
button,a.button{display:inline-block;margin-top:.5rem;padding:.5rem 1rem;border-radius:6px;border:0;background:#222;color:#fff;text-decoration:none;cursor:pointer}\
.error{color:#a00}.ok{color:#070}";

fn page(san: &SanMateo, status: StatusCode, title: &str, body: &str, script: &str) -> Response {
    let nonce = san.random_token(16);
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style nonce=\"{nonce}\">{STYLE}</style></head><body>{body}<script nonce=\"{nonce}\">{script}</script></body></html>",
        escape(title)
    );
    let csp = format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; connect-src 'self' {server}; form-action 'self' {server}; base-uri 'none'; frame-ancestors 'none'",
        server = san.config().server_url
    );
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned()),
            (header::CONTENT_SECURITY_POLICY, csp),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (header::REFERRER_POLICY, "no-referrer".to_owned()),
        ],
        html,
    )
        .into_response()
}

fn error_page(san: &SanMateo, e: &Error) -> Response {
    let status = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST);
    let body = format!(
        "<h1>The connection did not complete</h1><p class=\"error\">{}</p><p><code>{}</code></p>",
        escape(&e.public_message()),
        escape(e.code())
    );
    let msg = json!({"type": "san-mateo:auth", "eventType": "AUTHORIZATION_FAILED", "error": {"type": e.code()}});
    let script = format!(
        "if (window.opener) {{ window.opener.postMessage({}, '*'); }}",
        script_json(&msg)
    );
    page(san, status, "Connection failed", &body, &script)
}

/// `GET /oauth/callback`: ends the OAuth flow and shows the result.
pub(crate) async fn oauth_callback(State(san): State<SanMateo>, RawQuery(q): RawQuery) -> Response {
    let query: BTreeMap<String, String> = query_pairs(q.as_deref()).into_iter().collect();
    match san.oauth_callback(&query).await {
        Ok(c) => {
            let msg = json!({
                "type": "san-mateo:auth",
                "eventType": "AUTHORIZATION_SUCEEDED",
                "connectionId": c.connection_id,
                "providerConfigKey": c.provider_config_key,
            });
            let script = format!(
                "if (window.opener) {{ window.opener.postMessage({}, '*'); window.close(); }}",
                script_json(&msg)
            );
            let body = format!(
                "<h1>Connected</h1><p class=\"ok\">The connection to {} is ready. You can close this window.</p>",
                escape(&c.provider)
            );
            page(&san, StatusCode::OK, "Connected", &body, &script)
        }
        Err(e) => error_page(&san, &e),
    }
}

fn field_inputs(provider: &Provider, names: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, label) in names {
        let _ = write!(
            out,
            "<label>{}<input name=\"{}\" type=\"{}\" required></label>",
            escape(label),
            escape(name),
            if name.contains("password") || name.contains("secret") || name.contains("apiKey") {
                "password"
            } else {
                "text"
            }
        );
    }
    for (name, schema) in &provider.connection_config {
        if schema.automated.unwrap_or(false) {
            continue;
        }
        let label = schema.title.clone().unwrap_or_else(|| name.clone());
        let _ = write!(
            out,
            "<label>{}<input name=\"params[{}]\" type=\"text\"{}{}></label>",
            escape(&label),
            escape(name),
            schema
                .example
                .as_deref()
                .map(|e| format!(" placeholder=\"{}\"", escape(e)))
                .unwrap_or_default(),
            if schema.optional.unwrap_or(false) {
                ""
            } else {
                " required"
            }
        );
    }
    out
}

fn integration_section(san: &SanMateo, token: &str, key: &str, provider: &Provider) -> String {
    let server = san.config().server_url.trim_end_matches('/');
    let title = format!("<h2>{}</h2>", escape(&provider.display_name));
    let token_q = format!(
        "connect_session_token={}",
        crate::oauth2::encode_component(token)
    );
    let (path, fields): (&str, Vec<(&str, &str)>) = match provider.auth_mode {
        AuthMode::OAuth2 => {
            if provider
                .connection_config
                .values()
                .all(|s| s.automated.unwrap_or(false))
            {
                return format!(
                    "<section>{title}<a class=\"button\" href=\"{server}/oauth/connect/{}?{token_q}\">Connect</a></section>",
                    escape(key)
                );
            }
            // The script opens the URL as a link. A form submit that ends in
            // a redirect to the provider breaks the CSP `form-action`.
            return format!(
                "<section>{title}<form class=\"sm-oauth\" method=\"get\" action=\"{server}/oauth/connect/{key}\" data-action=\"{server}/oauth/connect/{key}?{token_q}\"><input type=\"hidden\" name=\"connect_session_token\" value=\"{}\">{}<button type=\"submit\">Connect</button></form></section>",
                escape(token),
                field_inputs(provider, &[]),
                key = escape(key),
            );
        }
        AuthMode::ApiKey => ("api-auth/api-key", vec![("apiKey", "API key")]),
        AuthMode::Basic => (
            "api-auth/basic",
            vec![("username", "User name"), ("password", "Password")],
        ),
        AuthMode::OAuth2ClientCredentials => (
            "oauth2/auth",
            vec![
                ("client_id", "Client id"),
                ("client_secret", "Client secret"),
            ],
        ),
        AuthMode::None => ("auth/unauthenticated", vec![]),
        AuthMode::Other(_) => return String::new(),
    };
    // `method="post"`: with no script, a submit never puts a secret in a URL.
    format!(
        "<section>{title}<form class=\"sm-json\" method=\"post\" action=\"{action}\" data-action=\"{action}\">{}<button type=\"submit\">Connect</button><p class=\"result\"></p></form></section>",
        field_inputs(provider, &fields),
        action = format_args!("{server}/{path}/{}?{token_q}", escape(key)),
    )
}

/// Opens each `.sm-oauth` form as a link. Posts each `.sm-json` form as JSON
/// and shows the result.
const FORM_SCRIPT: &str = "document.querySelectorAll('form.sm-oauth').forEach(function (f) {\
  f.addEventListener('submit', function (e) {\
    e.preventDefault();\
    var q = new URLSearchParams();\
    new FormData(f).forEach(function (v, k) { if (k !== 'connect_session_token') { q.append(k, v); } });\
    var s = q.toString();\
    window.location.assign(f.dataset.action + (s ? '&' + s : ''));\
  });\
});\
document.querySelectorAll('form.sm-json').forEach(function (f) {\
  f.addEventListener('submit', async function (e) {\
    e.preventDefault();\
    var body = {}; var cfg = {};\
    new FormData(f).forEach(function (v, k) { var m = /^params\\[(.+)\\]$/.exec(k); if (m) { cfg[m[1]] = v; } else { body[k] = v; } });\
    body.connection_config = cfg;\
    var out = f.querySelector('.result');\
    try {\
      var r = await fetch(f.dataset.action, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });\
      var j = await r.json();\
      if (r.ok) { out.className = 'result ok'; out.textContent = 'Connected.'; if (window.opener) { window.opener.postMessage({ type: 'san-mateo:auth', eventType: 'AUTHORIZATION_SUCEEDED', connectionId: j.connectionId, providerConfigKey: j.providerConfigKey }, '*'); } }\
      else { out.className = 'result error'; out.textContent = (j.error && j.error.message) || 'The connection failed.'; }\
    } catch (err) { out.className = 'result error'; out.textContent = 'The connection failed.'; }\
  });\
});";

/// `GET /connect?session_token=...`: the hosted connect page.
pub(crate) async fn connect_page(State(san): State<SanMateo>, RawQuery(q): RawQuery) -> Response {
    let pairs = query_pairs(q.as_deref());
    let token = query_get(&pairs, "session_token")
        .or_else(|| query_get(&pairs, "connect_session_token"))
        .unwrap_or_default()
        .to_owned();
    let session = match san.connect_session(&token).await {
        Ok(s) => s,
        Err(e) => return error_page(&san, &e),
    };
    let integrations = match san.list_integrations(&session.environment).await {
        Ok(list) => list,
        Err(e) => return error_page(&san, &e),
    };
    let allowed = |key: &str| match &session.reconnect {
        Some(k) => k.provider_config_key == key,
        None => {
            session.allowed_integrations.is_empty()
                || session.allowed_integrations.iter().any(|a| a == key)
        }
    };
    let mut sections = String::new();
    for i in integrations.iter().filter(|i| allowed(&i.unique_key)) {
        if let Some(p) = san.catalog().get(&i.provider) {
            let mut shown = (*p).clone();
            shown.display_name.clone_from(&i.display_name);
            sections.push_str(&integration_section(&san, &token, &i.unique_key, &shown));
        }
    }
    if sections.is_empty() {
        sections.push_str("<p>No integration is available for this session.</p>");
    }
    let body = format!("<h1>Connect an account</h1>{sections}");
    page(
        &san,
        StatusCode::OK,
        "Connect an account",
        &body,
        FORM_SCRIPT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup() {
        assert_eq!(
            escape("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
        let safe = script_json(&json!({"a": "</script>&"}));
        assert!(
            !safe.contains('<') && !safe.contains('>') && !safe.contains('&'),
            "{safe}"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&safe).unwrap()["a"],
            "</script>&"
        );
    }
}
