//! The storage seam. [`MemoryStore`] keeps data in memory. The
//! `san-mateo-postgres` crate keeps data in Postgres.
//!
//! The store never sees plaintext secrets. The service seals credentials,
//! client secrets, PKCE verifiers and record data before a write.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::Result;
use crate::model::{ConnectionKey, EndUser, IntegrationDefaults, Organization, Tags};
use crate::records::{Change, LastAction, RecordQuery};

mod memory;
pub use memory::MemoryStore;

#[cfg(any(test, feature = "test-support"))]
pub mod conformance;

/// A stored integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationRow {
    /// The environment id.
    pub environment: String,
    /// The integration key (Nango: `unique_key`, `provider_config_key`).
    pub unique_key: String,
    /// The provider name in the catalog.
    pub provider: String,
    /// The name to show.
    pub display_name: Option<String>,
    /// The OAuth client id.
    pub client_id: Option<String>,
    /// The sealed OAuth client secret.
    pub client_secret: Option<String>,
    /// The scopes.
    pub scopes: Vec<String>,
    /// Free-form config.
    pub custom: BTreeMap<String, String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last change time.
    pub updated_at: DateTime<Utc>,
}

/// A stored connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRow {
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
    /// The sealed credentials JSON.
    pub credentials: String,
    /// The credentials expiry, for the refresh sweep.
    pub credentials_expires_at: Option<DateTime<Utc>>,
    /// Connection config (a JSON object).
    pub connection_config: Value,
    /// Metadata (a JSON object).
    pub metadata: Value,
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
    /// Last successful refresh.
    pub last_refresh_success: Option<DateTime<Utc>>,
    /// Last failed refresh.
    pub last_refresh_failure: Option<DateTime<Utc>>,
    /// Number of days with a failed refresh.
    pub refresh_attempts: u32,
    /// `true` when refresh stops.
    pub refresh_exhausted: bool,
    /// The row version for compare-and-set writes.
    pub version: i64,
}

impl ConnectionRow {
    /// Returns the key of the connection.
    #[must_use]
    pub fn key(&self) -> ConnectionKey {
        ConnectionKey::new(&self.provider_config_key, &self.connection_id)
    }
}

/// The result of [`Store::upsert_connection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upserted {
    /// The stored row.
    pub row: ConnectionRow,
    /// `true` for a new connection, `false` for a replace.
    pub created: bool,
}

/// A filter for [`Store::list_connections`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionFilter {
    /// Only this integration.
    pub provider_config_key: Option<String>,
    /// Only this connection id.
    pub connection_id: Option<String>,
    /// Only this end user id.
    pub end_user_id: Option<String>,
}

/// A stored connect session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectSessionRow {
    /// The id.
    pub id: Uuid,
    /// The environment id.
    pub environment: String,
    /// The keyed hash of the token.
    pub token_hash: String,
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
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Expiry time.
    pub expires_at: DateTime<Utc>,
}

/// A stored OAuth session (one authorization request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthSessionRow {
    /// The keyed hash of `state`.
    pub state_hash: String,
    /// The environment id.
    pub environment: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id to make.
    pub connection_id: String,
    /// The connect session, if any.
    pub connect_session_id: Option<Uuid>,
    /// The sealed PKCE verifier, if PKCE is on.
    pub code_verifier: Option<String>,
    /// Connection config for the new connection.
    pub connection_config: Value,
    /// The callback URL that the authorization request used.
    pub redirect_uri: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Expiry time.
    pub expires_at: DateTime<Utc>,
}

/// One record to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordWrite {
    /// The stable id.
    pub id: Uuid,
    /// The external id.
    pub external_id: String,
    /// The sealed record JSON.
    pub data: String,
    /// The keyed content hash.
    pub data_hash: String,
}

/// A stored record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRow {
    /// The stable id.
    pub id: Uuid,
    /// The external id.
    pub external_id: String,
    /// The sealed record JSON.
    pub data: String,
    /// The keyed content hash.
    pub data_hash: String,
    /// First time seen, or the time of the last restore after a delete.
    pub created_at: DateTime<Utc>,
    /// Last change time.
    pub updated_at: DateTime<Utc>,
    /// Delete time.
    pub deleted_at: Option<DateTime<Utc>>,
}

impl RecordRow {
    /// Returns the last change of the record.
    #[must_use]
    pub fn last_action(&self) -> LastAction {
        if self.deleted_at.is_some() {
            LastAction::Deleted
        } else if self.created_at == self.updated_at {
            LastAction::Added
        } else {
            LastAction::Updated
        }
    }
}

/// The status of a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SyncStatus {
    /// A run is in progress.
    Running,
    /// The last run succeeded.
    Success,
    /// The last run failed.
    Error,
    /// The schedule is paused. Only reports use this status.
    Paused,
    /// The sync has not run.
    Stopped,
}

/// Counts of one sync run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCounts {
    /// Added records.
    pub added: u64,
    /// Updated records.
    pub updated: u64,
    /// Deleted records.
    pub deleted: u64,
}

/// The state of one sync for one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStateRow {
    /// The connection id (internal).
    pub connection: Uuid,
    /// The sync name.
    pub sync_name: String,
    /// The status.
    pub status: SyncStatus,
    /// The frequency in ms.
    pub frequency_ms: u64,
    /// `true` when the scheduler skips the sync.
    pub paused: bool,
    /// Start of the last run.
    pub last_started_at: Option<DateTime<Utc>>,
    /// End of the last run.
    pub last_finished_at: Option<DateTime<Utc>>,
    /// Start of the last successful run.
    pub last_success_at: Option<DateTime<Utc>>,
    /// The error of the last failed run.
    pub last_error: Option<String>,
    /// Counts of the last run.
    pub last_counts: SyncCounts,
}

/// Storage for San Mateo.
#[async_trait]
pub trait Store: Send + Sync + 'static {
    /// Adds an integration. [`crate::Error::Conflict`] when the key exists.
    async fn insert_integration(&self, row: IntegrationRow) -> Result<()>;
    /// Replaces an integration. Returns `false` when it does not exist.
    async fn update_integration(&self, row: IntegrationRow) -> Result<bool>;
    /// Reads an integration.
    async fn get_integration(&self, env: &str, key: &str) -> Result<Option<IntegrationRow>>;
    /// Lists the integrations of an environment, sorted by key.
    async fn list_integrations(&self, env: &str) -> Result<Vec<IntegrationRow>>;
    /// Deletes an integration and its connections. Returns `false` when it
    /// does not exist.
    async fn delete_integration(&self, env: &str, key: &str) -> Result<bool>;

    /// Adds or replaces a connection, by (environment, integration, connection
    /// id). A replace keeps `id` and `created_at` and adds one to `version`.
    async fn upsert_connection(&self, row: ConnectionRow) -> Result<Upserted>;
    /// Reads a connection.
    async fn get_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>>;
    /// Lists connections, sorted by creation time and id.
    async fn list_connections(
        &self,
        env: &str,
        filter: &ConnectionFilter,
    ) -> Result<Vec<ConnectionRow>>;
    /// Writes a connection when its `version` is the stored version. Returns
    /// the stored row (with `version + 1`), or `None` when the version is
    /// stale or the row is gone.
    async fn update_connection(&self, row: &ConnectionRow) -> Result<Option<ConnectionRow>>;
    /// Sets `last_fetched_at` of a connection. It does not change `version`,
    /// so a read does not make a compare-and-set write fail. A missing row
    /// is not an error.
    async fn touch_connection(&self, id: Uuid, at: DateTime<Utc>) -> Result<()>;
    /// Deletes a connection, its records and its sync states.
    async fn delete_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>>;
    /// Lists connections, in all environments, that are not exhausted and
    /// expire before `before`. Sorted by `(credentials_expires_at, id)`. With
    /// `after`, only rows after that `(expiry, id)` come back, for paging.
    async fn connections_expiring_before(
        &self,
        before: DateTime<Utc>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
    ) -> Result<Vec<ConnectionRow>>;

    /// Takes the lease `name` for `ttl_ms`. Returns `false` when another owner
    /// holds a lease that has not expired.
    async fn try_lease(
        &self,
        name: &str,
        owner: &str,
        now: DateTime<Utc>,
        ttl_ms: u64,
    ) -> Result<bool>;
    /// Gives back a lease. Only the owner can give it back.
    async fn release_lease(&self, name: &str, owner: &str) -> Result<()>;

    /// Adds a connect session.
    async fn insert_connect_session(&self, row: ConnectSessionRow) -> Result<()>;
    /// Reads a connect session by token hash. It can be expired.
    async fn get_connect_session(&self, token_hash: &str) -> Result<Option<ConnectSessionRow>>;
    /// Reads a connect session by id. It can be expired.
    async fn get_connect_session_by_id(&self, id: Uuid) -> Result<Option<ConnectSessionRow>>;
    /// Deletes a connect session.
    async fn delete_connect_session(&self, id: Uuid) -> Result<bool>;

    /// Adds an OAuth session.
    async fn insert_oauth_session(&self, row: OAuthSessionRow) -> Result<()>;
    /// Reads and deletes an OAuth session. A second call gives `None`.
    async fn take_oauth_session(&self, state_hash: &str) -> Result<Option<OAuthSessionRow>>;

    /// Deletes expired connect sessions, OAuth sessions and leases. Returns
    /// the number of deleted sessions.
    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64>;

    /// Writes records. Each row is compared with the stored row. Returns the
    /// change for each external id, in input order.
    ///
    /// The write time is [`crate::records::next_write_time`] of `now`, so
    /// times of one (connection, model) only go up. Batches of one
    /// (connection, model) do not overlap. [`crate::Error::UnknownConnection`]
    /// when the connection does not exist.
    async fn upsert_records(
        &self,
        connection: Uuid,
        model: &str,
        rows: Vec<RecordWrite>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>>;
    /// Marks records as deleted. Returns the change for each external id.
    /// The write time and the errors are as for [`Store::upsert_records`].
    async fn delete_records(
        &self,
        connection: Uuid,
        model: &str,
        ids: Vec<(Uuid, String)>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>>;
    /// Lists records sorted by `(updated_at, id)`. Returns up to
    /// `query.limit + 1` rows, so the caller can tell if a next page exists.
    async fn list_records(&self, connection: Uuid, query: &RecordQuery) -> Result<Vec<RecordRow>>;

    /// Writes the state of a sync. It keeps the stored `paused` value: only
    /// [`Store::set_sync_paused`] changes it. A new state takes `row.paused`.
    /// [`crate::Error::UnknownConnection`] when the connection does not exist.
    async fn put_sync_state(&self, row: SyncStateRow) -> Result<()>;
    /// Sets `paused` of a sync state to `row.paused`. It changes no other
    /// field. When no state is stored, it adds `row`. The errors are as for
    /// [`Store::put_sync_state`].
    async fn set_sync_paused(&self, row: SyncStateRow) -> Result<()>;
    /// Reads the state of a sync.
    async fn get_sync_state(
        &self,
        connection: Uuid,
        sync_name: &str,
    ) -> Result<Option<SyncStateRow>>;
    /// Lists the sync states of a connection, sorted by name.
    async fn list_sync_states(&self, connection: Uuid) -> Result<Vec<SyncStateRow>>;
}
