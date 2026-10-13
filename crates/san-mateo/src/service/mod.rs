//! The San Mateo service. It ties the catalog, the store, the key ring, the
//! transport, the functions and the webhooks together.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::clock::{Clock, Entropy, OsEntropy, SystemClock};
use crate::credentials::Credentials;
use crate::crypto::KeyRing;
use crate::error::{Error, Result};
use crate::functions::{ActionFunction, PaginateOptions, Registry, SyncFunction};
use crate::model::{ConnectionKey, EndUser, IntegrationDefaults, Organization, Tags};
use crate::provider::Catalog;
use crate::proxy::{ProxyRequest, ProxyResponse, ProxySettings};
use crate::records::BatchOutcome;
use crate::secret::Secret;
use crate::store::{Store, SyncCounts, SyncStatus};
use crate::transport::HttpTransport;
use crate::webhook::WebhookSettings;

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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectOptions {
    /// Connection config values from the end user.
    pub connection_config: Map<String, Value>,
    /// Authorization parameters from the end user.
    pub authorization_params: BTreeMap<String, String>,
    /// Scopes from the end user, comma separated.
    pub user_scopes: Option<String>,
}

/// A connection with its credentials.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
        let _ = (
            self.config,
            self.catalog,
            self.store,
            self.keys,
            self.transport,
            self.clock,
            self.entropy,
            self.registry,
        );
        todo!()
    }
}

pub(crate) struct Inner {}

/// The San Mateo service. Clone it freely: clones share state.
#[derive(Clone)]
pub struct SanMateo {
    #[allow(dead_code)]
    inner: Arc<Inner>,
}

impl std::fmt::Debug for SanMateo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SanMateo")
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
        todo!()
    }

    /// Returns the settings.
    #[must_use]
    pub fn config(&self) -> &Config {
        todo!()
    }

    /// Returns the functions.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        todo!()
    }

    /// Returns the environment of a secret key.
    ///
    /// # Errors
    ///
    /// [`Error::Unauthorized`] when no environment has the key.
    pub fn authenticate(&self, secret_key: &str) -> Result<&EnvironmentConfig> {
        let _ = secret_key;
        todo!()
    }

    /// Returns an environment.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the environment does not exist.
    pub fn environment(&self, id: &str) -> Result<&EnvironmentConfig> {
        let _ = id;
        todo!()
    }

    // ------------------------------------------------------- integrations

    /// Adds an integration.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownProvider`], [`Error::Conflict`], [`Error::Invalid`].
    pub async fn create_integration(&self, env: &str, input: NewIntegration) -> Result<Integration> {
        let _ = (env, input);
        todo!()
    }

    /// Reads an integration.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`].
    pub async fn get_integration(&self, env: &str, key: &str) -> Result<Integration> {
        let _ = (env, key);
        todo!()
    }

    /// Lists the integrations of an environment.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn list_integrations(&self, env: &str) -> Result<Vec<Integration>> {
        let _ = env;
        todo!()
    }

    /// Changes an integration.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`].
    pub async fn update_integration(
        &self,
        env: &str,
        key: &str,
        patch: IntegrationPatch,
    ) -> Result<Integration> {
        let _ = (env, key, patch);
        todo!()
    }

    /// Deletes an integration and its connections.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`].
    pub async fn delete_integration(&self, env: &str, key: &str) -> Result<()> {
        let _ = (env, key);
        todo!()
    }

    // ---------------------------------------------------- connect sessions

    /// Makes a connect session.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`] for an allowed integration that does not
    /// exist. [`Error::Invalid`] for a webhook URL that is not HTTPS.
    pub async fn create_connect_session(
        &self,
        env: &str,
        input: ConnectSessionInput,
    ) -> Result<ConnectSessionCreated> {
        let _ = (env, input);
        todo!()
    }

    /// Makes a connect session that reconnects one connection.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`] when the connection does not exist.
    pub async fn create_reconnect_session(
        &self,
        env: &str,
        key: &ConnectionKey,
        input: ConnectSessionInput,
    ) -> Result<ConnectSessionCreated> {
        let _ = (env, key, input);
        todo!()
    }

    /// Reads a live connect session.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConnectSession`] when the token is unknown, wrong or expired.
    pub async fn connect_session(&self, token: &str) -> Result<ConnectSession> {
        let _ = token;
        todo!()
    }

    /// Deletes a connect session.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConnectSession`].
    pub async fn delete_connect_session(&self, token: &str) -> Result<()> {
        let _ = token;
        todo!()
    }

    // ---------------------------------------------------------- auth flows

    /// Starts an OAuth2 flow. Returns the provider URL for the browser.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConnectSession`], [`Error::IntegrationNotAllowed`],
    /// [`Error::UnknownIntegration`], [`Error::UnsupportedAuthMode`],
    /// [`Error::MissingField`], [`Error::Template`].
    pub async fn oauth_start(&self, token: &str, integration: &str, options: ConnectOptions) -> Result<String> {
        let _ = (token, integration, options);
        todo!()
    }

    /// Ends an OAuth2 flow with the callback query.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidOAuthState`], [`Error::Provider`] when the provider
    /// gives an error or the code exchange fails.
    pub async fn oauth_callback(&self, query: &BTreeMap<String, String>) -> Result<Connection> {
        let _ = query;
        todo!()
    }

    /// Makes an API key connection.
    ///
    /// # Errors
    ///
    /// Session errors, and [`Error::Provider`] with `verification_failed`.
    pub async fn connect_api_key(
        &self,
        token: &str,
        integration: &str,
        api_key: Secret,
        options: ConnectOptions,
    ) -> Result<Connection> {
        let _ = (token, integration, api_key, options);
        todo!()
    }

    /// Makes a Basic auth connection.
    ///
    /// # Errors
    ///
    /// Session errors, and [`Error::Provider`] with `verification_failed`.
    pub async fn connect_basic(
        &self,
        token: &str,
        integration: &str,
        username: String,
        password: Secret,
        options: ConnectOptions,
    ) -> Result<Connection> {
        let _ = (token, integration, username, password, options);
        todo!()
    }

    /// Makes an OAuth2 client credentials connection.
    ///
    /// # Errors
    ///
    /// Session errors, and [`Error::Provider`] with `token_exchange_failed`.
    pub async fn connect_client_credentials(
        &self,
        token: &str,
        integration: &str,
        client_id: String,
        client_secret: Secret,
        options: ConnectOptions,
    ) -> Result<Connection> {
        let _ = (token, integration, client_id, client_secret, options);
        todo!()
    }

    /// Makes a connection with no auth.
    ///
    /// # Errors
    ///
    /// Session errors.
    pub async fn connect_unauthenticated(
        &self,
        token: &str,
        integration: &str,
        options: ConnectOptions,
    ) -> Result<Connection> {
        let _ = (token, integration, options);
        todo!()
    }

    /// Imports a connection with known credentials.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`], [`Error::Invalid`] when the credentials
    /// do not match the auth mode.
    pub async fn import_connection(&self, env: &str, input: ImportConnection) -> Result<Connection> {
        let _ = (env, input);
        todo!()
    }

    // --------------------------------------------------------- connections

    /// Lists connections with no credentials.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn list_connections(
        &self,
        env: &str,
        filter: &crate::store::ConnectionFilter,
    ) -> Result<Vec<ConnectionSummary>> {
        let _ = (env, filter);
        todo!()
    }

    /// Reads a connection with fresh credentials.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`], [`Error::RefreshCooldown`],
    /// [`Error::RefreshExhausted`], [`Error::Provider`] with `refresh_failed`.
    pub async fn get_connection(&self, env: &str, key: &ConnectionKey, options: GetOptions) -> Result<Connection> {
        let _ = (env, key, options);
        todo!()
    }

    /// Deletes a connection. Sends a `deletion` webhook.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`].
    pub async fn delete_connection(&self, env: &str, key: &ConnectionKey) -> Result<()> {
        let _ = (env, key);
        todo!()
    }

    /// Sets the metadata (`merge = false`) or merges keys into it (`merge = true`).
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`].
    pub async fn set_metadata(
        &self,
        env: &str,
        key: &ConnectionKey,
        metadata: Map<String, Value>,
        merge: bool,
    ) -> Result<Map<String, Value>> {
        let _ = (env, key, metadata, merge);
        todo!()
    }

    /// Changes the end user, tags or webhook URL of a connection.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`].
    pub async fn patch_connection(&self, env: &str, key: &ConnectionKey, patch: ConnectionPatch) -> Result<()> {
        let _ = (env, key, patch);
        todo!()
    }

    /// Refreshes the tokens that expire before `now + within`.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn refresh_due(&self, within: Duration, limit: usize) -> Result<RefreshReport> {
        let _ = (within, limit);
        todo!()
    }

    // --------------------------------------------------------------- proxy

    /// Sends a call to the provider of a connection, with its auth.
    ///
    /// # Errors
    ///
    /// Connection errors, [`Error::ForbiddenHost`], [`Error::Upstream`],
    /// [`Error::UpstreamTooLarge`].
    pub async fn proxy(&self, env: &str, key: &ConnectionKey, request: ProxyRequest) -> Result<ProxyResponse> {
        let _ = (env, key, request);
        todo!()
    }

    /// Reads all pages of a list call and returns all records.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::proxy`], and [`Error::Invalid`] when there is no
    /// pagination config or a page is not a 2xx JSON reply.
    pub async fn paginate(
        &self,
        env: &str,
        key: &ConnectionKey,
        request: ProxyRequest,
        options: PaginateOptions,
    ) -> Result<Vec<Value>> {
        let _ = (env, key, request, options);
        todo!()
    }

    // ------------------------------------------------------------- records

    /// Saves records. Each record must have an `id`. A later record with the
    /// same `id` in one batch replaces an earlier one.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`], [`Error::Invalid`].
    pub async fn save_records(
        &self,
        env: &str,
        key: &ConnectionKey,
        model: &str,
        records: Vec<Value>,
    ) -> Result<BatchOutcome> {
        let _ = (env, key, model, records);
        todo!()
    }

    /// Marks records as deleted.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`], [`Error::Invalid`].
    pub async fn delete_records(
        &self,
        env: &str,
        key: &ConnectionKey,
        model: &str,
        records: Vec<Value>,
    ) -> Result<BatchOutcome> {
        let _ = (env, key, model, records);
        todo!()
    }

    /// Reads a page of records.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`], [`Error::Invalid`].
    pub async fn list_records(&self, env: &str, key: &ConnectionKey, request: RecordsRequest) -> Result<RecordPage> {
        let _ = (env, key, request);
        todo!()
    }

    // ----------------------------------------------------------- functions

    /// Runs one sync for one connection now, and waits for the end.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownFunction`], [`Error::UnknownConnection`],
    /// [`Error::Conflict`] when the sync runs already.
    pub async fn run_sync(&self, env: &str, key: &ConnectionKey, sync_name: &str) -> Result<SyncRunReport> {
        let _ = (env, key, sync_name);
        todo!()
    }

    /// Starts syncs in the background. With no connection id, it starts them
    /// for each connection of the integration.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownFunction`], [`Error::UnknownIntegration`].
    pub async fn trigger_syncs(
        &self,
        env: &str,
        integration: &str,
        connection_id: Option<&str>,
        syncs: &[String],
    ) -> Result<usize> {
        let _ = (env, integration, connection_id, syncs);
        todo!()
    }

    /// Pauses or starts the schedule of syncs for connections.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownFunction`], [`Error::UnknownConnection`].
    pub async fn set_sync_paused(
        &self,
        env: &str,
        integration: &str,
        connection_id: Option<&str>,
        syncs: &[String],
        paused: bool,
    ) -> Result<()> {
        let _ = (env, integration, connection_id, syncs, paused);
        todo!()
    }

    /// Returns the sync states.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownFunction`], [`Error::UnknownIntegration`].
    pub async fn sync_status(
        &self,
        env: &str,
        integration: &str,
        connection_id: Option<&str>,
        syncs: &[String],
    ) -> Result<Vec<SyncStatusEntry>> {
        let _ = (env, integration, connection_id, syncs);
        todo!()
    }

    /// Runs the auto-start syncs that are due, in all environments.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn run_due_syncs(&self) -> Result<Vec<SyncRunReport>> {
        todo!()
    }

    /// Runs an action and returns its output.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownFunction`], [`Error::UnknownConnection`], and the
    /// errors of the action.
    pub async fn run_action(&self, env: &str, key: &ConnectionKey, action: &str, input: Value) -> Result<Value> {
        let _ = (env, key, action, input);
        todo!()
    }

    // ----------------------------------------------------------- lifecycle

    /// Waits for background tasks (webhooks and triggered syncs) to end.
    pub async fn flush(&self) {
        todo!()
    }

    /// Starts the background loops: refresh sweep, sync scheduler and session
    /// purge. Stop them with [`Background::stop`].
    #[must_use]
    pub fn start_background(&self, settings: BackgroundSettings) -> Background {
        let _ = settings;
        todo!()
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
        let _ = (&self.stop, self.task.take());
        todo!()
    }
}

#[allow(dead_code)]
fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}
