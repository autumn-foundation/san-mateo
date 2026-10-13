//! The San Mateo server: the REST API with Postgres or memory storage.
//!
//! Configure it with environment variables:
//!
//! | Variable | Meaning |
//! |---|---|
//! | `SAN_MATEO_SERVER_URL` | Public URL of the API. Default `http://localhost:3003`. |
//! | `SAN_MATEO_BIND` | Listen address. Default `0.0.0.0:3003`. |
//! | `SAN_MATEO_DATABASE_URL` | Postgres URL. With no URL, data stays in memory. |
//! | `SAN_MATEO_ENCRYPTION_KEY` | Base64 32-byte key. Required. |
//! | `SAN_MATEO_ENCRYPTION_KEY_ID` | Id of that key. Default `k1`. |
//! | `SAN_MATEO_OLD_ENCRYPTION_KEYS` | Old keys for rotation: `id:base64,id:base64`. |
//! | `SAN_MATEO_LOOKUP_KEY_ID` | Key id for lookup hashes. Default: the first key id. |
//! | `SAN_MATEO_ENVIRONMENTS` | Environment ids, comma separated. Default `dev`. |
//! | `SAN_MATEO_<ENV>_SECRET_KEY` | Secret key of each environment. Required. |
//! | `SAN_MATEO_<ENV>_CALLBACK_URL` | OAuth callback URL of the environment. |
//! | `SAN_MATEO_<ENV>_WEBHOOK_URL`, `..._WEBHOOK_URL_SECONDARY` | Webhook URLs. |
//! | `SAN_MATEO_<ENV>_WEBHOOK_SECRET` | Webhook signing secret. |
//! | `SAN_MATEO_PROVIDERS_FILE` | A Nango-format YAML file. It adds to the built-in catalog. |
//! | `SAN_MATEO_ALLOWED_HOSTS` | Extra hosts for `Base-Url-Override`, comma separated. |
//! | `SAN_MATEO_ALLOW_PRIVATE_HOSTS`, `SAN_MATEO_ALLOW_PLAIN_HTTP` | Development only. |
//! | `SAN_MATEO_BACKGROUND` | Run the refresh sweep and the scheduler. Default `true`. |
//! | `SAN_MATEO_TICK_SECS` | Seconds between background ticks. Default `30`. |
//!
//! `<ENV>` is the environment id in upper case, with `-` as `_`.

use std::future::Future;
use std::net::SocketAddr;

use san_mateo::{Result, SanMateo};

/// The largest request body: 25 MiB.
pub const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

/// Server settings from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// The service settings.
    pub service: san_mateo::Config,
    /// The listen address.
    pub bind: SocketAddr,
    /// The Postgres URL, if any.
    pub database_url: Option<String>,
    /// The active key: (id, base64).
    pub encryption_key: (String, String),
    /// Old keys: (id, base64).
    pub old_keys: Vec<(String, String)>,
    /// The key id for lookup hashes.
    pub lookup_key_id: Option<String>,
    /// An extra catalog file.
    pub providers_file: Option<String>,
    /// Run the background loops.
    pub background: bool,
    /// Seconds between background ticks.
    pub tick_secs: u64,
}

impl ServerConfig {
    /// Reads the settings with `get`, for example `|k| std::env::var(k).ok()`.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Invalid`] that names the variable that is missing
    /// or wrong.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let _ = get;
        todo!()
    }
}

/// Builds the service from the settings. It runs the migrations when a
/// database is set.
///
/// # Errors
///
/// Fails when the key, the catalog file or the database is not valid.
pub async fn build(config: &ServerConfig) -> Result<SanMateo> {
    let _ = config;
    todo!()
}

/// Returns the HTTP app: the API with a body limit.
pub fn app(san: SanMateo) -> axum::Router {
    let _ = san;
    todo!()
}

/// Serves `app` on `listener` until `shutdown` ends.
///
/// # Errors
///
/// [`san_mateo::Error::Internal`] when the server fails.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let _ = (listener, app, shutdown);
    todo!()
}

/// Builds the service, starts the background loops, and serves until
/// `shutdown` ends. Then it stops the loops and waits for webhooks.
///
/// # Errors
///
/// The errors of [`build`] and [`serve`], and a bind failure.
pub async fn run(config: ServerConfig, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<()> {
    let _ = (config, shutdown);
    todo!()
}

/// Ends on Ctrl-C or, on Unix, on `SIGTERM`.
pub async fn shutdown_signal() {
    todo!()
}

/// Starts logs on stderr. `RUST_LOG` sets the level (default `info`).
pub fn init_tracing() {
    todo!()
}
