//! Endpoint URI options: parse and metadata.

use camel_api::CamelError;
use camel_component_api::uri::parse_bool_param;
use camel_component_api::{OptionKind, UriOption, parse_uri};

use crate::{DEFAULT_MAX_BODY_SIZE, SCHEME};

/// The HTTP methods that the `method` option and the `SanMateoMethod` header
/// accept.
pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// The largest `retries` value. The core counts at most this many retries.
pub(crate) const MAX_RETRIES: u32 = san_mateo::policy::MAX_ATTEMPTS - 1;

/// The options of one `san-mateo:` endpoint, after the parse.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EndpointOptions {
    /// The integration key: the URI path.
    pub integration: String,
    /// The default connection id.
    pub connection_id: Option<String>,
    /// The San Mateo environment.
    pub environment: String,
    /// The default upper-case HTTP method.
    pub method: String,
    /// The default upstream path and query.
    pub path: Option<String>,
    /// Retries after the first attempt.
    pub retries: u32,
    /// Status codes to retry also for non-idempotent methods.
    pub retry_on: Vec<u16>,
    /// A base URL in place of the provider base URL.
    pub base_url_override: Option<String>,
    /// Fail the exchange on a non-2xx reply.
    pub throw_exception_on_failure: bool,
    /// The largest request body in bytes.
    pub max_body_size: usize,
}

impl EndpointOptions {
    /// Parses a `san-mateo:` URI.
    ///
    /// `default_environment` and `default_max_body_size` come from the
    /// component.
    ///
    /// # Errors
    ///
    /// [`CamelError::InvalidUri`] for a URI problem, for example an unknown
    /// option or no integration key. [`CamelError::Config`] when no
    /// environment is set.
    pub(crate) fn parse(
        uri: &str,
        default_environment: Option<&str>,
        default_max_body_size: usize,
    ) -> Result<Self, CamelError> {
        let parts = parse_uri(uri)?;
        if parts.scheme != SCHEME {
            return Err(bad_uri(format!(
                "expected the scheme {SCHEME}, got {}",
                parts.scheme
            )));
        }
        let integration = parts.path.trim_matches('/');
        if integration.is_empty() {
            return Err(bad_uri(format!(
                "the integration key is missing: use {SCHEME}:<integration>"
            )));
        }
        let mut options = Self {
            integration: integration.to_owned(),
            connection_id: None,
            environment: String::new(),
            method: "GET".to_owned(),
            path: None,
            retries: 0,
            retry_on: Vec::new(),
            base_url_override: None,
            throw_exception_on_failure: true,
            max_body_size: default_max_body_size,
        };
        let mut environment = None;
        // Sort the keys, so that the first error is the same on each run.
        let mut params: Vec<(&String, &String)> = parts.params.iter().collect();
        params.sort();
        for (key, value) in params {
            match key.as_str() {
                "connectionId" => options.connection_id = Some(non_empty(key, value)?),
                "environment" => environment = Some(non_empty(key, value)?),
                "method" => {
                    options.method = normalize_method(value)
                        .ok_or_else(|| {
                            bad_uri(format!(
                                "method must be one of {}, got '{value}'",
                                METHODS.join(", ")
                            ))
                        })?
                        .to_owned();
                }
                "path" => {
                    check_path(value).map_err(|m| bad_uri(format!("path {m}")))?;
                    options.path = Some(value.clone());
                }
                "retries" => options.retries = parse_retries(value)?,
                "retryOn" => options.retry_on = parse_retry_on(value)?,
                "baseUrlOverride" => options.base_url_override = Some(non_empty(key, value)?),
                "throwExceptionOnFailure" => {
                    options.throw_exception_on_failure =
                        parse_bool_param(value).map_err(|e| bad_uri(format!("{key}: {e}")))?;
                }
                "maxBodySize" => options.max_body_size = parse_max_body_size(value)?,
                other => {
                    return Err(bad_uri(format!(
                        "unknown {SCHEME}: option '{other}'; the options are {}",
                        option_names().join(", ")
                    )));
                }
            }
        }
        options.environment = environment
            .or_else(|| default_environment.map(str::to_owned))
            .ok_or_else(|| {
                CamelError::Config(format!(
                    "no San Mateo environment for {SCHEME}:{integration}: set the environment \
                     option or SanMateoComponent::environment"
                ))
            })?;
        Ok(options)
    }
}

fn bad_uri(message: String) -> CamelError {
    CamelError::InvalidUri(message)
}

fn non_empty(key: &str, value: &str) -> Result<String, CamelError> {
    if value.trim().is_empty() {
        return Err(bad_uri(format!("{key} must not be empty")));
    }
    Ok(value.to_owned())
}

fn parse_retries(value: &str) -> Result<u32, CamelError> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n <= MAX_RETRIES)
        .ok_or_else(|| {
            bad_uri(format!(
                "retries must be a whole number from 0 to {MAX_RETRIES}, got '{value}'"
            ))
        })
}

fn parse_retry_on(value: &str) -> Result<Vec<u16>, CamelError> {
    value
        .split(',')
        .map(|code| {
            code.trim()
                .parse::<u16>()
                .ok()
                .filter(|c| (100..=599).contains(c))
                .ok_or_else(|| {
                    bad_uri(format!(
                        "retryOn must be a list of HTTP status codes, got '{value}'"
                    ))
                })
        })
        .collect()
}

fn parse_max_body_size(value: &str) -> Result<usize, CamelError> {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            bad_uri(format!(
                "maxBodySize must be a number of bytes above 0, got '{value}'"
            ))
        })
}

/// Returns the upper-case method, or `None` when the method is not in
/// [`METHODS`].
pub(crate) fn normalize_method(method: &str) -> Option<&'static str> {
    let method = method.trim();
    METHODS
        .iter()
        .find(|m| m.eq_ignore_ascii_case(method))
        .copied()
}

/// Checks an upstream path. It must start with `/` and have no white space
/// or control characters.
pub(crate) fn check_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err("must start with '/'".to_owned());
    }
    if path.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("must not have white space or control characters".to_owned());
    }
    Ok(())
}

fn option_names() -> Vec<String> {
    uri_options().into_iter().map(|o| o.name).collect()
}

/// Returns the URI options for the component metadata.
#[must_use]
pub fn uri_options() -> Vec<UriOption> {
    let max = DEFAULT_MAX_BODY_SIZE.to_string();
    vec![
        UriOption::new(
            "connectionId",
            "The connection id. The SanMateoConnectionId header replaces it.",
            OptionKind::String,
        ),
        UriOption::new(
            "environment",
            "The San Mateo environment. The default comes from the component.",
            OptionKind::String,
        ),
        UriOption::new(
            "method",
            "The HTTP method. The SanMateoMethod header replaces it.",
            OptionKind::Enum(METHODS.iter().map(|m| (*m).to_owned()).collect()),
        )
        .with_default("GET"),
        UriOption::new(
            "path",
            "The upstream path and query, percent-encoded. The SanMateoPath header replaces it.",
            OptionKind::String,
        ),
        UriOption::new(
            "retries",
            "Retries after the first attempt, from 0 to 9.",
            OptionKind::Int,
        )
        .with_default("0"),
        UriOption::new(
            "retryOn",
            "Status codes to retry also for non-idempotent methods, for example 500,502.",
            OptionKind::List(Box::new(OptionKind::Int)),
        ),
        UriOption::new(
            "baseUrlOverride",
            "A base URL in place of the provider base URL. San Mateo checks the host.",
            OptionKind::String,
        ),
        UriOption::new(
            "throwExceptionOnFailure",
            "Fail the exchange on a non-2xx reply.",
            OptionKind::Bool,
        )
        .with_default("true"),
        UriOption::new(
            "maxBodySize",
            "The largest request body in bytes.",
            OptionKind::Int,
        )
        .with_default(&max),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const MIB2: usize = 2 * 1024 * 1024;

    fn parse(uri: &str) -> Result<EndpointOptions, CamelError> {
        EndpointOptions::parse(uri, Some("dev"), MIB2)
    }

    fn invalid_uri(uri: &str) -> String {
        match parse(uri) {
            Err(CamelError::InvalidUri(m)) => m,
            other => panic!("expected InvalidUri for {uri}, got {other:?}"),
        }
    }

    #[test]
    fn parses_all_options() {
        let o = parse(
            "san-mateo:crm?connectionId=c1&environment=prod&method=post&path=/v3/contacts%3Flimit%3D10\
             &retries=2&retryOn=500,%20502&baseUrlOverride=https://eu.crm.example.com\
             &throwExceptionOnFailure=false&maxBodySize=1024",
        )
        .unwrap();
        assert_eq!(o.integration, "crm");
        assert_eq!(o.connection_id.as_deref(), Some("c1"));
        assert_eq!(o.environment, "prod");
        assert_eq!(o.method, "POST");
        assert_eq!(o.path.as_deref(), Some("/v3/contacts?limit=10"));
        assert_eq!(o.retries, 2);
        assert_eq!(o.retry_on, vec![500, 502]);
        assert_eq!(
            o.base_url_override.as_deref(),
            Some("https://eu.crm.example.com")
        );
        assert!(!o.throw_exception_on_failure);
        assert_eq!(o.max_body_size, 1024);
    }

    #[test]
    fn defaults() {
        let o = parse("san-mateo:crm").unwrap();
        assert_eq!(o.integration, "crm");
        assert_eq!(o.connection_id, None);
        assert_eq!(o.environment, "dev", "the component default");
        assert_eq!(o.method, "GET");
        assert_eq!(o.path, None);
        assert_eq!(o.retries, 0);
        assert!(o.retry_on.is_empty());
        assert_eq!(o.base_url_override, None);
        assert!(o.throw_exception_on_failure);
        assert_eq!(o.max_body_size, MIB2);
        let o = EndpointOptions::parse("san-mateo:crm?environment=prod", None, 7).unwrap();
        assert_eq!(o.environment, "prod");
        assert_eq!(o.max_body_size, 7);
    }

    #[test]
    fn slashes_around_the_integration_key_are_ignored() {
        assert_eq!(parse("san-mateo://crm").unwrap().integration, "crm");
        assert_eq!(parse("san-mateo:crm/").unwrap().integration, "crm");
    }

    #[test]
    fn a_missing_environment_is_a_config_error() {
        match EndpointOptions::parse("san-mateo:crm?connectionId=c1", None, MIB2) {
            Err(CamelError::Config(m)) => assert!(m.contains("environment"), "{m}"),
            other => panic!("{other:?}"),
        }
        match parse("san-mateo:crm?environment=") {
            Err(CamelError::InvalidUri(m)) => assert!(m.contains("environment"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_bad_uris() {
        let m = invalid_uri("san-mateo:crm?connectionId=c1&pathh=/x");
        assert!(m.contains("unknown") && m.contains("pathh"), "{m}");
        assert!(m.contains("connectionId"), "the message lists the options: {m}");
        assert!(invalid_uri("san-mateo:?connectionId=c1").contains("integration"));
        assert!(invalid_uri("san-mateo:/").contains("integration"));
        assert!(invalid_uri("http:crm").contains("san-mateo"));
        assert!(invalid_uri("san-mateo:crm?method=TRACE").contains("method"));
        assert!(invalid_uri("san-mateo:crm?method=").contains("method"));
        assert!(invalid_uri("san-mateo:crm?retries=x").contains("retries"));
        assert!(invalid_uri("san-mateo:crm?retries=-1").contains("retries"));
        assert!(invalid_uri("san-mateo:crm?retries=10").contains("retries"));
        assert!(invalid_uri("san-mateo:crm?retryOn=5xx").contains("retryOn"));
        assert!(invalid_uri("san-mateo:crm?retryOn=99").contains("retryOn"));
        assert!(invalid_uri("san-mateo:crm?retryOn=600").contains("retryOn"));
        assert!(invalid_uri("san-mateo:crm?retryOn=").contains("retryOn"));
        assert!(
            invalid_uri("san-mateo:crm?throwExceptionOnFailure=maybe")
                .contains("throwExceptionOnFailure")
        );
        assert!(invalid_uri("san-mateo:crm?maxBodySize=0").contains("maxBodySize"));
        assert!(invalid_uri("san-mateo:crm?maxBodySize=big").contains("maxBodySize"));
        assert!(invalid_uri("san-mateo:crm?path=v3/contacts").contains("path"));
        assert!(invalid_uri("san-mateo:crm?path=/a%20b").contains("path"));
        assert!(invalid_uri("san-mateo:crm?path=").contains("path"));
        assert!(invalid_uri("san-mateo:crm?connectionId=").contains("connectionId"));
        assert!(invalid_uri("san-mateo:crm?baseUrlOverride=").contains("baseUrlOverride"));
        assert!(invalid_uri("san-mateo:crm?connectionId=a&connectionId=b").contains("duplicate"));
    }

    #[test]
    fn retries_up_to_the_core_limit() {
        assert_eq!(MAX_RETRIES, 9);
        assert_eq!(parse("san-mateo:crm?retries=9").unwrap().retries, 9);
    }

    #[test]
    fn methods() {
        for m in METHODS {
            assert_eq!(normalize_method(&m.to_ascii_lowercase()), Some(*m));
        }
        assert_eq!(normalize_method(" get "), Some("GET"));
        assert_eq!(normalize_method("TRACE"), None);
        assert_eq!(normalize_method("CONNECT"), None);
        assert_eq!(normalize_method(""), None);
    }

    #[test]
    fn paths() {
        assert!(check_path("/v3/contacts?limit=10&after=x").is_ok());
        assert!(check_path("/").is_ok());
        assert!(check_path("").is_err());
        assert!(check_path("v3").is_err());
        assert!(check_path("https://evil.example.com/x").is_err());
        assert!(check_path("/a b").is_err());
        assert!(check_path("/a\r\nX-Injected: 1").is_err());
        assert!(check_path("/a\u{0}").is_err());
    }

    #[test]
    fn metadata_options_match_the_parser() {
        let opts = uri_options();
        let names: Vec<&str> = opts.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "connectionId",
                "environment",
                "method",
                "path",
                "retries",
                "retryOn",
                "baseUrlOverride",
                "throwExceptionOnFailure",
                "maxBodySize",
            ]
        );
        for o in &opts {
            assert!(!o.description.is_empty(), "{}", o.name);
            assert!(!o.secret, "{}", o.name);
            assert!(!o.required, "the header can give {}", o.name);
        }
        let method = opts.iter().find(|o| o.name == "method").unwrap();
        assert_eq!(method.default_value.as_deref(), Some("GET"));
        assert_eq!(
            method.kind,
            OptionKind::Enum(METHODS.iter().map(|m| (*m).to_owned()).collect())
        );
        let throw = opts
            .iter()
            .find(|o| o.name == "throwExceptionOnFailure")
            .unwrap();
        assert_eq!(throw.kind, OptionKind::Bool);
        assert_eq!(throw.default_value.as_deref(), Some("true"));
        let max = opts.iter().find(|o| o.name == "maxBodySize").unwrap();
        assert_eq!(max.default_value.as_deref(), Some("2097152"));
        let retry_on = opts.iter().find(|o| o.name == "retryOn").unwrap();
        assert_eq!(retry_on.kind, OptionKind::List(Box::new(OptionKind::Int)));
        // Each listed option parses.
        let uri = "san-mateo:crm?connectionId=c&environment=e&method=GET&path=/x&retries=1\
                   &retryOn=500&baseUrlOverride=https://a.example.com\
                   &throwExceptionOnFailure=true&maxBodySize=5";
        assert!(parse(uri).is_ok());
    }
}
