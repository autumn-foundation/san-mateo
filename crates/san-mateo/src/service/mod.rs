//! The San Mateo service. It ties the catalog, the store, the key ring, the
//! transport, the functions and the webhooks together.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio_util::task::TaskTracker;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::clock::{Clock, Entropy, OsEntropy, SystemClock};
use crate::credentials::Credentials;
use crate::crypto::KeyRing;
use crate::error::{Error, Result};
use crate::functions::{ActionFunction, Registry, SyncFunction};
use crate::model::{ConnectionKey, EndUser, IntegrationDefaults, Organization, Tags};
use crate::provider::Catalog;
use crate::proxy::ProxySettings;
use crate::secret::Secret;
use crate::store::{MemoryStore, Store, SyncCounts, SyncStatus};
use crate::transport::{HttpTransport, ReqwestTransport, TransportConfig};
use crate::webhook::{WebhookEvent, WebhookSender, WebhookSettings};

mod connect;
mod connections;
mod functions;
mod integrations;
mod proxying;
mod records;

/// The prefix of a connect session token.
pub const CONNECT_TOKEN_PREFIX: &str = "smcs_";

/// One environment, for example `dev` or `prod`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentConfig {
    /// The environment id.
    pub id: String,
    /// The secret key for the API.
    pub secret_key: Secret,
    /// The OAuth callback URL. Default: `<server_url>/oauth/callback`.
    pub callback_url: Option<String>,
    /// Webhook settings.
    pub webhooks: WebhookSettings,
}

impl EnvironmentConfig {
    /// Makes an environment with no webhooks.
    #[must_use]
    pub fn new(id: impl Into<String>, secret_key: impl Into<Secret>) -> Self {
        Self {
            id: id.into(),
            secret_key: secret_key.into(),
            callback_url: None,
            webhooks: WebhookSettings::default(),
        }
    }
}

/// Service settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The public URL of the API, for example `https://sm.example.com`.
    pub server_url: String,
    /// The base of `connect_link`. Default: `<server_url>/connect`.
    pub connect_url: Option<String>,
    /// The environments.
    pub environments: Vec<EnvironmentConfig>,
    /// Proxy settings.
    pub proxy: ProxySettings,
    /// Connect session life. Default 30 minutes.
    pub connect_session_ttl: Duration,
    /// OAuth session life. Default 15 minutes.
    pub oauth_session_ttl: Duration,
    /// Refresh lease life. Default 10 seconds.
    pub refresh_lease_ttl: Duration,
    /// The longest wait for the lease of another process. Default 12 seconds.
    pub refresh_lease_wait: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:3003".to_owned(),
            connect_url: None,
            environments: Vec::new(),
            proxy: ProxySettings::default(),
            connect_session_ttl: Duration::from_secs(30 * 60),
            oauth_session_ttl: Duration::from_secs(15 * 60),
            refresh_lease_ttl: Duration::from_secs(10),
            refresh_lease_wait: Duration::from_secs(12),
        }
    }
}

/// An integration as the API shows it. The client secret is never shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integration {
    /// The integration key.
    pub unique_key: String,
    /// The provider name.
    pub provider: String,
    /// The name to show.
    pub display_name: String,
    /// The OAuth client id.
    pub client_id: Option<String>,
    /// `true` when a client secret is set.
    pub has_client_secret: bool,
    /// The scopes.
    pub scopes: Vec<String>,
    /// Free-form config.
    pub custom: BTreeMap<String, String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last change time.
    pub updated_at: DateTime<Utc>,
}

/// Input for a new integration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewIntegration {
    /// The integration key.
    pub unique_key: String,
    /// The provider name in the catalog.
    pub provider: String,
    /// The name to show. Default: the provider name.
    pub display_name: Option<String>,
    /// The OAuth client id.
    pub client_id: Option<String>,
    /// The OAuth client secret.
    pub client_secret: Option<Secret>,
    /// The scopes.
    pub scopes: Vec<String>,
    /// Free-form config.
    pub custom: BTreeMap<String, String>,
}

/// Changes to an integration. `None` keeps the value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntegrationPatch {
    /// The name to show.
    pub display_name: Option<String>,
    /// The OAuth client id.
    pub client_id: Option<String>,
    /// The OAuth client secret.
    pub client_secret: Option<Secret>,
    /// The scopes.
    pub scopes: Option<Vec<String>>,
    /// Free-form config.
    pub custom: Option<BTreeMap<String, String>>,
}

/// Input for a connect session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectSessionInput {
    /// The end user.
    pub end_user: Option<EndUser>,
    /// The organization.
    pub organization: Option<Organization>,
    /// Allowed integrations. Empty means all.
    pub allowed_integrations: Vec<String>,
    /// Defaults by integration key.
    pub integrations_config_defaults: BTreeMap<String, IntegrationDefaults>,
    /// Tags for the new connection.
    pub tags: Tags,
    /// A webhook URL for the new connection.
    pub webhook_url_override: Option<String>,
}

/// A new connect session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectSessionCreated {
    /// The token. Give it to the end user only.
    pub token: Secret,
    /// The link to the connect page.
    pub connect_link: String,
    /// Expiry time.
    pub expires_at: DateTime<Utc>,
}

/// A live connect session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectSession {
    /// The id.
    pub id: Uuid,
    /// The environment id.
    pub environment: String,
    /// The end user.
    pub end_user: Option<EndUser>,
    /// The organization.
    pub organization: Option<Organization>,
    /// Allowed integrations. Empty means all.
    pub allowed_integrations: Vec<String>,
    /// Defaults by integration key.
    pub integrations_config_defaults: BTreeMap<String, IntegrationDefaults>,
    /// Tags for the new connection.
    pub tags: Tags,
    /// A webhook URL for the new connection.
    pub webhook_url_override: Option<String>,
    /// The connection to reconnect, if any.
    pub reconnect: Option<ConnectionKey>,
    /// Expiry time.
    pub expires_at: DateTime<Utc>,
}

/// Options for the start of an OAuth flow and for other connect calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectOptions {
    /// Connection config values from the end user.
    pub connection_config: Map<String, Value>,
    /// Authorization parameters from the end user.
    pub authorization_params: BTreeMap<String, String>,
    /// Scopes from the end user, comma separated.
    pub user_scopes: Option<String>,
}

/// A connection with its credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// The internal id.
    pub id: Uuid,
    /// The environment id.
    pub environment: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
    /// The provider name.
    pub provider: String,
    /// The credentials.
    pub credentials: Credentials,
    /// Connection config.
    pub connection_config: Map<String, Value>,
    /// Metadata.
    pub metadata: Map<String, Value>,
    /// Tags.
    pub tags: Tags,
    /// The end user.
    pub end_user: Option<EndUser>,
    /// A webhook URL that replaces the environment URLs.
    pub webhook_url_override: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last change time.
    pub updated_at: DateTime<Utc>,
    /// Last time a caller read the credentials.
    pub last_fetched_at: Option<DateTime<Utc>>,
}

impl Connection {
    /// Returns the connection key.
    #[must_use]
    pub fn key(&self) -> ConnectionKey {
        ConnectionKey::new(&self.provider_config_key, &self.connection_id)
    }
}

/// A connection with no credentials, for lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSummary {
    /// The internal id.
    pub id: Uuid,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
    /// The provider name.
    pub provider: String,
    /// Metadata.
    pub metadata: Map<String, Value>,
    /// Tags.
    pub tags: Tags,
    /// The end user.
    pub end_user: Option<EndUser>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// `true` when token refresh stopped.
    pub refresh_exhausted: bool,
}

/// Options for [`SanMateo::get_connection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetOptions {
    /// Refresh when the token expires soon. Default `true`.
    pub refresh_if_needed: bool,
    /// Refresh now, also when the token is fresh, in cooldown or exhausted.
    pub force_refresh: bool,
}

impl Default for GetOptions {
    fn default() -> Self {
        Self {
            refresh_if_needed: true,
            force_refresh: false,
        }
    }
}

/// Input to import a connection with known credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportConnection {
    /// The connection id. Default: a new UUID.
    pub connection_id: Option<String>,
    /// The integration key.
    pub provider_config_key: String,
    /// The credentials.
    pub credentials: Credentials,
    /// Connection config.
    pub connection_config: Map<String, Value>,
    /// Metadata.
    pub metadata: Map<String, Value>,
    /// Tags.
    pub tags: Tags,
    /// The end user.
    pub end_user: Option<EndUser>,
    /// A webhook URL that replaces the environment URLs.
    pub webhook_url_override: Option<String>,
}

/// Changes to a connection. `None` keeps the value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionPatch {
    /// The end user.
    pub end_user: Option<EndUser>,
    /// Tags.
    pub tags: Option<Tags>,
    /// The webhook URL. `Some(None)` removes it.
    #[allow(clippy::option_option)]
    pub webhook_url_override: Option<Option<String>>,
}

/// The result of [`SanMateo::refresh_due`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshReport {
    /// Connections with new tokens.
    pub refreshed: usize,
    /// Connections with a failed refresh.
    pub failed: usize,
    /// Connections that another process refreshed, or that are in cooldown.
    pub skipped: usize,
}

/// A query for [`SanMateo::list_records`], as the API gives it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordsRequest {
    /// The model name.
    pub model: String,
    /// The cursor from the last page.
    pub cursor: Option<String>,
    /// Return records changed at or after this RFC 3339 time.
    pub modified_after: Option<String>,
    /// The page size, 1 to 10,000. Default 100.
    pub limit: Option<u32>,
    /// A filter such as `added,updated`.
    pub filter: Option<String>,
    /// Only these external ids (100 or fewer).
    pub ids: Vec<String>,
}

/// One page of records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordPage {
    /// The records, each with `_nango_metadata`.
    pub records: Vec<Value>,
    /// The cursor of the next page, if one exists.
    pub next_cursor: Option<String>,
}

/// The result of one sync run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRunReport {
    /// The sync name.
    pub sync_name: String,
    /// The connection.
    pub connection: ConnectionKey,
    /// `true` when the run succeeded.
    pub success: bool,
    /// Counts of the run.
    pub counts: SyncCounts,
    /// The error, when it failed.
    pub error: Option<String>,
}

/// The state of one sync for one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatusEntry {
    /// The sync name.
    pub name: String,
    /// The connection id.
    pub connection_id: String,
    /// The status.
    pub status: SyncStatus,
    /// The frequency.
    pub frequency: Duration,
    /// End of the last run.
    pub finished_at: Option<DateTime<Utc>>,
    /// Time of the next scheduled run.
    pub next_scheduled_at: Option<DateTime<Utc>>,
    /// Counts of the last run.
    pub latest_result: SyncCounts,
}

/// Builds a [`SanMateo`].
pub struct SanMateoBuilder {
    config: Config,
    catalog: Option<Catalog>,
    store: Option<Arc<dyn Store>>,
    keys: Option<KeyRing>,
    transport: Option<Arc<dyn HttpTransport>>,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
    registry: Registry,
}

impl std::fmt::Debug for SanMateoBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SanMateoBuilder")
            .field("server_url", &self.config.server_url)
            .finish_non_exhaustive()
    }
}

impl Default for SanMateoBuilder {
    fn default() -> Self {
        Self {
            config: Config::default(),
            catalog: None,
            store: None,
            keys: None,
            transport: None,
            clock: Arc::new(SystemClock),
            entropy: Arc::new(OsEntropy),
            registry: Registry::default(),
        }
    }
}

impl SanMateoBuilder {
    /// Sets all settings.
    #[must_use]
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Sets the public URL of the API.
    #[must_use]
    pub fn server_url(mut self, url: impl Into<String>) -> Self {
        self.config.server_url = url.into();
        self
    }

    /// Adds an environment.
    #[must_use]
    pub fn environment(mut self, env: EnvironmentConfig) -> Self {
        self.config.environments.push(env);
        self
    }

    /// Sets the proxy settings.
    #[must_use]
    pub fn proxy_settings(mut self, settings: ProxySettings) -> Self {
        self.config.proxy = settings;
        self
    }

    /// Sets the catalog. Default: the built-in catalog.
    #[must_use]
    pub fn catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Sets the store. Default: a [`crate::store::MemoryStore`].
    #[must_use]
    pub fn store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Sets the key ring. Required.
    #[must_use]
    pub fn key_ring(mut self, keys: KeyRing) -> Self {
        self.keys = Some(keys);
        self
    }

    /// Sets the transport. Default: a [`crate::transport::ReqwestTransport`].
    #[must_use]
    pub fn transport(mut self, transport: Arc<dyn HttpTransport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Sets the clock. Default: the system clock.
    #[must_use]
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Sets the random source. Default: the operating system.
    #[must_use]
    pub fn entropy(mut self, entropy: Arc<dyn Entropy>) -> Self {
        self.entropy = entropy;
        self
    }

    /// Adds a sync.
    #[must_use]
    pub fn sync(mut self, function: impl SyncFunction) -> Self {
        self.registry = self.registry.sync(function);
        self
    }

    /// Adds an action.
    #[must_use]
    pub fn action(mut self, function: impl ActionFunction) -> Self {
        self.registry = self.registry.action(function);
        self
    }

    /// Sets all functions.
    #[must_use]
    pub fn registry(mut self, registry: Registry) -> Self {
        self.registry = registry;
        self
    }

    /// Builds the service.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when there is no key ring, no environment, an
    /// environment id or secret key that repeats or is empty, or a server
    /// URL that is not valid.
    pub fn build(self) -> Result<SanMateo> {
        let keys = self.keys.ok_or_else(|| invalid("a key ring is required"))?;
        let config = self.config;
        if config.environments.is_empty() {
            return Err(invalid("at least one environment is required"));
        }
        url::Url::parse(&config.server_url).map_err(|_| invalid("the server URL is not valid"))?;
        let mut env_by_hash = BTreeMap::new();
        let mut ids = std::collections::BTreeSet::new();
        for (i, env) in config.environments.iter().enumerate() {
            if env.id.is_empty() || !ids.insert(env.id.clone()) {
                return Err(invalid("each environment needs a unique, non-empty id"));
            }
            if env.secret_key.is_empty() {
                return Err(invalid("each environment needs a secret key"));
            }
            if env_by_hash
                .insert(keys.lookup_hash(env.secret_key.expose()), i)
                .is_some()
            {
                return Err(invalid("two environments have the same secret key"));
            }
        }
        let catalog = match self.catalog {
            Some(c) => c,
            None => Catalog::builtin()?,
        };
        let transport: Arc<dyn HttpTransport> = match self.transport {
            Some(t) => t,
            None => Arc::new(
                ReqwestTransport::new(TransportConfig::default())
                    .map_err(|e| Error::Internal(format!("transport: {e}")))?,
            ),
        };
        let owner = self.entropy.uuid().to_string();
        Ok(SanMateo {
            inner: Arc::new(Inner {
                config,
                catalog,
                store: self.store.unwrap_or_else(|| Arc::new(MemoryStore::new())),
                keys,
                webhooks: WebhookSender::new(transport.clone()),
                transport,
                clock: self.clock,
                entropy: self.entropy,
                registry: self.registry,
                env_by_hash,
                flights: std::sync::Mutex::new(HashMap::new()),
                running_syncs: std::sync::Mutex::new(HashSet::new()),
                tasks: TaskTracker::new(),
                owner,
            }),
        })
    }
}

pub(crate) struct Inner {
    pub(crate) config: Config,
    pub(crate) catalog: Catalog,
    pub(crate) store: Arc<dyn Store>,
    pub(crate) keys: KeyRing,
    pub(crate) transport: Arc<dyn HttpTransport>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) entropy: Arc<dyn Entropy>,
    pub(crate) registry: Registry,
    pub(crate) webhooks: WebhookSender,
    /// Environment index by the lookup hash of its secret key.
    pub(crate) env_by_hash: BTreeMap<String, usize>,
    /// One refresh lock for each connection, for single flight.
    pub(crate) flights: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Syncs that run now, by (connection, sync name).
    pub(crate) running_syncs: std::sync::Mutex<HashSet<(Uuid, String)>>,
    /// Background tasks: webhooks and triggered syncs.
    pub(crate) tasks: TaskTracker,
    /// The lease owner id of this process.
    pub(crate) owner: String,
}

/// The San Mateo service. Clone it freely: clones share state.
#[derive(Clone)]
pub struct SanMateo {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for SanMateo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SanMateo")
            .field("server_url", &self.inner.config.server_url)
            .field(
                "environments",
                &self
                    .inner
                    .config
                    .environments
                    .iter()
                    .map(|e| &e.id)
                    .collect::<Vec<_>>(),
            )
            .field("providers", &self.inner.catalog.len())
            .finish_non_exhaustive()
    }
}

impl SanMateo {
    /// Returns a builder.
    #[must_use]
    pub fn builder() -> SanMateoBuilder {
        SanMateoBuilder::default()
    }

    /// Returns the catalog.
    #[must_use]
    pub fn catalog(&self) -> &Catalog {
        &self.inner.catalog
    }

    /// Returns the settings.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// Returns the functions.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.inner.registry
    }

    /// Returns the store.
    #[must_use]
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.inner.store
    }

    /// Returns the environment of a secret key. The lookup uses a keyed hash
    /// and a constant-time compare.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] when no environment has the key.
    pub fn authenticate(&self, secret_key: &str) -> Result<&EnvironmentConfig> {
        let deny = || Error::Unauthorized("the secret key is not valid".to_owned());
        if secret_key.is_empty() {
            return Err(deny());
        }
        let hash = self.inner.keys.lookup_hash(secret_key);
        let env = self
            .inner
            .env_by_hash
            .get(&hash)
            .and_then(|i| self.inner.config.environments.get(*i))
            .ok_or_else(deny)?;
        if crate::crypto::constant_time_eq(
            env.secret_key.expose().as_bytes(),
            secret_key.as_bytes(),
        ) {
            Ok(env)
        } else {
            Err(deny())
        }
    }

    /// Returns an environment.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the environment does not exist.
    pub fn environment(&self, id: &str) -> Result<&EnvironmentConfig> {
        self.inner
            .config
            .environments
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| invalid(format!("unknown environment {id}")))
    }

    /// Waits for background tasks (webhooks and triggered syncs) to end.
    pub async fn flush(&self) {
        let tasks = &self.inner.tasks;
        tasks.close();
        tasks.wait().await;
        tasks.reopen();
    }

    // -------------------------------------------------------------- helpers

    /// Returns the time now, cut to whole microseconds (the store precision).
    pub(crate) fn now(&self) -> DateTime<Utc> {
        let t = self.inner.clock.now();
        DateTime::<Utc>::from_timestamp_micros(t.timestamp_micros()).unwrap_or(t)
    }

    pub(crate) fn now_ms(&self) -> u64 {
        crate::clock::to_ms(self.now())
    }

    /// Returns a URL-safe random token with `bytes` bytes of entropy.
    pub(crate) fn random_token(&self, bytes: usize) -> String {
        use base64::Engine as _;
        let mut buf = vec![0_u8; bytes];
        self.inner.entropy.fill(&mut buf);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
    }

    pub(crate) fn connection_aad(env: &str, key: &str, connection_id: &str) -> Vec<u8> {
        crate::crypto::aad(&["connection", env, key, connection_id])
    }

    pub(crate) fn integration_aad(env: &str, key: &str) -> Vec<u8> {
        crate::crypto::aad(&["integration", env, key])
    }

    /// Sends an event in the background.
    pub(crate) fn spawn_webhook(
        &self,
        env: &str,
        url_override: Option<String>,
        event: WebhookEvent,
    ) {
        let Ok(settings) = self.environment(env).map(|e| e.webhooks.clone()) else {
            return;
        };
        let sender = self.inner.webhooks.clone();
        self.inner.tasks.spawn(async move {
            sender
                .deliver(&settings, url_override.as_deref(), &event)
                .await;
        });
    }
}

/// Settings for [`SanMateo::start_background`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundSettings {
    /// Time between ticks.
    pub tick: Duration,
    /// Refresh tokens that expire within this time.
    pub refresh_within: Duration,
    /// Largest number of refreshes in one tick.
    pub refresh_batch: usize,
    /// Run the sync scheduler.
    pub run_syncs: bool,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(30),
            refresh_within: Duration::from_secs(30 * 60),
            refresh_batch: 100,
            run_syncs: true,
        }
    }
}

/// Running background loops.
#[derive(Debug)]
pub struct Background {
    stop: tokio_util::sync::CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Background {
    /// Stops the loops and waits for the current tick to end.
    pub async fn stop(mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

pub(crate) fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}
