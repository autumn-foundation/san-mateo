//! The provider catalog. It reads the Nango `providers.yaml` format.
//!
//! San Mateo reads only the fields that it uses. It ignores other fields.
//! An entry with `alias: base` starts from the `base` entry. Its own keys
//! replace the keys of the base (a shallow merge).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Error, Result};
use crate::paginate::PaginationConfig;

/// The built-in catalog source. It comes from public provider docs.
pub const BUILTIN_YAML: &str = include_str!("../providers/providers.yaml");

/// How a provider authenticates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AuthMode {
    /// OAuth 2.0 authorization code flow.
    OAuth2,
    /// OAuth 2.0 client credentials flow.
    OAuth2ClientCredentials,
    /// An API key.
    ApiKey,
    /// HTTP Basic auth.
    Basic,
    /// No auth.
    None,
    /// A mode that San Mateo reads but does not support.
    Other(String),
}

impl Default for AuthMode {
    fn default() -> Self {
        Self::Other(String::new())
    }
}

impl AuthMode {
    /// Returns the catalog name, for example `OAUTH2`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::OAuth2 => "OAUTH2",
            Self::OAuth2ClientCredentials => "OAUTH2_CC",
            Self::ApiKey => "API_KEY",
            Self::Basic => "BASIC",
            Self::None => "NONE",
            Self::Other(s) => s,
        }
    }

    /// Returns `true` when San Mateo can make connections with this mode.
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        !matches!(self, Self::Other(_))
    }

    /// Parses a catalog name. Unknown names give [`AuthMode::Other`].
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "OAUTH2" => Self::OAuth2,
            "OAUTH2_CC" => Self::OAuth2ClientCredentials,
            "API_KEY" => Self::ApiKey,
            "BASIC" => Self::Basic,
            "NONE" => Self::None,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl Serialize for AuthMode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AuthMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Self::parse(&s))
    }
}

/// The token request body format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyFormat {
    /// `application/x-www-form-urlencoded`.
    #[default]
    Form,
    /// `application/json`.
    Json,
}

/// How the client authenticates at the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TokenAuthMethod {
    /// `client_id` and `client_secret` in the body.
    #[default]
    Body,
    /// `Authorization: Basic base64(client_id:client_secret)`.
    Basic,
    /// A method that San Mateo does not support.
    Other(String),
}

impl<'de> Deserialize<'de> for TokenAuthMethod {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "basic" => Self::Basic,
            "body" => Self::Body,
            _ => Self::Other(s),
        })
    }
}

impl Serialize for TokenAuthMethod {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::Body => "body",
            Self::Basic => "basic",
            Self::Other(o) => o,
        })
    }
}

/// The Nango `authorization_method` field: where the client puts its id and
/// secret at the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthorizationMethod {
    /// In the body.
    #[default]
    Body,
    /// In an `Authorization: Basic` header.
    Header,
    /// A method that San Mateo does not support.
    Other(String),
}

impl<'de> Deserialize<'de> for AuthorizationMethod {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "header" => Self::Header,
            "body" => Self::Body,
            _ => Self::Other(s),
        })
    }
}

impl Serialize for AuthorizationMethod {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::Body => "body",
            Self::Header => "header",
            Self::Other(o) => o,
        })
    }
}

/// A token URL: one URL, or one URL for each auth mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TokenUrl {
    /// One URL for all modes.
    One(String),
    /// URLs by auth mode name (`OAUTH2`, `OAUTH2CC`, ...).
    ByMode(BTreeMap<String, String>),
}

/// A schema for one connection config or credential field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSchema {
    /// The JSON type. Always `string` in the Nango format.
    #[serde(default, rename = "type", deserialize_with = "de_opt_string")]
    pub kind: Option<String>,
    /// A short title.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub title: Option<String>,
    /// A description.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub description: Option<String>,
    /// An example value.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub example: Option<String>,
    /// A regular expression that the value must match.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub pattern: Option<String>,
    /// `true` when the field can be empty.
    #[serde(default, deserialize_with = "de_opt_bool")]
    pub optional: Option<bool>,
    /// A format: `hostname`, `uri`, `uuid` or `email`.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub format: Option<String>,
    /// A default value.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub default_value: Option<String>,
    /// `true` for a secret field.
    #[serde(default, deserialize_with = "de_opt_bool")]
    pub secret: Option<bool>,
    /// `true` when San Mateo fills the field (for example from the token reply).
    #[serde(default, deserialize_with = "de_opt_bool")]
    pub automated: Option<bool>,
    /// The allowed values.
    #[serde(default, rename = "enum", deserialize_with = "de_string_vec")]
    pub allowed: Vec<String>,
    /// Text to show before the input.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub prefix: Option<String>,
    /// Text to show after the input.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub suffix: Option<String>,
}

/// Provider headers that tell when to retry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryHeaders {
    /// Headers with an instant (epoch seconds, epoch ms, or a date).
    #[serde(default, deserialize_with = "de_string_or_vec")]
    pub at: Vec<String>,
    /// Headers with a wait in seconds.
    #[serde(default, deserialize_with = "de_string_or_vec")]
    pub after: Vec<String>,
    /// A header with the number of calls that remain.
    #[serde(default)]
    pub remaining: Option<String>,
    /// Status values to retry: `"429"`, `"5xx"` or `"500-502"`.
    #[serde(default, deserialize_with = "de_string_vec")]
    pub error_code: Vec<String>,
}

/// An endpoint that checks API key or basic credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    /// HTTP method.
    #[serde(default = "default_get")]
    pub method: String,
    /// Paths to try. One success is enough.
    #[serde(default)]
    pub endpoints: Vec<String>,
    /// A base URL to use in place of `proxy.base_url`.
    #[serde(default)]
    pub base_url_override: Option<String>,
    /// A JSON body to send.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    /// Extra headers.
    #[serde(default, deserialize_with = "de_string_map")]
    pub headers: BTreeMap<String, String>,
}

fn default_get() -> String {
    "GET".to_owned()
}

/// The proxy block of a provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// The upstream base URL. It can be a template.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Headers to add. Values can be templates.
    #[serde(default, deserialize_with = "de_string_map")]
    pub headers: BTreeMap<String, String>,
    /// Query parameters to add. Values can be templates.
    #[serde(default, deserialize_with = "de_string_map")]
    pub query: BTreeMap<String, String>,
    /// Provider retry headers.
    #[serde(default, deserialize_with = "de_lenient")]
    pub retry: Option<RetryHeaders>,
    /// The default pagination. A kind that San Mateo does not know gives `None`.
    #[serde(default, deserialize_with = "de_lenient")]
    pub paginate: Option<PaginationConfig>,
    /// A check for API key and basic credentials.
    #[serde(default, deserialize_with = "de_lenient")]
    pub verification: Option<Verification>,
}

/// One provider from the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// The catalog key, for example `github`. The catalog sets it.
    #[serde(default)]
    pub name: String,
    /// The name to show.
    #[serde(default)]
    pub display_name: String,
    /// The auth mode.
    #[serde(default)]
    pub auth_mode: AuthMode,
    /// Categories, for example `crm`.
    #[serde(default, deserialize_with = "de_string_vec")]
    pub categories: Vec<String>,
    /// A link to the provider API docs.
    #[serde(default)]
    pub docs: Option<String>,
    /// The `OAuth2` authorization URL. It can be a template.
    #[serde(default)]
    pub authorization_url: Option<String>,
    /// Extra authorization query parameters. Values can be templates.
    #[serde(default, deserialize_with = "de_string_map")]
    pub authorization_params: BTreeMap<String, String>,
    /// The callback query parameter with the code. Default `code`.
    #[serde(default)]
    pub authorization_code_param_in_callback: Option<String>,
    /// The token URL.
    #[serde(default, deserialize_with = "de_lenient")]
    pub token_url: Option<TokenUrl>,
    /// Extra token request parameters. Values can be templates.
    #[serde(default, deserialize_with = "de_string_map")]
    pub token_params: BTreeMap<String, String>,
    /// A refresh URL. Default: the token URL.
    #[serde(default)]
    pub refresh_url: Option<String>,
    /// Extra refresh request parameters. Default: `token_params`.
    #[serde(default, deserialize_with = "de_string_map")]
    pub refresh_params: BTreeMap<String, String>,
    /// The scope separator. Default one space.
    #[serde(default)]
    pub scope_separator: Option<String>,
    /// Scopes to always ask for.
    #[serde(default, deserialize_with = "de_string_vec")]
    pub default_scopes: Vec<String>,
    /// `true` to turn off PKCE.
    #[serde(default, deserialize_with = "de_opt_bool")]
    pub disable_pkce: Option<bool>,
    /// The token request body format.
    #[serde(default, deserialize_with = "de_lenient")]
    pub body_format: Option<BodyFormat>,
    /// The client auth at the token endpoint.
    #[serde(default, deserialize_with = "de_lenient")]
    pub token_request_auth_method: Option<TokenAuthMethod>,
    /// The Nango client auth field. `header` is the same as
    /// `token_request_auth_method: basic`.
    #[serde(default, deserialize_with = "de_lenient")]
    pub authorization_method: Option<AuthorizationMethod>,
    /// A dotted path to the token object in the token reply.
    #[serde(default)]
    pub alternate_access_token_response_path: Option<String>,
    /// `milliseconds` when `expires_in` is in ms.
    #[serde(default)]
    pub expires_in_unit: Option<String>,
    /// The refresh margin in seconds.
    #[serde(default, deserialize_with = "de_lenient")]
    pub token_expiration_buffer: Option<u64>,
    /// Dotted paths in the token reply to copy into the connection config.
    #[serde(default, deserialize_with = "de_string_vec")]
    pub token_response_metadata: Vec<String>,
    /// Callback query parameters to copy into the connection config.
    #[serde(default, deserialize_with = "de_string_vec")]
    pub redirect_uri_metadata: Vec<String>,
    /// The proxy block.
    #[serde(default, deserialize_with = "de_lenient")]
    pub proxy: Option<ProxyConfig>,
    /// Connection config fields.
    #[serde(default, deserialize_with = "de_schema_map")]
    pub connection_config: BTreeMap<String, FieldSchema>,
    /// Credential fields.
    #[serde(default, deserialize_with = "de_schema_map")]
    pub credentials: BTreeMap<String, FieldSchema>,
    /// Legacy list of connection config field names.
    #[serde(default, deserialize_with = "de_string_vec")]
    pub connection_configuration: Vec<String>,
}

impl Provider {
    /// Returns the token URL template for `mode`.
    #[must_use]
    pub fn token_url_for(&self, mode: &AuthMode) -> Option<&str> {
        match self.token_url.as_ref()? {
            TokenUrl::One(u) => Some(u),
            TokenUrl::ByMode(m) => {
                let key = match mode {
                    AuthMode::OAuth2ClientCredentials => "OAUTH2CC",
                    other => other.as_str(),
                };
                m.get(key).map(String::as_str)
            }
        }
    }

    /// Returns the refresh margin in ms.
    #[must_use]
    pub fn refresh_buffer_ms(&self) -> u64 {
        self.token_expiration_buffer.map_or(
            crate::policy::DEFAULT_REFRESH_BUFFER_MS,
            crate::policy::secs_to_ms,
        )
    }

    /// Returns the scope separator.
    #[must_use]
    pub fn scope_separator(&self) -> &str {
        self.scope_separator.as_deref().unwrap_or(" ")
    }

    /// Returns `true` when the authorization request uses PKCE.
    #[must_use]
    pub fn uses_pkce(&self) -> bool {
        !self.disable_pkce.unwrap_or(false)
    }

    /// Returns `true` when the client uses Basic auth at the token endpoint:
    /// `token_request_auth_method: basic` or `authorization_method: header`.
    #[must_use]
    pub const fn uses_basic_client_auth(&self) -> bool {
        matches!(self.token_request_auth_method, Some(TokenAuthMethod::Basic))
            || matches!(self.authorization_method, Some(AuthorizationMethod::Header))
    }

    /// Returns `true` when `expires_in` is in ms.
    #[must_use]
    pub fn expires_in_ms(&self) -> bool {
        self.expires_in_unit.as_deref() == Some("milliseconds")
    }

    /// Returns the proxy base URL template.
    #[must_use]
    pub fn base_url(&self) -> Option<&str> {
        self.proxy.as_ref()?.base_url.as_deref()
    }
}

/// The provider catalog.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    providers: BTreeMap<String, Arc<Provider>>,
}

impl Catalog {
    /// Returns the built-in catalog.
    ///
    /// # Errors
    ///
    /// Fails only when the built-in file is broken. A test checks it.
    pub fn builtin() -> Result<Self> {
        Self::from_yaml_str(BUILTIN_YAML)
    }

    /// Reads a catalog in the Nango `providers.yaml` format.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the YAML does not parse, an alias is unknown or
    /// makes a loop, or an entry does not match the schema.
    pub fn from_yaml_str(yaml: &str) -> Result<Self> {
        let root: serde_yaml::Value =
            serde_yaml::from_str(yaml).map_err(|e| Error::Invalid(format!("catalog YAML: {e}")))?;
        let entries = match root {
            serde_yaml::Value::Mapping(m) => m,
            serde_yaml::Value::Null => serde_yaml::Mapping::new(),
            _ => {
                return Err(Error::Invalid(
                    "the catalog must be a map of providers".to_owned(),
                ));
            }
        };
        let mut raw = BTreeMap::new();
        for (k, v) in entries {
            let name = scalar_to_string(k)
                .ok_or_else(|| Error::Invalid("a provider name is not text".to_owned()))?;
            raw.insert(name, v);
        }
        let mut providers = BTreeMap::new();
        for name in raw.keys() {
            let merged = resolve_alias(&raw, name)?;
            let mut p: Provider = serde_yaml::from_value(merged)
                .map_err(|e| Error::Invalid(format!("provider {name}: {e}")))?;
            p.name.clone_from(name);
            providers.insert(name.clone(), Arc::new(p));
        }
        Ok(Self { providers })
    }

    /// Reads a catalog file.
    ///
    /// # Errors
    ///
    /// Fails when the file does not read or parse.
    pub fn from_yaml_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Invalid(format!("catalog file {}: {e}", path.display())))?;
        Self::from_yaml_str(&text)
    }

    /// Adds the providers of `other`. A provider in `other` replaces one with
    /// the same name.
    #[must_use]
    pub fn merge(mut self, other: Self) -> Self {
        self.providers.extend(other.providers);
        self
    }

    /// Returns a provider.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<Provider>> {
        self.providers.get(name).cloned()
    }

    /// Returns a provider or [`Error::UnknownProvider`].
    ///
    /// # Errors
    ///
    /// [`Error::UnknownProvider`] when the catalog does not have it.
    pub fn require(&self, name: &str) -> Result<Arc<Provider>> {
        self.get(name)
            .ok_or_else(|| Error::UnknownProvider(name.to_owned()))
    }

    /// Returns all providers, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<Provider>> {
        self.providers.values()
    }

    /// Returns the number of providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Returns `true` when the catalog is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

/// Reads a string, a number or a bool as a string.
fn scalar_to_string(v: serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The deepest chain of aliases that the catalog accepts.
const MAX_ALIAS_DEPTH: usize = 8;

/// Returns the entry with its alias chain merged in (shallow, own keys win).
fn resolve_alias(
    raw: &BTreeMap<String, serde_yaml::Value>,
    name: &str,
) -> Result<serde_yaml::Value> {
    let alias_key = serde_yaml::Value::String("alias".to_owned());
    let mut chain = vec![name.to_owned()];
    let mut current = raw
        .get(name)
        .cloned()
        .ok_or_else(|| Error::Invalid(format!("unknown provider {name}")))?;
    loop {
        let serde_yaml::Value::Mapping(mut own) = current else {
            return Err(Error::Invalid(format!("provider {name} is not a map")));
        };
        let Some(base_name) = own.remove(&alias_key).and_then(scalar_to_string) else {
            return Ok(serde_yaml::Value::Mapping(own));
        };
        if chain.contains(&base_name) || chain.len() > MAX_ALIAS_DEPTH {
            return Err(Error::Invalid(format!("provider {name} has an alias loop")));
        }
        let base = raw
            .get(&base_name)
            .ok_or_else(|| Error::Invalid(format!("provider {name}: unknown alias {base_name}")))?;
        let serde_yaml::Value::Mapping(mut merged) = base.clone() else {
            return Err(Error::Invalid(format!("provider {base_name} is not a map")));
        };
        for (k, v) in own {
            merged.insert(k, v);
        }
        chain.push(base_name);
        current = serde_yaml::Value::Mapping(merged);
    }
}

/// Reads a value with `T`. A value of another shape gives `None`.
fn de_lenient<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_yaml::Value::deserialize(d)?;
    Ok(serde_yaml::from_value(v).ok())
}

fn de_schema_map<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<String, FieldSchema>, D::Error> {
    let v = serde_yaml::Value::deserialize(d)?;
    let mut out = BTreeMap::new();
    if let serde_yaml::Value::Mapping(m) = v {
        for (k, v) in m {
            if let (Some(k), Ok(f)) = (scalar_to_string(k), serde_yaml::from_value(v)) {
                out.insert(k, f);
            }
        }
    }
    Ok(out)
}

fn de_opt_bool<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<bool>, D::Error> {
    let v = serde_yaml::Value::deserialize(d)?;
    Ok(match v {
        serde_yaml::Value::Bool(b) => Some(b),
        serde_yaml::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

fn de_opt_string<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<String>, D::Error> {
    let v = serde_yaml::Value::deserialize(d)?;
    Ok(scalar_to_string(v))
}

fn de_string_map<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    let v = serde_yaml::Value::deserialize(d)?;
    let mut out = BTreeMap::new();
    if let serde_yaml::Value::Mapping(m) = v {
        for (k, v) in m {
            if let (Some(k), Some(v)) = (scalar_to_string(k), scalar_to_string(v)) {
                out.insert(k, v);
            }
        }
    }
    Ok(out)
}

fn de_string_vec<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    let v = serde_yaml::Value::deserialize(d)?;
    Ok(match v {
        serde_yaml::Value::Sequence(items) => {
            items.into_iter().filter_map(scalar_to_string).collect()
        }
        other => scalar_to_string(other).into_iter().collect(),
    })
}

fn de_string_or_vec<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    de_string_vec(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r"
base-crm:
    display_name: Base CRM
    auth_mode: OAUTH2
    categories: [crm]
    authorization_url: https://auth.example.com/authorize
    token_url: https://auth.example.com/token
    authorization_params:
        prompt: consent
        max_age: 3600
        offline: true
    scope_separator: ','
    disable_pkce: true
    token_expiration_buffer: 120
    body_format: json
    token_request_auth_method: basic
    token_response_metadata: [instance_url]
    unknown_field_is_ignored: { a: 1 }
    proxy:
        base_url: https://${connectionConfig.subdomain}.example.com
        headers:
            x-version: '2024'
        retry:
            at: x-reset
            after: [retry-after, x-wait]
            error_code: ['429', 5xx]
        paginate:
            type: cursor
            cursor_path_in_response: next
            cursor_name_in_request: cursor
            response_path: data
    connection_config:
        subdomain:
            type: string
            title: Subdomain
            pattern: '^[a-z0-9-]+$'
            enum: [a, b]
    docs: https://example.com/docs
base-crm-eu:
    alias: base-crm
    display_name: Base CRM EU
    proxy:
        base_url: https://eu.example.com
key-api:
    display_name: Key API
    auth_mode: API_KEY
    proxy:
        base_url: https://api.key.example.com
        headers:
            authorization: Bearer ${apiKey}
        verification:
            endpoints: [/v1/me]
cc-api:
    display_name: CC
    auth_mode: OAUTH2_CC
    token_url:
        OAUTH2CC: https://cc.example.com/token
two-step:
    display_name: Two Step
    auth_mode: TWO_STEP
";

    #[test]
    fn reads_fields() {
        let c = Catalog::from_yaml_str(SAMPLE).unwrap();
        assert_eq!(c.len(), 5);
        let p = c.get("base-crm").unwrap();
        assert_eq!(p.name, "base-crm");
        assert_eq!(p.auth_mode, AuthMode::OAuth2);
        assert_eq!(p.authorization_params["max_age"], "3600");
        assert_eq!(p.authorization_params["offline"], "true");
        assert_eq!(p.scope_separator(), ",");
        assert!(!p.uses_pkce());
        assert_eq!(p.refresh_buffer_ms(), 120_000);
        assert_eq!(p.body_format, Some(BodyFormat::Json));
        assert_eq!(p.token_request_auth_method, Some(TokenAuthMethod::Basic));
        assert!(p.uses_basic_client_auth());
        assert_eq!(
            p.token_url_for(&AuthMode::OAuth2),
            Some("https://auth.example.com/token")
        );
        let retry = p.proxy.as_ref().unwrap().retry.as_ref().unwrap();
        assert_eq!(retry.at, vec!["x-reset"]);
        assert_eq!(retry.after, vec!["retry-after", "x-wait"]);
        assert_eq!(retry.error_code, vec!["429", "5xx"]);
        assert!(matches!(
            p.proxy.as_ref().unwrap().paginate,
            Some(PaginationConfig::Cursor { .. })
        ));
        assert_eq!(p.connection_config["subdomain"].allowed, vec!["a", "b"]);
        assert_eq!(
            p.base_url(),
            Some("https://${connectionConfig.subdomain}.example.com")
        );
    }

    #[test]
    fn alias_is_a_shallow_merge() {
        let c = Catalog::from_yaml_str(SAMPLE).unwrap();
        let eu = c.get("base-crm-eu").unwrap();
        assert_eq!(eu.name, "base-crm-eu");
        assert_eq!(eu.display_name, "Base CRM EU");
        assert_eq!(eu.auth_mode, AuthMode::OAuth2);
        assert_eq!(eu.base_url(), Some("https://eu.example.com"));
        // `proxy` is replaced as a whole: the base headers are gone.
        assert!(eu.proxy.as_ref().unwrap().headers.is_empty());
    }

    #[test]
    fn defaults() {
        let c = Catalog::from_yaml_str(SAMPLE).unwrap();
        let k = c.get("key-api").unwrap();
        assert_eq!(k.scope_separator(), " ");
        assert!(k.uses_pkce());
        assert_eq!(
            k.refresh_buffer_ms(),
            crate::policy::DEFAULT_REFRESH_BUFFER_MS
        );
        assert!(!k.expires_in_ms());
        assert_eq!(
            k.proxy
                .as_ref()
                .unwrap()
                .verification
                .as_ref()
                .unwrap()
                .method,
            "GET"
        );
        let cc = c.get("cc-api").unwrap();
        assert_eq!(
            cc.token_url_for(&AuthMode::OAuth2ClientCredentials),
            Some("https://cc.example.com/token")
        );
        assert_eq!(cc.token_url_for(&AuthMode::OAuth2), None);
    }

    #[test]
    fn authorization_method_header_is_basic_client_auth() {
        let c = Catalog::from_yaml_str(
            "h:\n  authorization_method: header\nb:\n  authorization_method: body\nx:\n  authorization_method: other\nn:\n  display_name: N\n",
        )
        .unwrap();
        let h = c.get("h").unwrap();
        assert_eq!(h.authorization_method, Some(AuthorizationMethod::Header));
        assert!(h.uses_basic_client_auth());
        assert_eq!(
            serde_json::to_value(&*h).unwrap()["authorization_method"],
            "header"
        );
        for name in ["b", "x", "n"] {
            assert!(!c.get(name).unwrap().uses_basic_client_auth(), "{name}");
        }
        assert_eq!(
            c.get("x").unwrap().authorization_method,
            Some(AuthorizationMethod::Other("other".into()))
        );
    }

    #[test]
    fn auth_modes() {
        for (s, m, ok) in [
            ("OAUTH2", AuthMode::OAuth2, true),
            ("OAUTH2_CC", AuthMode::OAuth2ClientCredentials, true),
            ("API_KEY", AuthMode::ApiKey, true),
            ("BASIC", AuthMode::Basic, true),
            ("NONE", AuthMode::None, true),
            ("TWO_STEP", AuthMode::Other("TWO_STEP".into()), false),
        ] {
            assert_eq!(AuthMode::parse(s), m);
            assert_eq!(m.as_str(), s);
            assert_eq!(m.is_supported(), ok);
        }
        let c = Catalog::from_yaml_str(SAMPLE).unwrap();
        assert!(!c.get("two-step").unwrap().auth_mode.is_supported());
    }

    #[test]
    fn bad_catalogs() {
        assert!(Catalog::from_yaml_str("a: [").is_err());
        assert!(Catalog::from_yaml_str("a:\n  alias: nope\n").is_err());
        assert!(Catalog::from_yaml_str("a:\n  alias: b\nb:\n  alias: a\n").is_err());
        assert!(Catalog::from_yaml_str("- not a map").is_err());
        assert!(Catalog::from_yaml_file("/no/such/file.yaml").is_err());
    }

    #[test]
    fn merge_and_require() {
        let a = Catalog::from_yaml_str("x:\n  display_name: A\n  auth_mode: NONE\n").unwrap();
        let b = Catalog::from_yaml_str(
            "x:\n  display_name: B\n  auth_mode: NONE\ny:\n  display_name: Y\n  auth_mode: NONE\n",
        )
        .unwrap();
        let m = a.merge(b);
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("x").unwrap().display_name, "B");
        assert!(matches!(m.require("zzz"), Err(Error::UnknownProvider(_))));
        assert_eq!(m.iter().count(), 2);
        assert!(!m.is_empty());
    }

    #[test]
    fn builtin_catalog_covers_each_supported_mode() {
        let c = Catalog::builtin().unwrap();
        assert!(c.len() >= 25, "{}", c.len());
        for mode in [
            AuthMode::OAuth2,
            AuthMode::OAuth2ClientCredentials,
            AuthMode::ApiKey,
            AuthMode::Basic,
            AuthMode::None,
        ] {
            assert!(c.iter().any(|p| p.auth_mode == mode), "{mode:?}");
        }
        for p in c.iter() {
            assert!(p.auth_mode.is_supported(), "{}", p.name);
            assert!(!p.display_name.is_empty(), "{}", p.name);
            assert!(
                p.docs.as_deref().is_some_and(|d| d.starts_with("https://")),
                "{}",
                p.name
            );
            assert!(p.base_url().is_some(), "{}", p.name);
            if p.auth_mode == AuthMode::OAuth2 {
                assert!(p.authorization_url.is_some(), "{}", p.name);
                assert!(p.token_url_for(&AuthMode::OAuth2).is_some(), "{}", p.name);
            }
        }
    }

    /// Reads the full Nango catalog when `SAN_MATEO_NANGO_PROVIDERS` names it.
    #[test]
    fn reads_the_full_nango_catalog_when_present() {
        let Ok(path) = std::env::var("SAN_MATEO_NANGO_PROVIDERS") else {
            return;
        };
        let c = Catalog::from_yaml_file(path).unwrap();
        assert!(c.len() > 1_000, "{}", c.len());
        let supported = c.iter().filter(|p| p.auth_mode.is_supported()).count();
        assert!(supported * 100 / c.len() >= 80, "{supported}");
    }
}
