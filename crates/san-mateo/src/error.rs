//! Errors with stable codes and HTTP status values.
//!
//! An error message never holds a token, a key or a request body.

/// The result type of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error from San Mateo.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The secret key is missing or wrong.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// The connect session token is missing, wrong or expired.
    #[error("the connect session is not valid")]
    InvalidConnectSession,
    /// The OAuth `state` is missing, wrong, used or expired.
    #[error("the OAuth state is not valid")]
    InvalidOAuthState,
    /// The provider is not in the catalog.
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    /// The integration does not exist in this environment.
    #[error("unknown integration: {0}")]
    UnknownIntegration(String),
    /// The connection does not exist.
    #[error("unknown connection: {0}")]
    UnknownConnection(String),
    /// The input is not valid.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// A required field is missing.
    #[error("missing field: {0}")]
    MissingField(String),
    /// San Mateo does not support this auth mode.
    #[error("unsupported auth mode: {0}")]
    UnsupportedAuthMode(String),
    /// The connect session does not allow this integration.
    #[error("the connect session does not allow integration {0}")]
    IntegrationNotAllowed(String),
    /// A provider template is not valid or a value is missing.
    #[error("template error: {0}")]
    Template(String),
    /// The upstream host is not allowed.
    #[error("host not allowed: {0}")]
    ForbiddenHost(String),
    /// The item exists already.
    #[error("conflict: {0}")]
    Conflict(String),
    /// The provider gave an error.
    #[error("provider error ({code}): {message}")]
    Provider {
        /// A fixed code, for example `token_exchange_failed`.
        code: &'static str,
        /// The upstream HTTP status, if any.
        status: Option<u16>,
        /// A short message. It has no secrets.
        message: String,
    },
    /// The refresh failed on too many days.
    #[error("the token refresh failed too many times")]
    RefreshExhausted,
    /// A refresh failed a short time ago.
    #[error("a token refresh failed a short time ago; try again later")]
    RefreshCooldown,
    /// The upstream call failed with no HTTP reply.
    #[error("upstream error: {0}")]
    Upstream(String),
    /// The upstream reply is larger than the limit.
    #[error("the upstream reply is too large")]
    UpstreamTooLarge,
    /// A function (sync or action) is not registered.
    #[error("unknown function: {0}")]
    UnknownFunction(String),
    /// A sync or action failed.
    #[error("function failed: {0}")]
    Function(String),
    /// Sealed data did not open.
    #[error("decryption failed")]
    Crypto,
    /// The store failed.
    #[error("store error: {0}")]
    Store(String),
    /// An unexpected error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Returns the stable error code for the API.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::InvalidConnectSession => "invalid_connect_session",
            Self::InvalidOAuthState => "invalid_oauth_state",
            Self::UnknownProvider(_) => "unknown_provider",
            Self::UnknownIntegration(_) => "unknown_provider_config",
            Self::UnknownConnection(_) => "unknown_connection",
            Self::Invalid(_) => "invalid_body",
            Self::MissingField(_) => "missing_field",
            Self::UnsupportedAuthMode(_) => "unsupported_auth_mode",
            Self::IntegrationNotAllowed(_) => "integration_not_allowed",
            Self::Template(_) => "invalid_template",
            Self::ForbiddenHost(_) => "forbidden_host",
            Self::Conflict(_) => "conflict",
            Self::Provider { code, .. } => code,
            Self::RefreshExhausted => "connection_refresh_exhausted",
            Self::RefreshCooldown => "connection_refresh_backoff",
            Self::Upstream(_) => "upstream_error",
            Self::UpstreamTooLarge => "upstream_body_too_large",
            Self::UnknownFunction(_) => "unknown_function",
            Self::Function(_) => "function_failed",
            Self::Crypto => "decryption_failed",
            Self::Store(_) => "store_error",
            Self::Internal(_) => "server_error",
        }
    }

    /// Returns the HTTP status for the API.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Unauthorized(_) | Self::InvalidConnectSession => 401,
            Self::InvalidOAuthState
            | Self::Invalid(_)
            | Self::MissingField(_)
            | Self::UnsupportedAuthMode(_)
            | Self::Template(_) => 400,
            Self::IntegrationNotAllowed(_) | Self::ForbiddenHost(_) => 403,
            Self::UnknownProvider(_)
            | Self::UnknownIntegration(_)
            | Self::UnknownConnection(_)
            | Self::UnknownFunction(_) => 404,
            Self::Conflict(_) => 409,
            Self::RefreshExhausted | Self::RefreshCooldown => 424,
            Self::Provider { .. } | Self::Upstream(_) | Self::UpstreamTooLarge => 502,
            Self::Function(_) | Self::Crypto | Self::Store(_) | Self::Internal(_) => 500,
        }
    }

    /// Returns `true` when a later attempt can succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Upstream(_) | Self::RefreshCooldown | Self::Store(_) => true,
            Self::Provider { status, .. } => match status {
                Some(s) => *s >= 500 || *s == 408 || *s == 425 || *s == 429,
                None => true,
            },
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_status() {
        let cases: Vec<(Error, &str, u16)> = vec![
            (Error::Unauthorized("x".into()), "unauthorized", 401),
            (Error::InvalidConnectSession, "invalid_connect_session", 401),
            (Error::InvalidOAuthState, "invalid_oauth_state", 400),
            (Error::UnknownProvider("x".into()), "unknown_provider", 404),
            (
                Error::UnknownIntegration("x".into()),
                "unknown_provider_config",
                404,
            ),
            (
                Error::UnknownConnection("x".into()),
                "unknown_connection",
                404,
            ),
            (Error::Invalid("x".into()), "invalid_body", 400),
            (Error::MissingField("x".into()), "missing_field", 400),
            (
                Error::UnsupportedAuthMode("x".into()),
                "unsupported_auth_mode",
                400,
            ),
            (
                Error::IntegrationNotAllowed("x".into()),
                "integration_not_allowed",
                403,
            ),
            (Error::Template("x".into()), "invalid_template", 400),
            (Error::ForbiddenHost("x".into()), "forbidden_host", 403),
            (Error::Conflict("x".into()), "conflict", 409),
            (
                Error::Provider {
                    code: "token_exchange_failed",
                    status: Some(400),
                    message: "m".into(),
                },
                "token_exchange_failed",
                502,
            ),
            (Error::RefreshExhausted, "connection_refresh_exhausted", 424),
            (Error::RefreshCooldown, "connection_refresh_backoff", 424),
            (Error::Upstream("x".into()), "upstream_error", 502),
            (Error::UpstreamTooLarge, "upstream_body_too_large", 502),
            (Error::UnknownFunction("x".into()), "unknown_function", 404),
            (Error::Function("x".into()), "function_failed", 500),
            (Error::Crypto, "decryption_failed", 500),
            (Error::Store("x".into()), "store_error", 500),
            (Error::Internal("x".into()), "server_error", 500),
        ];
        for (e, code, status) in cases {
            assert_eq!(e.code(), code, "{e:?}");
            assert_eq!(e.status(), status, "{e:?}");
        }
    }

    #[test]
    fn retryable_errors() {
        assert!(Error::Upstream("x".into()).is_retryable());
        assert!(Error::RefreshCooldown.is_retryable());
        assert!(Error::Store("x".into()).is_retryable());
        assert!(
            Error::Provider {
                code: "refresh_failed",
                status: Some(503),
                message: String::new()
            }
            .is_retryable()
        );
        assert!(
            Error::Provider {
                code: "refresh_failed",
                status: None,
                message: String::new()
            }
            .is_retryable()
        );
        assert!(
            !Error::Provider {
                code: "refresh_failed",
                status: Some(400),
                message: String::new()
            }
            .is_retryable()
        );
        assert!(!Error::RefreshExhausted.is_retryable());
        assert!(!Error::Invalid("x".into()).is_retryable());
        assert!(!Error::Crypto.is_retryable());
    }
}
