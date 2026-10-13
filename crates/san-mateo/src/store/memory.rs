//! A store in memory. Data is lost at exit. Use it for tests and local
//! development.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{
    ConnectSessionRow, ConnectionFilter, ConnectionRow, IntegrationRow, OAuthSessionRow,
    RecordRow, RecordWrite, Store, SyncStateRow, Upserted,
};
use crate::error::Result;
use crate::records::{Change, RecordQuery};

#[derive(Debug, Default)]
struct Inner {
    integrations: BTreeMap<(String, String), IntegrationRow>,
    connections: BTreeMap<Uuid, ConnectionRow>,
    leases: BTreeMap<String, (String, DateTime<Utc>)>,
    connect_sessions: BTreeMap<Uuid, ConnectSessionRow>,
    oauth_sessions: BTreeMap<String, OAuthSessionRow>,
    records: BTreeMap<(Uuid, String, Uuid), RecordRow>,
    sync_states: BTreeMap<(Uuid, String), SyncStateRow>,
}

/// A store in memory.
#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

impl MemoryStore {
    /// Makes an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn insert_integration(&self, row: IntegrationRow) -> Result<()> {
        let _ = (row, &self.inner);
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

    async fn get_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        let _ = (env, key, connection_id);
        todo!()
    }

    async fn list_connections(
        &self,
        env: &str,
        filter: &ConnectionFilter,
    ) -> Result<Vec<ConnectionRow>> {
        let _ = (env, filter);
        todo!()
    }

    async fn update_connection(&self, row: &ConnectionRow) -> Result<Option<ConnectionRow>> {
        let _ = row;
        todo!()
    }

    async fn delete_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        let _ = (env, key, connection_id);
        todo!()
    }

    async fn connections_expiring_before(
        &self,
        before: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ConnectionRow>> {
        let _ = (before, limit);
        todo!()
    }

    async fn try_lease(
        &self,
        name: &str,
        owner: &str,
        now: DateTime<Utc>,
        ttl_ms: u64,
    ) -> Result<bool> {
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
    use std::sync::Arc;

    use super::*;
    use crate::store::conformance;

    #[tokio::test]
    async fn memory_store_passes_the_conformance_suite() {
        conformance::run_all(|| async { Arc::new(MemoryStore::new()) as Arc<dyn Store> }).await;
    }
}
