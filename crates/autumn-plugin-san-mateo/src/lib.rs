//! San Mateo plugin for autumn-web.
//!
//! The plugin installs San Mateo into an autumn app with one call. San Mateo
//! keeps OAuth tokens and API keys, refreshes tokens, sends authorized calls
//! through a proxy, and runs syncs and actions.
//!
//! - **API:** the San Mateo REST API under a route prefix (default
//!   `/san-mateo`). autumn lists the routes in `autumn routes`.
//! - **Config:** the `[san_mateo]` section. It holds env var names, never
//!   secrets.
//! - **State:** [`SanMateoService`] gives handlers and jobs the service.
//! - **Operations:** a `san_mateo` health indicator, background loops, and a
//!   drain at shutdown.
//!
//! Notes:
//!
//! - autumn needs one or more routes of the app. The nested San Mateo router
//!   does not count.
//! - With `[security.csrf] enabled = true`, add the prefix to `exempt_paths`.
//!   San Mateo clients send tokens, not cookies. Else the startup fails and
//!   the error gives the fix.
//! - The hosted connect page needs `[security.headers.csp_nonce]
//!   enabled = true`. The plugin then puts the autumn nonce in the page.
//! - At the root prefix (`"/"`), the plugin merges its router. `autumn routes
//!   audit` cannot prove a merged router, so the audit reports it.
//! - The background loops start in the startup hook, in roles that run
//!   workers. When `san-mateo-harvest` runs the syncs, set
//!   `[san_mateo.background] run_syncs = false`. Else each sync runs two
//!   times.
//! - With `.service(san)`, the `server_url` of `san` must end with the
//!   prefix. Else OAuth callbacks give 404. The plugin logs a warning.
//!
//! ```rust,no_run
//! use autumn_plugin_san_mateo::SanMateoPlugin;
//!
//! # async fn demo() {
//! autumn_web::app()
//!     .plugin(SanMateoPlugin::new().prefix("/san-mateo"))
//!     .run()
//!     .await;
//! # }
//! ```

pub mod config;
mod error;
mod health;
mod plugin;
mod routes;
mod runtime;
mod service;
#[cfg(test)]
mod test_support;

pub use config::SanMateoConfig;
pub use error::PluginError;
pub use health::{INDICATOR_NAME, SanMateoHealth};
pub use plugin::{
    AUTUMN_WEB_REQUIREMENT, DEFAULT_PREFIX, PLUGIN_NAME, SanMateoPlugin, SanMateoRuntime,
    plugin_contract,
};
pub use san_mateo;
pub use service::{SanMateoService, autumn_error};
