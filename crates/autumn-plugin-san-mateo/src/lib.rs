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
