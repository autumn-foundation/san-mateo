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

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

use crate::error::{Error, Result};

/// Characters that `encodeURIComponent` keeps: letters, digits and `-_.!~*'()`.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// Percent-encodes like `encodeURIComponent`.
#[must_use]
pub fn encode_component(s: &str) -> String {
    utf8_percent_encode(s, COMPONENT).to_string()
}

/// One part of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Text(String),
    Path(String),
    Call(String, Vec<Node>),
}

/// A render failure inside one alternative.
enum Fail {
    Missing(String),
    Bad(String),
}

/// Renders a template with `vars`. See the module docs.
///
/// # Errors
///
/// [`Error::Template`] when the syntax is wrong, a function is unknown, or a
/// value is missing in all alternatives.
pub fn render(template: &str, vars: &Value) -> Result<String> {
    render_with(template, vars, false)
}

/// Renders a URL template with `vars`. See the module docs.
///
/// # Errors
///
/// [`Error::Template`] for the errors of [`render`], and for a value that is
/// not a valid host name part in the host position.
pub fn render_url(template: &str, vars: &Value) -> Result<String> {
    render_with(template, vars, true)
}

fn render_with(template: &str, vars: &Value, url: bool) -> Result<String> {
    let alts = alternatives(template)?;
    let mut missing = String::new();
    for nodes in &alts {
        match eval(nodes, vars, url) {
            Ok(s) => return Ok(s),
            Err(Fail::Missing(p)) => missing = p,
            Err(Fail::Bad(m)) => return Err(Error::Template(m)),
        }
    }
    Err(Error::Template(format!("missing value: {missing}")))
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
    fn walk(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Text(_) => {}
                Node::Path(p) => out.push(p.clone()),
                Node::Call(_, args) => walk(args, out),
            }
        }
    }
    let mut out = Vec::new();
    for nodes in alternatives(template)? {
        walk(&nodes, &mut out);
    }
    Ok(out)
}

/// Reads a dotted path from a JSON value. A key with dots (for example a
/// token reply path) also matches.
#[must_use]
pub fn lookup<'a>(vars: &'a Value, path: &str) -> Option<&'a Value> {
    let obj = vars.as_object()?;
    if let Some(v) = obj.get(path) {
        return Some(v);
    }
    let (head, rest) = path.split_once('.')?;
    lookup(obj.get(head)?, rest)
}

/// Splits a template at top-level `||` and parses each part.
fn alternatives(template: &str) -> Result<Vec<Vec<Node>>> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut last = 0;
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"${") {
            depth += 1;
            i += 2;
            continue;
        }
        match bytes[i] {
            b'}' if depth > 0 => depth -= 1,
            b'|' if depth == 0 && bytes.get(i + 1) == Some(&b'|') => {
                parts.push(&template[last..i]);
                last = i + 2;
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&template[last..]);
    let trim = parts.len() > 1;
    parts
        .into_iter()
        .map(|p| parse(if trim { p.trim() } else { p }))
        .collect()
}

fn syntax(msg: &str) -> Error {
    Error::Template(format!("bad template syntax: {msg}"))
}

fn parse(s: &str) -> Result<Vec<Node>> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = s;
    while let Some(at) = rest.find("${") {
        text.push_str(&rest[..at]);
        let inner_start = at + 2;
        let close = matching(rest, inner_start).ok_or_else(|| syntax("a `${` has no `}`"))?;
        if !text.is_empty() {
            out.push(Node::Text(std::mem::take(&mut text)));
        }
        out.push(parse_expr(&rest[inner_start..close])?);
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    if !text.is_empty() {
        out.push(Node::Text(text));
    }
    Ok(out)
}

/// Returns the index of the `}` that closes a `${` whose content starts at `start`.
fn matching(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 1_usize;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"${") {
            depth += 1;
            i += 2;
            continue;
        }
        if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_path(s: &str) -> bool {
    s.split('.').all(|part| {
        !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }) && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

fn parse_expr(inner: &str) -> Result<Node> {
    let t = inner.trim();
    if t.is_empty() {
        return Err(syntax("an empty `${}`"));
    }
    if let Some(open) = t.find('(') {
        let name = &t[..open];
        if t.ends_with(')') && is_ident(name) {
            return Ok(Node::Call(name.to_owned(), parse(&t[open + 1..t.len() - 1])?));
        }
        return Err(syntax("a function call is not closed"));
    }
    if is_path(t) {
        Ok(Node::Path(t.to_owned()))
    } else {
        Err(syntax("a placeholder is not a dotted path"))
    }
}

fn value_text(vars: &Value, path: &str) -> std::result::Result<String, Fail> {
    match lookup(vars, path) {
        None | Some(Value::Null) => Err(Fail::Missing(path.to_owned())),
        Some(Value::String(s)) if s.is_empty() => Err(Fail::Missing(path.to_owned())),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        Some(_) => Err(Fail::Bad(format!("the value of {path} is not text"))),
    }
}

fn call(name: &str, arg: &str) -> std::result::Result<String, Fail> {
    use base64::Engine as _;
    use sha2::Digest as _;
    let engine = &base64::engine::general_purpose::STANDARD;
    match name {
        "base64" => Ok(engine.encode(arg)),
        "sha256Hex" => Ok(hex::encode(sha2::Sha256::digest(arg.as_bytes()))),
        "sha256Base64" => Ok(engine.encode(sha2::Sha256::digest(arg.as_bytes()))),
        _ => Err(Fail::Bad(format!("unknown template function: {name}"))),
    }
}

fn eval_plain(nodes: &[Node], vars: &Value) -> std::result::Result<String, Fail> {
    let mut out = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Path(p) => out.push_str(&value_text(vars, p)?),
            Node::Call(name, args) => out.push_str(&call(name, &eval_plain(args, vars)?)?),
        }
    }
    Ok(out)
}

fn eval(nodes: &[Node], vars: &Value, url: bool) -> std::result::Result<String, Fail> {
    if !url {
        return eval_plain(nodes, vars);
    }
    let mut out = String::new();
    for n in nodes {
        let value = match n {
            Node::Text(t) => {
                out.push_str(t);
                continue;
            }
            Node::Path(p) => value_text(vars, p)?,
            Node::Call(name, args) => call(name, &eval_plain(args, vars)?)?,
        };
        if out.is_empty() {
            out.push_str(&value);
        } else if in_authority(&out) {
            if !is_host_part(&value) {
                return Err(Fail::Bad("a value in the host part of a URL is not a valid host name part".to_owned()));
            }
            out.push_str(&value);
        } else {
            out.push_str(&encode_component(&value));
        }
    }
    Ok(out)
}

/// Returns `true` when the next char of `prefix` is in the URL authority.
fn in_authority(prefix: &str) -> bool {
    prefix
        .split_once("://")
        .is_some_and(|(_, rest)| !rest.contains(['/', '?', '#']))
}

fn is_host_part(v: &str) -> bool {
    let edge_ok = |c: char| c.is_ascii_alphanumeric() || c == '_';
    !v.is_empty()
        && !v.contains("..")
        && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && v.chars().next().is_some_and(edge_ok)
        && v.chars().last().is_some_and(edge_ok)
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
