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
use std::sync::Arc;
use std::time::Duration;

use san_mateo::{
    BackgroundSettings, Catalog, EnvironmentConfig, Error, KeyRing, MemoryStore, ProxySettings,
    Result, SanMateo, Secret, Store, WebhookSettings,
};
use san_mateo_postgres::PgStore;

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
        let var = |name: &str| {
            get(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let bad = |name: &str, why: &str| Error::Invalid(format!("{name}: {why}"));
        let flag = |name: &str, default: bool| -> Result<bool> {
            match var(name).as_deref() {
                None => Ok(default),
                Some("1" | "true" | "yes" | "on") => Ok(true),
                Some("0" | "false" | "no" | "off") => Ok(false),
                Some(_) => Err(bad(name, "must be true or false")),
            }
        };
        let list = |name: &str| -> Vec<String> {
            var(name)
                .map(|v| {
                    v.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };

        let key = var("SAN_MATEO_ENCRYPTION_KEY")
            .ok_or_else(|| bad("SAN_MATEO_ENCRYPTION_KEY", "is required"))?;
        let key_id = var("SAN_MATEO_ENCRYPTION_KEY_ID").unwrap_or_else(|| "k1".to_owned());
        KeyRing::from_base64(&key_id, &key).map_err(|_| {
            bad(
                "SAN_MATEO_ENCRYPTION_KEY",
                "must be a base64 32-byte key with a valid id",
            )
        })?;
        let mut old_keys = Vec::new();
        for item in list("SAN_MATEO_OLD_ENCRYPTION_KEYS") {
            let (id, k) = item.split_once(':').ok_or_else(|| {
                bad(
                    "SAN_MATEO_OLD_ENCRYPTION_KEYS",
                    "each item must be id:base64",
                )
            })?;
            KeyRing::from_base64(id, k).map_err(|_| {
                bad(
                    "SAN_MATEO_OLD_ENCRYPTION_KEYS",
                    "each key must be a base64 32-byte key",
                )
            })?;
            old_keys.push((id.to_owned(), k.to_owned()));
        }

        let env_ids = match var("SAN_MATEO_ENVIRONMENTS") {
            Some(_) => list("SAN_MATEO_ENVIRONMENTS"),
            None => vec!["dev".to_owned()],
        };
        if env_ids.is_empty() {
            return Err(bad("SAN_MATEO_ENVIRONMENTS", "has no environment id"));
        }
        let mut environments = Vec::new();
        for id in env_ids {
            let p = format!("SAN_MATEO_{}_", id.to_ascii_uppercase().replace('-', "_"));
            let secret_name = format!("{p}SECRET_KEY");
            let secret = var(&secret_name).ok_or_else(|| bad(&secret_name, "is required"))?;
            let mut env = EnvironmentConfig::new(id, secret);
            env.callback_url = var(&format!("{p}CALLBACK_URL"));
            env.webhooks = WebhookSettings {
                primary_url: var(&format!("{p}WEBHOOK_URL")),
                secondary_url: var(&format!("{p}WEBHOOK_URL_SECONDARY")),
                signing_secret: var(&format!("{p}WEBHOOK_SECRET")).map(Secret::new),
                ..WebhookSettings::default()
            };
            environments.push(env);
        }

        let bind_text = var("SAN_MATEO_BIND").unwrap_or_else(|| "0.0.0.0:3003".to_owned());
        let bind = bind_text
            .parse()
            .map_err(|_| bad("SAN_MATEO_BIND", "must be an address such as 0.0.0.0:3003"))?;
        let tick_secs = match var("SAN_MATEO_TICK_SECS") {
            Some(t) => t
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| bad("SAN_MATEO_TICK_SECS", "must be a positive whole number"))?,
            None => 30,
        };
        let service = san_mateo::Config {
            server_url: var("SAN_MATEO_SERVER_URL")
                .unwrap_or_else(|| "http://localhost:3003".to_owned()),
            connect_url: var("SAN_MATEO_CONNECT_URL"),
            environments,
            proxy: ProxySettings {
                allowed_hosts: list("SAN_MATEO_ALLOWED_HOSTS"),
                allow_private_hosts: flag("SAN_MATEO_ALLOW_PRIVATE_HOSTS", false)?,
                allow_plain_http: flag("SAN_MATEO_ALLOW_PLAIN_HTTP", false)?,
                ..ProxySettings::default()
            },
            ..san_mateo::Config::default()
        };
        Ok(Self {
            service,
            bind,
            database_url: var("SAN_MATEO_DATABASE_URL"),
            encryption_key: (key_id, key),
            old_keys,
            lookup_key_id: var("SAN_MATEO_LOOKUP_KEY_ID"),
            providers_file: var("SAN_MATEO_PROVIDERS_FILE"),
            background: flag("SAN_MATEO_BACKGROUND", true)?,
            tick_secs,
        })
    }

    /// Builds the key ring.
    fn key_ring(&self) -> Result<KeyRing> {
        let mut keys = KeyRing::from_base64(&self.encryption_key.0, &self.encryption_key.1)?;
        for (id, k) in &self.old_keys {
            keys = keys.with_base64_key(id, k)?;
        }
        match &self.lookup_key_id {
            Some(id) => keys.lookup_with(id),
            None => Ok(keys),
        }
    }
}

/// Builds the service from the settings. It runs the migrations when a
/// database is set.
///
/// # Errors
///
/// Fails when the key, the catalog file or the database is not valid.
pub async fn build(config: &ServerConfig) -> Result<SanMateo> {
    let mut catalog = Catalog::builtin()?;
    if let Some(path) = &config.providers_file {
        catalog = catalog.merge(Catalog::from_yaml_file(path)?);
    }
    let store: Arc<dyn Store> = match &config.database_url {
        Some(url) => {
            let pg = PgStore::connect(url, 16)?;
            let applied = pg.migrate().await?;
            tracing::info!(count = applied.len(), "database migrations applied");
            Arc::new(pg)
        }
        None => {
            tracing::warn!("no SAN_MATEO_DATABASE_URL: data stays in memory and is lost at exit");
            Arc::new(MemoryStore::new())
        }
    };
    SanMateo::builder()
        .config(config.service.clone())
        .catalog(catalog)
        .store(store)
        .key_ring(config.key_ring()?)
        .build()
}

/// Returns the HTTP app: the API with a body limit.
pub fn app(san: SanMateo) -> axum::Router {
    san_mateo::api::router(san).layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| Error::Internal(format!("server: {e}")))
}

/// Builds the service, starts the background loops, and serves until
/// `shutdown` ends. Then it stops the loops and waits for webhooks.
///
/// # Errors
///
/// The errors of [`build`] and [`serve`], and a bind failure.
pub async fn run(
    config: ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let san = build(&config).await?;
    let background = config.background.then(|| {
        san.start_background(BackgroundSettings {
            tick: Duration::from_secs(config.tick_secs),
            ..BackgroundSettings::default()
        })
    });
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|e| Error::Internal(format!("bind {}: {e}", config.bind)))?;
    tracing::info!(addr = %listener.local_addr().map_or_else(|_| config.bind.to_string(), |a| a.to_string()), "san-mateo listens");
    let served = serve(listener, app(san.clone()), shutdown).await;
    if let Some(bg) = background {
        bg.stop().await;
    }
    san.flush().await;
    tracing::info!("san-mateo stopped");
    served
}

/// Ends on Ctrl-C or, on Unix, on `SIGTERM`.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}

/// Starts logs on stderr. `RUST_LOG` sets the level (default `info`).
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
