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
//! | `SAN_MATEO_LOOKUP_KEY_ID` | Key id for lookup hashes. Default: the active key. Required with old keys. |
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

/// A key id and its base64 value.
pub type NamedKey = (String, Secret);

/// Server settings from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// The service settings.
    pub service: san_mateo::Config,
    /// The listen address.
    pub bind: SocketAddr,
    /// The Postgres URL, if any. It can contain a password.
    pub database_url: Option<Secret>,
    /// The active key: (id, base64).
    pub encryption_key: NamedKey,
    /// Old keys: (id, base64).
    pub old_keys: Vec<NamedKey>,
    /// The key id for lookup hashes.
    pub lookup_key_id: Option<String>,
    /// An extra catalog file.
    pub providers_file: Option<String>,
    /// Run the background loops.
    pub background: bool,
    /// Seconds between background ticks.
    pub tick_secs: u64,
}

/// Reads and checks variables. Errors name the variable, never the value.
struct Vars<F>(F);

impl<F: Fn(&str) -> Option<String>> Vars<F> {
    fn get(&self, name: &str) -> Option<String> {
        (self.0)(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    fn required(&self, name: &str) -> Result<String> {
        self.get(name).ok_or_else(|| bad(name, "is required"))
    }

    fn flag(&self, name: &str, default: bool) -> Result<bool> {
        match self.get(name).as_deref() {
            None => Ok(default),
            Some("1" | "true" | "yes" | "on") => Ok(true),
            Some("0" | "false" | "no" | "off") => Ok(false),
            Some(_) => Err(bad(name, "must be true or false")),
        }
    }

    fn list(&self, name: &str) -> Option<Vec<String>> {
        self.get(name).map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
    }

    fn keys(&self) -> Result<(NamedKey, Vec<NamedKey>)> {
        let name = "SAN_MATEO_ENCRYPTION_KEY";
        let key = self.required(name)?;
        let id = self
            .get("SAN_MATEO_ENCRYPTION_KEY_ID")
            .unwrap_or_else(|| "k1".to_owned());
        KeyRing::from_base64(&id, &key)
            .map_err(|_| bad(name, "must be a base64 32-byte key with a valid id"))?;
        let name = "SAN_MATEO_OLD_ENCRYPTION_KEYS";
        let mut old = Vec::new();
        for item in self.list(name).unwrap_or_default() {
            let (old_id, k) = item
                .split_once(':')
                .ok_or_else(|| bad(name, "each item must be id:base64"))?;
            KeyRing::from_base64(old_id, k)
                .map_err(|_| bad(name, "each key must be a base64 32-byte key"))?;
            old.push((old_id.to_owned(), Secret::new(k)));
        }
        Ok(((id, Secret::new(key)), old))
    }

    fn environments(&self) -> Result<Vec<EnvironmentConfig>> {
        let ids = self
            .list("SAN_MATEO_ENVIRONMENTS")
            .unwrap_or_else(|| vec!["dev".to_owned()]);
        if ids.is_empty() {
            return Err(bad("SAN_MATEO_ENVIRONMENTS", "has no environment id"));
        }
        ids.into_iter()
            .map(|id| {
                let p = format!("SAN_MATEO_{}_", id.to_ascii_uppercase().replace('-', "_"));
                let mut env = EnvironmentConfig::new(id, self.required(&format!("{p}SECRET_KEY"))?);
                env.callback_url = self.get(&format!("{p}CALLBACK_URL"));
                env.webhooks = WebhookSettings {
                    primary_url: self.get(&format!("{p}WEBHOOK_URL")),
                    secondary_url: self.get(&format!("{p}WEBHOOK_URL_SECONDARY")),
                    signing_secret: self.get(&format!("{p}WEBHOOK_SECRET")).map(Secret::new),
                    ..WebhookSettings::default()
                };
                Ok(env)
            })
            .collect()
    }
}

fn bad(name: &str, why: &str) -> Error {
    Error::Invalid(format!("{name}: {why}"))
}

impl ServerConfig {
    /// Reads the settings with `get`, for example `|k| std::env::var(k).ok()`.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Invalid`] that names the variable that is missing
    /// or wrong. The error never contains the value.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let v = Vars(get);
        let (encryption_key, old_keys) = v.keys()?;
        let bind = v
            .get("SAN_MATEO_BIND")
            .unwrap_or_else(|| "0.0.0.0:3003".to_owned())
            .parse()
            .map_err(|_| bad("SAN_MATEO_BIND", "must be an address such as 0.0.0.0:3003"))?;
        let tick_secs = v.get("SAN_MATEO_TICK_SECS").map_or(Ok(30), |t| {
            t.parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| bad("SAN_MATEO_TICK_SECS", "must be a positive whole number"))
        })?;
        let service = san_mateo::Config {
            server_url: v
                .get("SAN_MATEO_SERVER_URL")
                .unwrap_or_else(|| "http://localhost:3003".to_owned()),
            connect_url: v.get("SAN_MATEO_CONNECT_URL"),
            environments: v.environments()?,
            proxy: ProxySettings {
                allowed_hosts: v.list("SAN_MATEO_ALLOWED_HOSTS").unwrap_or_default(),
                allow_private_hosts: v.flag("SAN_MATEO_ALLOW_PRIVATE_HOSTS", false)?,
                allow_plain_http: v.flag("SAN_MATEO_ALLOW_PLAIN_HTTP", false)?,
                ..ProxySettings::default()
            },
            ..san_mateo::Config::default()
        };
        let config = Self {
            service,
            bind,
            database_url: v.get("SAN_MATEO_DATABASE_URL").map(Secret::new),
            encryption_key,
            old_keys,
            lookup_key_id: v.get("SAN_MATEO_LOOKUP_KEY_ID"),
            providers_file: v.get("SAN_MATEO_PROVIDERS_FILE"),
            background: v.flag("SAN_MATEO_BACKGROUND", true)?,
            tick_secs,
        };
        config.check_keys()?;
        Ok(config)
    }

    /// Checks that key ids are unique and that the lookup key id names a key.
    fn check_keys(&self) -> Result<()> {
        let mut ids: Vec<&str> = self.old_keys.iter().map(|(id, _)| id.as_str()).collect();
        ids.push(&self.encryption_key.0);
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(bad(
                "SAN_MATEO_OLD_ENCRYPTION_KEYS",
                "key ids must be unique",
            ));
        }
        // With old keys, the lookup key must stay on the first key. A
        // default would follow the new active key and break live sessions.
        match &self.lookup_key_id {
            None if !self.old_keys.is_empty() => Err(bad(
                "SAN_MATEO_LOOKUP_KEY_ID",
                "is required with old keys: use the id of the first key",
            )),
            Some(id) if !ids.contains(&id.as_str()) => {
                Err(bad("SAN_MATEO_LOOKUP_KEY_ID", "must name a configured key"))
            }
            _ => Ok(()),
        }
    }

    /// Builds the key ring.
    fn key_ring(&self) -> Result<KeyRing> {
        let (id, key) = &self.encryption_key;
        let mut keys = KeyRing::from_base64(id, key.expose())?;
        for (id, k) in &self.old_keys {
            keys = keys.with_base64_key(id, k.expose())?;
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
    let store: Arc<dyn Store> = if let Some(url) = &config.database_url {
        let pg = PgStore::connect(url.expose(), 16)?;
        let applied = pg.migrate().await?;
        tracing::info!(count = applied.len(), "database migrations applied");
        Arc::new(pg)
    } else {
        tracing::warn!("no SAN_MATEO_DATABASE_URL: data stays in memory and is lost at exit");
        Arc::new(MemoryStore::new())
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

/// Runs the binary: reads the settings with `get`, then runs until
/// `shutdown` ends. Returns the exit code: 0 after a clean stop, 1 when the
/// server fails, 2 when the settings are wrong.
pub async fn main_exit_code(
    get: impl Fn(&str) -> Option<String>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> u8 {
    let config = match ServerConfig::from_env(get) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("san-mateo: {e}");
            return 2;
        }
    };
    match run(config, shutdown).await {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!(code = e.code(), error = %e, "the server stopped with an error");
            1
        }
    }
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
