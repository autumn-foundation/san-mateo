//! Endpoint URI options: parse and metadata.

use camel_api::CamelError;
use camel_component_api::{OptionKind, UriOption};

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
        let _ = (uri, default_environment, default_max_body_size);
        todo!()
    }
}

/// Returns the upper-case method, or `None` when the method is not in
/// [`METHODS`].
pub(crate) fn normalize_method(method: &str) -> Option<&'static str> {
    let _ = method;
    todo!()
}

/// Checks an upstream path. It must start with `/` and have no white space
/// or control characters.
pub(crate) fn check_path(path: &str) -> Result<(), String> {
    let _ = path;
    todo!()
}

/// Returns the URI options for the component metadata.
#[must_use]
pub fn uri_options() -> Vec<UriOption> {
    let _ = (OptionKind::String, UriOption::new("", "", OptionKind::Bool));
    todo!()
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
