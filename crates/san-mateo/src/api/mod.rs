//! The REST API as an axum [`Router`].
//!
//! Paths and JSON shapes follow the Nango public API. See `docs/plan.md`
//! section 8. `Authorization: Bearer <secret key>` selects the environment.
//! Connect routes take a connect session token.

use axum::Router;

use crate::service::SanMateo;

/// One route of the API, for route listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteSpec {
    /// The HTTP method, or `ANY`.
    pub method: &'static str,
    /// The path, with `{name}` for path parameters.
    pub path: &'static str,
    /// `true` when the route needs the secret key.
    pub secret_key: bool,
}

/// All routes of the API.
pub const ROUTES: &[RouteSpec] = &[];

/// Returns the API router.
#[must_use]
pub fn router(san: SanMateo) -> Router {
    let _ = san;
    todo!()
}
