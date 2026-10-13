//! Maps San Mateo errors and non-2xx replies to camel errors.
//!
//! harvest-camel classifies camel errors for activity retries. The mapping
//! keeps that class equal to [`san_mateo::Error::is_retryable`]:
//!
//! | San Mateo code | Camel error | harvest-camel retries |
//! |---|---|---|
//! | `invalid_body`, `missing_field`, `invalid_template`, `conflict`, `upstream_body_too_large` | `ValidationError` | no |
//! | `unknown_provider`, `unknown_provider_config`, `unknown_connection`, `unknown_function`, `unsupported_auth_mode`, `decryption_failed`, `server_error`, `function_failed` | `Config` | no |
//! | `forbidden_host`, `integration_not_allowed` | `Unauthorized` | no |
//! | `unauthorized`, `invalid_connect_session`, `invalid_oauth_state`, `connection_refresh_exhausted` | `Unauthenticated` | no |
//! | provider error with a 4xx or 5xx status | `HttpOperationFailed` with that status | 5xx, 408, 425, 429 |
//! | provider error with no status, `connection_refresh_backoff` | `AuthProviderUnavailable` | yes |
//! | `upstream_error`, `store_error` | `Io` | yes |
//!
//! A provider error with a status outside 4xx and 5xx follows the core:
//! `AuthProviderUnavailable` when it is retryable, else `Unauthenticated`.

use camel_api::CamelError;
use san_mateo::Error;

/// Maps a [`san_mateo::Error`] to a camel error.
///
/// `method` and `url` identify the call in an HTTP error. The `url` must
/// have no query, because a query can hold a key. The message has the
/// stable San Mateo code. It never has a token or a body.
#[must_use]
pub fn to_camel_error(err: &Error, method: &str, url: &str) -> CamelError {
    let _ = (err, method, url);
    todo!()
}

/// Makes the camel HTTP error for a non-2xx reply. It has no reply body.
pub(crate) fn status_error(method: &str, url: &str, status: u16) -> CamelError {
    let _ = (method, url, status);
    todo!()
}

/// Returns the reason phrase for a status, or `HTTP error` for a status
/// with no phrase here.
pub(crate) const fn reason(status: u16) -> &'static str {
    let _ = status;
    todo!()
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A copy of `harvest_camel::is_retryable` (harvest-camel `route.rs`).
    /// harvest-camel uses camel 0.55, so this crate cannot depend on it.
    pub(crate) fn harvest_retryable(err: &CamelError) -> bool {
        match err {
            CamelError::ValidationError(_)
            | CamelError::TypeConversionFailed(_)
            | CamelError::InvalidUri(_)
            | CamelError::EndpointUri(_)
            | CamelError::Config(_)
            | CamelError::ConfigValidation(_)
            | CamelError::ComponentNotFound(_)
            | CamelError::EndpointCreationFailed(_)
            | CamelError::EndpointCreationFailedWithSource(..)
            | CamelError::Unauthenticated(_)
            | CamelError::Unauthorized(_)
            | CamelError::UnsupportedMediaType { .. }
            | CamelError::NotAcceptable { .. }
            | CamelError::StreamLimitExceeded(_)
            | CamelError::AlreadyConsumed => false,
            CamelError::HttpOperationFailed { status_code, .. } => {
                !(400..500).contains(status_code) || matches!(status_code, 408 | 425 | 429)
            }
            _ => true,
        }
    }

    fn provider(status: Option<u16>) -> Error {
        Error::Provider {
            code: "refresh_failed",
            status,
            message: "the provider gave an error".to_owned(),
        }
    }

    fn all_errors() -> Vec<(Error, &'static str)> {
        let s = || "x".to_owned();
        vec![
            (Error::Invalid(s()), "ValidationError"),
            (Error::MissingField(s()), "ValidationError"),
            (Error::Template(s()), "ValidationError"),
            (Error::Conflict(s()), "ValidationError"),
            (Error::UpstreamTooLarge, "ValidationError"),
            (Error::UnknownProvider(s()), "Config"),
            (Error::UnknownIntegration(s()), "Config"),
            (Error::UnknownConnection(s()), "Config"),
            (Error::UnknownFunction(s()), "Config"),
            (Error::UnsupportedAuthMode(s()), "Config"),
            (Error::Crypto, "Config"),
            (Error::Internal(s()), "Config"),
            (Error::Function(s()), "Config"),
            (Error::ForbiddenHost(s()), "Unauthorized"),
            (Error::IntegrationNotAllowed(s()), "Unauthorized"),
            (Error::Unauthorized(s()), "Unauthenticated"),
            (Error::InvalidConnectSession, "Unauthenticated"),
            (Error::InvalidOAuthState, "Unauthenticated"),
            (Error::RefreshExhausted, "Unauthenticated"),
            (Error::RefreshCooldown, "ProcessorError"),
            (Error::Upstream(s()), "Io"),
            (Error::Store(s()), "Io"),
            (provider(None), "ProcessorError"),
            (provider(Some(302)), "Unauthenticated"),
            (provider(Some(400)), "HttpOperationFailed"),
            (provider(Some(401)), "HttpOperationFailed"),
            (provider(Some(408)), "HttpOperationFailed"),
            (provider(Some(425)), "HttpOperationFailed"),
            (provider(Some(429)), "HttpOperationFailed"),
            (provider(Some(500)), "HttpOperationFailed"),
            (provider(Some(503)), "HttpOperationFailed"),
            (provider(Some(600)), "ProcessorError"),
        ]
    }

    #[test]
    fn each_error_maps_to_the_documented_variant() {
        for (err, variant) in all_errors() {
            let camel = to_camel_error(&err, "GET", "san-mateo:crm/v3/contacts");
            // `AuthProviderUnavailable` has the variant name `ProcessorError`.
            assert_eq!(camel.variant_name(), variant, "{err:?} -> {camel:?}");
        }
        assert!(matches!(
            to_camel_error(&Error::RefreshCooldown, "GET", "u"),
            CamelError::AuthProviderUnavailable(_)
        ));
        assert!(matches!(
            to_camel_error(&provider(None), "GET", "u"),
            CamelError::AuthProviderUnavailable(_)
        ));
    }

    #[test]
    fn harvest_camel_retries_what_the_core_retries() {
        for (err, _) in all_errors() {
            let camel = to_camel_error(&err, "POST", "san-mateo:crm/v3/contacts");
            assert_eq!(
                harvest_retryable(&camel),
                err.is_retryable(),
                "{err:?} -> {camel:?}"
            );
        }
    }

    #[test]
    fn messages_have_the_code_and_no_body() {
        let camel = to_camel_error(
            &Error::UnknownConnection("c9".to_owned()),
            "GET",
            "san-mateo:crm/x",
        );
        let text = camel.to_string();
        assert!(text.contains("unknown_connection"), "{text}");
        assert!(text.contains("c9"), "{text}");
        match to_camel_error(&provider(Some(400)), "POST", "san-mateo:crm/v3/deals") {
            CamelError::HttpOperationFailed {
                method,
                url,
                status_code,
                status_text,
                response_body,
            } => {
                assert_eq!(method, "POST");
                assert_eq!(url, "san-mateo:crm/v3/deals");
                assert_eq!(status_code, 400);
                assert!(status_text.contains("refresh_failed"), "{status_text}");
                assert_eq!(response_body, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn status_errors() {
        for (status, retry) in [
            (400, false),
            (401, false),
            (403, false),
            (404, false),
            (409, false),
            (422, false),
            (408, true),
            (425, true),
            (429, true),
            (500, true),
            (502, true),
            (503, true),
            (504, true),
        ] {
            let e = status_error("GET", "san-mateo:crm/v3/contacts", status);
            assert_eq!(harvest_retryable(&e), retry, "{status}");
            match e {
                CamelError::HttpOperationFailed {
                    status_code,
                    status_text,
                    response_body,
                    ..
                } => {
                    assert_eq!(status_code, status);
                    assert_eq!(status_text, reason(status));
                    assert_eq!(response_body, None);
                }
                other => panic!("{other:?}"),
            }
        }
        let text = status_error("DELETE", "san-mateo:crm/v3/x", 404).to_string();
        assert_eq!(text, "HTTP DELETE san-mateo:crm/v3/x failed: 404 Not Found");
    }

    #[test]
    fn reason_phrases() {
        assert_eq!(reason(400), "Bad Request");
        assert_eq!(reason(401), "Unauthorized");
        assert_eq!(reason(403), "Forbidden");
        assert_eq!(reason(404), "Not Found");
        assert_eq!(reason(405), "Method Not Allowed");
        assert_eq!(reason(408), "Request Timeout");
        assert_eq!(reason(409), "Conflict");
        assert_eq!(reason(410), "Gone");
        assert_eq!(reason(413), "Content Too Large");
        assert_eq!(reason(415), "Unsupported Media Type");
        assert_eq!(reason(422), "Unprocessable Content");
        assert_eq!(reason(425), "Too Early");
        assert_eq!(reason(429), "Too Many Requests");
        assert_eq!(reason(500), "Internal Server Error");
        assert_eq!(reason(501), "Not Implemented");
        assert_eq!(reason(502), "Bad Gateway");
        assert_eq!(reason(503), "Service Unavailable");
        assert_eq!(reason(504), "Gateway Timeout");
        assert_eq!(reason(302), "Found");
        assert_eq!(reason(304), "Not Modified");
        assert_eq!(reason(418), "HTTP error");
    }
}
