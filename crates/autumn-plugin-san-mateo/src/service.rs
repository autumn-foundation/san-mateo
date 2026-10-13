//! `SanMateoService`: the started service, for handlers and jobs.

use std::ops::Deref;

use autumn_web::reexports::axum::extract::FromRequestParts;
use autumn_web::reexports::http::StatusCode;
use autumn_web::reexports::http::request::Parts;
use autumn_web::{AppState, AutumnError, AutumnResult};
use san_mateo::SanMateo;

/// The message of a 503 before the service starts.
pub(crate) const NOT_STARTED: &str = "San Mateo has not started";

/// The started San Mateo service. The plugin puts it in the app state.
///
/// Get it in a handler with [`SanMateoService::from_state`] or
/// [`SanMateoService::require`], or as an extractor:
///
/// ```rust,ignore
/// async fn contacts(san: SanMateoService) -> AutumnResult<String> {
///     let reply = san
///         .proxy("prod", &key, ProxyRequest::get("/v3/contacts"))
///         .await
///         .map_err(autumn_error)?;
///     Ok(String::from_utf8_lossy(&reply.body).into_owned())
/// }
/// ```
#[derive(Clone, Debug)]
pub struct SanMateoService {
    san: SanMateo,
}

impl SanMateoService {
    /// Wraps a service.
    #[must_use]
    pub const fn new(san: SanMateo) -> Self {
        Self { san }
    }

    /// The service, if the plugin started.
    #[must_use]
    pub fn from_state(state: &AppState) -> Option<Self> {
        state.extension::<Self>().map(|s| (*s).clone())
    }

    /// The service.
    ///
    /// # Errors
    /// Returns a 503 [`AutumnError`] when the plugin did not start.
    pub fn require(state: &AppState) -> AutumnResult<Self> {
        Self::from_state(state).ok_or_else(|| AutumnError::service_unavailable_msg(NOT_STARTED))
    }

    /// The core service.
    #[must_use]
    pub const fn san(&self) -> &SanMateo {
        &self.san
    }

    /// Returns the core service.
    #[must_use]
    pub fn into_inner(self) -> SanMateo {
        self.san
    }
}

impl Deref for SanMateoService {
    type Target = SanMateo;

    fn deref(&self) -> &SanMateo {
        &self.san
    }
}

impl FromRequestParts<AppState> for SanMateoService {
    type Rejection = AutumnError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let _ = parts;
        Self::require(state)
    }
}

/// Converts a San Mateo error to an [`AutumnError`] with the same HTTP
/// status. Use it with `?` in autumn handlers.
#[must_use]
pub fn autumn_error(err: san_mateo::Error) -> AutumnError {
    let status = StatusCode::from_u16(err.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match err {
        // Internal details stay in the log, as in the core API.
        san_mateo::Error::Store(_) | san_mateo::Error::Internal(_) => {
            tracing::error!(code = err.code(), "san mateo call failed");
            AutumnError::internal_server_error_msg("an internal error occurred")
        }
        other => AutumnError::from(other).with_status(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::service;

    #[test]
    fn from_state_and_require() {
        let state = AppState::for_test();
        assert!(SanMateoService::from_state(&state).is_none());
        let e = SanMateoService::require(&state).unwrap_err();
        assert_eq!(e.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(e.message().contains(NOT_STARTED), "{}", e.message());
        state.insert_extension(SanMateoService::new(service()));
        let s = SanMateoService::require(&state).unwrap();
        assert_eq!(s.config().server_url, "https://sm.example.com");
        assert_eq!(s.san().config().environments.len(), 1);
        assert!(SanMateoService::from_state(&state).is_some());
        assert_eq!(s.into_inner().catalog().len(), 0);
    }

    #[tokio::test]
    async fn extractor_gives_503_before_start() {
        use autumn_web::reexports::http::Request;
        let state = AppState::for_test();
        let (mut parts, ()) = Request::new(()).into_parts();
        let e = SanMateoService::from_request_parts(&mut parts, &state)
            .await
            .unwrap_err();
        assert_eq!(e.status(), StatusCode::SERVICE_UNAVAILABLE);
        state.insert_extension(SanMateoService::new(service()));
        assert!(
            SanMateoService::from_request_parts(&mut parts, &state)
                .await
                .is_ok()
        );
    }

    #[test]
    fn errors_keep_their_status() {
        use san_mateo::Error;
        for (err, status) in [
            (Error::Unauthorized("x".into()), 401),
            (Error::UnknownConnection("c".into()), 404),
            (Error::Invalid("bad".into()), 400),
            (Error::Conflict("dup".into()), 409),
            (Error::Upstream("down".into()), 502),
            (Error::Internal("db detail".into()), 500),
        ] {
            let e = autumn_error(err);
            assert_eq!(e.status().as_u16(), status, "{}", e.message());
        }
        let e = autumn_error(Error::Invalid("bad input".into()));
        assert!(e.message().contains("bad input"), "{}", e.message());
        // Internal details stay in the log.
        let e = autumn_error(Error::Store("db detail".into()));
        assert!(!e.message().contains("db detail"), "{}", e.message());
    }
}
