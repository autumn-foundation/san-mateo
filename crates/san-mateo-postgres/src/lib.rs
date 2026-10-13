//! A Postgres store for San Mateo, with diesel-async and embedded migrations.
//!
//! The pool type is the same as in autumn-web 0.8 and autumn-harvest 0.7, so
//! an app can share one pool.
//!
//! ```no_run
//! use std::sync::Arc;
//! use san_mateo_postgres::PgStore;
//!
//! # async fn demo() -> san_mateo::Result<()> {
//! let store = PgStore::connect("postgres://sm:sm@localhost/sm", 8)?;
//! store.migrate().await?;
//! let san = san_mateo::SanMateo::builder()
//!     .store(Arc::new(store))
//!     # .environment(san_mateo::EnvironmentConfig::new("dev", "sk"))
//!     # .key_ring(san_mateo::KeyRing::new("k1", [1; 32])?)
//!     .build()?;
//! # let _ = san;
//! # Ok(())
//! # }
//! ```
//!
//! [`PgStore::connect`] makes a pool with no TLS. For TLS, build a pool with
//! a TLS connection setup and use [`PgStore::new`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use san_mateo::Result;
use san_mateo::records::{Change, RecordQuery};
use san_mateo::store::{
    ConnectSessionRow, ConnectionFilter, ConnectionRow, IntegrationRow, OAuthSessionRow,
    RecordRow, RecordWrite, Store, SyncStateRow, Upserted,
};
use uuid::Uuid;

/// The diesel-async pool type. It is the same as `autumn_harvest::DbPool`.
pub type Pool = diesel_async::pooled_connection::deadpool::Pool<diesel_async::AsyncPgConnection>;

/// The embedded migrations: (version, SQL), in order.
pub const MIGRATIONS: &[(&str, &str)] = &[(
    "20261013000000_san_mateo_init",
    include_str!("../migrations/20261013000000_san_mateo_init/up.sql"),
)];

/// Returns all migrations as one SQL text. Tests and tools can run it.
#[must_use]
pub fn full_migrations_sql() -> String {
    MIGRATIONS.iter().map(|(_, sql)| *sql).collect::<Vec<_>>().join("\n")
}

/// A San Mateo store in Postgres.
#[derive(Clone)]
pub struct PgStore {
    pool: Pool,
}

impl std::fmt::Debug for PgStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgStore").finish_non_exhaustive()
    }
}

impl PgStore {
    /// Makes a store on a pool.
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Makes a pool with no TLS and a store on it.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Store`] when the pool does not build.
    pub fn connect(database_url: &str, max_size: usize) -> Result<Self> {
        let _ = (database_url, max_size);
        todo!()
    }

    /// Returns the pool.
    #[must_use]
    pub const fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Applies the migrations that are not applied yet. An advisory lock
    /// makes concurrent calls safe.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Store`] when a migration fails.
    pub async fn migrate(&self) -> Result<Vec<&'static str>> {
        todo!()
    }
}

#[async_trait]
impl Store for PgStore {
    async fn insert_integration(&self, row: IntegrationRow) -> Result<()> {
        let _ = row;
        todo!()
    }

    async fn update_integration(&self, row: IntegrationRow) -> Result<bool> {
        let _ = row;
        todo!()
    }

    async fn get_integration(&self, env: &str, key: &str) -> Result<Option<IntegrationRow>> {
        let _ = (env, key);
        todo!()
    }

    async fn list_integrations(&self, env: &str) -> Result<Vec<IntegrationRow>> {
        let _ = env;
        todo!()
    }

    async fn delete_integration(&self, env: &str, key: &str) -> Result<bool> {
        let _ = (env, key);
        todo!()
    }

    async fn upsert_connection(&self, row: ConnectionRow) -> Result<Upserted> {
        let _ = row;
        todo!()
    }

    async fn get_connection(&self, env: &str, key: &str, connection_id: &str) -> Result<Option<ConnectionRow>> {
        let _ = (env, key, connection_id);
        todo!()
    }

    async fn list_connections(&self, env: &str, filter: &ConnectionFilter) -> Result<Vec<ConnectionRow>> {
        let _ = (env, filter);
        todo!()
    }

    async fn update_connection(&self, row: &ConnectionRow) -> Result<Option<ConnectionRow>> {
        let _ = row;
        todo!()
    }

    async fn delete_connection(&self, env: &str, key: &str, connection_id: &str) -> Result<Option<ConnectionRow>> {
        let _ = (env, key, connection_id);
        todo!()
    }

    async fn connections_expiring_before(&self, before: DateTime<Utc>, limit: usize) -> Result<Vec<ConnectionRow>> {
        let _ = (before, limit);
        todo!()
    }

    async fn try_lease(&self, name: &str, owner: &str, now: DateTime<Utc>, ttl_ms: u64) -> Result<bool> {
        let _ = (name, owner, now, ttl_ms);
        todo!()
    }

    async fn release_lease(&self, name: &str, owner: &str) -> Result<()> {
        let _ = (name, owner);
        todo!()
    }

    async fn insert_connect_session(&self, row: ConnectSessionRow) -> Result<()> {
        let _ = row;
        todo!()
    }

    async fn get_connect_session(&self, token_hash: &str) -> Result<Option<ConnectSessionRow>> {
        let _ = token_hash;
        todo!()
    }

    async fn get_connect_session_by_id(&self, id: Uuid) -> Result<Option<ConnectSessionRow>> {
        let _ = id;
        todo!()
    }

    async fn delete_connect_session(&self, id: Uuid) -> Result<bool> {
        let _ = id;
        todo!()
    }

    async fn insert_oauth_session(&self, row: OAuthSessionRow) -> Result<()> {
        let _ = row;
        todo!()
    }

    async fn take_oauth_session(&self, state_hash: &str) -> Result<Option<OAuthSessionRow>> {
        let _ = state_hash;
        todo!()
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64> {
        let _ = now;
        todo!()
    }

    async fn upsert_records(
        &self,
        connection: Uuid,
        model: &str,
        rows: Vec<RecordWrite>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        let _ = (connection, model, rows, now);
        todo!()
    }

    async fn delete_records(
        &self,
        connection: Uuid,
        model: &str,
        ids: Vec<(Uuid, String)>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        let _ = (connection, model, ids, now);
        todo!()
    }

    async fn list_records(&self, connection: Uuid, query: &RecordQuery) -> Result<Vec<RecordRow>> {
        let _ = (connection, query);
        todo!()
    }

    async fn put_sync_state(&self, row: SyncStateRow) -> Result<()> {
        let _ = row;
        todo!()
    }

    async fn get_sync_state(&self, connection: Uuid, sync_name: &str) -> Result<Option<SyncStateRow>> {
        let _ = (connection, sync_name);
        todo!()
    }

    async fn list_sync_states(&self, connection: Uuid) -> Result<Vec<SyncStateRow>> {
        let _ = connection;
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_ordered_and_embedded() {
        assert!(!MIGRATIONS.is_empty());
        for w in MIGRATIONS.windows(2) {
            assert!(w[0].0 < w[1].0);
        }
        let sql = full_migrations_sql();
        for table in ["sm_integrations", "sm_connections", "sm_leases", "sm_connect_sessions", "sm_oauth_sessions", "sm_records", "sm_sync_states"] {
            assert!(sql.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")), "{table}");
        }
    }

    #[test]
    fn connect_rejects_a_bad_url() {
        assert!(PgStore::connect("not a url", 2).is_err());
    }
}
