//! `${...}` templates from the provider catalog.
//!
//! - `${a.b}` reads a dotted path from the variables.
//! - `${base64(...)}`, `${sha256Hex(...)}` and `${sha256Base64(...)}` apply a
//!   function to a nested template.
//! - `A || B` uses `A` when all its values exist. Else it uses `B`.
//! - A missing or empty value is an error.
//!
//! [`render_url`] knows the position of each value in a URL:
//!
//! - A value at the start of the template is a full URL. It goes in as it is.
//! - A value in the host part must be a host name part (`[A-Za-z0-9._-]`).
//! - A value in the path or query is percent-encoded.

use serde_json::Value;

use crate::error::{Error, Result};

/// Renders a template with `vars`. See the module docs.
///
/// # Errors
///
/// [`Error::Template`] when the syntax is wrong, a function is unknown, or a
/// value is missing in all alternatives.
pub fn render(template: &str, vars: &Value) -> Result<String> {
    let _ = (template, vars);
    todo!()
}

/// Renders a URL template with `vars`. See the module docs.
///
/// # Errors
///
/// [`Error::Template`] for the errors of [`render`], and for a value that is
/// not a valid host name part in the host position.
pub fn render_url(template: &str, vars: &Value) -> Result<String> {
    let _ = (template, vars);
    todo!()
}

/// Returns `true` when the template has a `${` placeholder.
#[must_use]
pub fn has_placeholders(template: &str) -> bool {
    template.contains("${")
}

/// Returns the dotted paths that the template reads, in order.
///
/// # Errors
///
/// [`Error::Template`] when the syntax is wrong.
pub fn paths(template: &str) -> Result<Vec<String>> {
    let _ = template;
    todo!()
}

/// Reads a dotted path from a JSON value.
#[must_use]
pub fn lookup<'a>(vars: &'a Value, path: &str) -> Option<&'a Value> {
    let _ = (vars, path);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars() -> Value {
        json!({
            "apiKey": "k-1",
            "accessToken": "at",
            "port": 8443,
            "flag": true,
            "empty": "",
            "obj": {"x": 1},
            "connectionConfig": {"subdomain": "acme", "region": "eu-west-1", "path": "a b/c", "url": "https://x.example.com/v1"},
            "credentials": {"username": "user", "password": "p@ss"}
        })
    }

    #[test]
    fn plain_values() {
        let v = vars();
        assert_eq!(render("Bearer ${apiKey}", &v).unwrap(), "Bearer k-1");
        assert_eq!(render("${connectionConfig.subdomain}-${connectionConfig.region}", &v).unwrap(), "acme-eu-west-1");
        assert_eq!(render("${port}/${flag}", &v).unwrap(), "8443/true");
        assert_eq!(render("no placeholders", &v).unwrap(), "no placeholders");
        assert_eq!(render("${ apiKey }", &v).unwrap(), "k-1");
    }

    #[test]
    fn missing_values_fail() {
        let v = vars();
        for t in ["${nope}", "${connectionConfig.nope}", "${empty}", "x ${credentials.username.deep}"] {
            assert!(matches!(render(t, &v), Err(Error::Template(_))), "{t}");
        }
        assert!(matches!(render("${obj}", &v), Err(Error::Template(_))));
    }

    #[test]
    fn fallback_uses_the_first_complete_alternative() {
        let v = vars();
        assert_eq!(render("https://${connectionConfig.host} || https://default.example.com", &v).unwrap(), "https://default.example.com");
        assert_eq!(render("https://${connectionConfig.subdomain}.example.com || https://default.example.com", &v).unwrap(), "https://acme.example.com");
        assert_eq!(render("${connectionConfig.version} || 2025-05-05", &v).unwrap(), "2025-05-05");
        assert!(matches!(render("${a} || ${b}", &v), Err(Error::Template(_))));
    }

    #[test]
    fn functions() {
        let v = vars();
        assert_eq!(
            render("Basic ${base64(${credentials.username}:${credentials.password})}", &v).unwrap(),
            "Basic dXNlcjpwQHNz"
        );
        assert_eq!(
            render("${sha256Hex(abc)}", &v).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(render("${sha256Base64(abc)}", &v).unwrap(), "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
        assert!(matches!(render("${nope(x)}", &v), Err(Error::Template(_))));
        assert!(matches!(render("${base64(${missing})}", &v), Err(Error::Template(_))));
    }

    #[test]
    fn syntax_errors() {
        let v = vars();
        for t in ["${apiKey", "${", "${}", "${a b}", "${base64(x}"] {
            assert!(matches!(render(t, &v), Err(Error::Template(_))), "{t}");
        }
    }

    #[test]
    fn url_positions() {
        let v = vars();
        assert_eq!(render_url("https://${connectionConfig.subdomain}.zendesk.com/api", &v).unwrap(), "https://acme.zendesk.com/api");
        assert_eq!(render_url("https://api.example.com/${connectionConfig.path}?q=${connectionConfig.path}", &v).unwrap(), "https://api.example.com/a%20b%2Fc?q=a%20b%2Fc");
        assert_eq!(render_url("${connectionConfig.url}", &v).unwrap(), "https://x.example.com/v1");
        assert_eq!(render_url("${connectionConfig.url}/users", &v).unwrap(), "https://x.example.com/v1/users");
    }

    #[test]
    fn host_injection_is_rejected() {
        for bad in ["evil.com/x", "a@b", "a:80", "a?b", "a#b", "..", "a b", "-a", "a\\b"] {
            let v = json!({"connectionConfig": {"subdomain": bad}});
            assert!(
                matches!(render_url("https://${connectionConfig.subdomain}.zendesk.com", &v), Err(Error::Template(_))),
                "{bad}"
            );
        }
        let ok = json!({"connectionConfig": {"subdomain": "my-team_1.eu"}});
        assert!(render_url("https://${connectionConfig.subdomain}.zendesk.com", &ok).is_ok());
    }

    #[test]
    fn url_fallback() {
        let v = vars();
        assert_eq!(
            render_url("https://${connectionConfig.hostname}/auth || https://login.example.com/auth", &v).unwrap(),
            "https://login.example.com/auth"
        );
    }

    #[test]
    fn paths_and_lookup() {
        assert_eq!(
            paths("${a.b} x ${base64(${c}:${d.e})} || ${f}").unwrap(),
            vec!["a.b", "c", "d.e", "f"]
        );
        let v = vars();
        assert_eq!(lookup(&v, "connectionConfig.subdomain"), Some(&json!("acme")));
        assert_eq!(lookup(&v, "connectionConfig.nope"), None);
        assert!(has_placeholders("${x}"));
        assert!(!has_placeholders("x"));
    }
}
