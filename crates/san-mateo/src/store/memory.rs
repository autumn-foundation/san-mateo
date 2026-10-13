//! A store in memory. Data is lost at exit. Use it for tests and local
//! development.

// Each operation holds the lock from start to end, so each operation is atomic.
#![allow(clippy::significant_drop_tightening)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{
    ConnectSessionRow, ConnectionFilter, ConnectionRow, IntegrationRow, OAuthSessionRow, RecordRow,
    RecordWrite, Store, SyncStateRow, Upserted,
};
use crate::error::{Error, Result};
use crate::records::{Change, RecordQuery, RecordState, classify_delete, classify_upsert};

#[derive(Debug, Default)]
struct Inner {
    integrations: BTreeMap<(String, String), IntegrationRow>,
    connections: BTreeMap<Uuid, ConnectionRow>,
    by_key: BTreeMap<(String, String, String), Uuid>,
    leases: BTreeMap<String, (String, DateTime<Utc>)>,
    connect_sessions: BTreeMap<Uuid, ConnectSessionRow>,
    oauth_sessions: BTreeMap<String, OAuthSessionRow>,
    records: BTreeMap<(Uuid, String, Uuid), RecordRow>,
    sync_states: BTreeMap<(Uuid, String), SyncStateRow>,
}

fn key_of(row: &ConnectionRow) -> (String, String, String) {
    (
        row.environment.clone(),
        row.provider_config_key.clone(),
        row.connection_id.clone(),
    )
}

impl Inner {
    fn remove_connection(&mut self, id: Uuid) -> Option<ConnectionRow> {
        let row = self.connections.remove(&id)?;
        self.by_key.remove(&key_of(&row));
        self.records.retain(|(c, _, _), _| *c != id);
        self.sync_states.retain(|(c, _), _| *c != id);
        Some(row)
    }
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

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn record_matches(row: &RecordRow, query: &RecordQuery) -> bool {
    if let Some(c) = &query.cursor
        && (row.updated_at.timestamp_micros(), row.id) <= (c.updated_at_us, c.id)
    {
        return false;
    }
    if let Some(after) = query.modified_after_us
        && row.updated_at.timestamp_micros() < after
    {
        return false;
    }
    if !query.filter.is_empty() && !query.filter.contains(&row.last_action()) {
        return false;
    }
    query.external_ids.is_empty() || query.external_ids.contains(&row.external_id)
}

#[async_trait]
impl Store for MemoryStore {
    async fn insert_integration(&self, row: IntegrationRow) -> Result<()> {
        let mut g = self.lock();
        let key = (row.environment.clone(), row.unique_key.clone());
        if g.integrations.contains_key(&key) {
            return Err(Error::Conflict(format!(
                "integration {} exists",
                row.unique_key
            )));
        }
        g.integrations.insert(key, row);
        Ok(())
    }

    async fn update_integration(&self, row: IntegrationRow) -> Result<bool> {
        let mut g = self.lock();
        let key = (row.environment.clone(), row.unique_key.clone());
        Ok(g.integrations.get_mut(&key).is_some_and(|slot| {
            *slot = row;
            true
        }))
    }

    async fn get_integration(&self, env: &str, key: &str) -> Result<Option<IntegrationRow>> {
        Ok(self
            .lock()
            .integrations
            .get(&(env.to_owned(), key.to_owned()))
            .cloned())
    }

    async fn list_integrations(&self, env: &str) -> Result<Vec<IntegrationRow>> {
        Ok(self
            .lock()
            .integrations
            .values()
            .filter(|i| i.environment == env)
            .cloned()
            .collect())
    }

    async fn delete_integration(&self, env: &str, key: &str) -> Result<bool> {
        let mut g = self.lock();
        if g.integrations
            .remove(&(env.to_owned(), key.to_owned()))
            .is_none()
        {
            return Ok(false);
        }
        let ids: Vec<Uuid> = g
            .connections
            .values()
            .filter(|c| c.environment == env && c.provider_config_key == key)
            .map(|c| c.id)
            .collect();
        for id in ids {
            g.remove_connection(id);
        }
        Ok(true)
    }

    async fn upsert_connection(&self, mut row: ConnectionRow) -> Result<Upserted> {
        let mut g = self.lock();
        let key = key_of(&row);
        let existing = g
            .by_key
            .get(&key)
            .and_then(|id| g.connections.get(id))
            .cloned();
        let created = existing.is_none();
        if let Some(old) = existing {
            row.id = old.id;
            row.created_at = old.created_at;
            row.version = old.version + 1;
        }
        g.by_key.insert(key, row.id);
        g.connections.insert(row.id, row.clone());
        Ok(Upserted { row, created })
    }

    async fn get_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        let g = self.lock();
        let k = (env.to_owned(), key.to_owned(), connection_id.to_owned());
        Ok(g.by_key
            .get(&k)
            .and_then(|id| g.connections.get(id))
            .cloned())
    }

    async fn list_connections(
        &self,
        env: &str,
        filter: &ConnectionFilter,
    ) -> Result<Vec<ConnectionRow>> {
        let g = self.lock();
        let mut rows: Vec<ConnectionRow> = g
            .connections
            .values()
            .filter(|c| c.environment == env)
            .filter(|c| {
                filter
                    .provider_config_key
                    .as_ref()
                    .is_none_or(|k| &c.provider_config_key == k)
            })
            .filter(|c| {
                filter
                    .connection_id
                    .as_ref()
                    .is_none_or(|k| &c.connection_id == k)
            })
            .filter(|c| {
                filter
                    .end_user_id
                    .as_ref()
                    .is_none_or(|u| c.end_user.as_ref().is_some_and(|e| &e.id == u))
            })
            .cloned()
            .collect();
        rows.sort_by_key(|a| (a.created_at, a.id));
        Ok(rows)
    }

    async fn update_connection(&self, row: &ConnectionRow) -> Result<Option<ConnectionRow>> {
        let mut g = self.lock();
        let Some(stored) = g.connections.get(&row.id) else {
            return Ok(None);
        };
        if stored.version != row.version {
            return Ok(None);
        }
        let old_key = key_of(stored);
        let mut next = row.clone();
        next.version = row.version + 1;
        g.by_key.remove(&old_key);
        g.by_key.insert(key_of(&next), next.id);
        g.connections.insert(next.id, next.clone());
        Ok(Some(next))
    }

    async fn delete_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        let mut g = self.lock();
        let k = (env.to_owned(), key.to_owned(), connection_id.to_owned());
        let Some(id) = g.by_key.get(&k).copied() else {
            return Ok(None);
        };
        Ok(g.remove_connection(id))
    }

    async fn connections_expiring_before(
        &self,
        before: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<ConnectionRow>> {
        let g = self.lock();
        let mut rows: Vec<ConnectionRow> = g
            .connections
            .values()
            .filter(|c| {
                !c.refresh_exhausted && c.credentials_expires_at.is_some_and(|t| t < before)
            })
            .cloned()
            .collect();
        rows.sort_by_key(|c| (c.credentials_expires_at, c.id));
        rows.truncate(limit);
        Ok(rows)
    }

    async fn try_lease(
        &self,
        name: &str,
        owner: &str,
        now: DateTime<Utc>,
        ttl_ms: u64,
    ) -> Result<bool> {
        let mut g = self.lock();
        if let Some((holder, until)) = g.leases.get(name)
            && holder != owner
            && *until > now
        {
            return Ok(false);
        }
        let ttl = chrono::Duration::milliseconds(i64::try_from(ttl_ms).unwrap_or(i64::MAX));
        let until = now
            .checked_add_signed(ttl)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        g.leases.insert(name.to_owned(), (owner.to_owned(), until));
        Ok(true)
    }

    async fn release_lease(&self, name: &str, owner: &str) -> Result<()> {
        let mut g = self.lock();
        if g.leases
            .get(name)
            .is_some_and(|(holder, _)| holder == owner)
        {
            g.leases.remove(name);
        }
        Ok(())
    }

    async fn insert_connect_session(&self, row: ConnectSessionRow) -> Result<()> {
        self.lock().connect_sessions.insert(row.id, row);
        Ok(())
    }

    async fn get_connect_session(&self, token_hash: &str) -> Result<Option<ConnectSessionRow>> {
        Ok(self
            .lock()
            .connect_sessions
            .values()
            .find(|s| s.token_hash == token_hash)
            .cloned())
    }

    async fn get_connect_session_by_id(&self, id: Uuid) -> Result<Option<ConnectSessionRow>> {
        Ok(self.lock().connect_sessions.get(&id).cloned())
    }

    async fn delete_connect_session(&self, id: Uuid) -> Result<bool> {
        Ok(self.lock().connect_sessions.remove(&id).is_some())
    }

    async fn insert_oauth_session(&self, row: OAuthSessionRow) -> Result<()> {
        self.lock()
            .oauth_sessions
            .insert(row.state_hash.clone(), row);
        Ok(())
    }

    async fn take_oauth_session(&self, state_hash: &str) -> Result<Option<OAuthSessionRow>> {
        Ok(self.lock().oauth_sessions.remove(state_hash))
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64> {
        let mut g = self.lock();
        let before = g.connect_sessions.len() + g.oauth_sessions.len();
        g.connect_sessions.retain(|_, s| s.expires_at > now);
        g.oauth_sessions.retain(|_, s| s.expires_at > now);
        g.leases.retain(|_, (_, until)| *until > now);
        let after = g.connect_sessions.len() + g.oauth_sessions.len();
        Ok(u64::try_from(before - after).unwrap_or(u64::MAX))
    }

    async fn upsert_records(
        &self,
        connection: Uuid,
        model: &str,
        rows: Vec<RecordWrite>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        let mut g = self.lock();
        let mut out = Vec::with_capacity(rows.len());
        for w in rows {
            let key = (connection, model.to_owned(), w.id);
            let state = g.records.get(&key).map(|r| RecordState {
                data_hash: r.data_hash.clone(),
                deleted: r.deleted_at.is_some(),
            });
            let change = classify_upsert(state.as_ref(), &w.data_hash);
            match change {
                Change::Added => {
                    g.records.insert(
                        key,
                        RecordRow {
                            id: w.id,
                            external_id: w.external_id.clone(),
                            data: w.data,
                            data_hash: w.data_hash,
                            created_at: now,
                            updated_at: now,
                            deleted_at: None,
                        },
                    );
                }
                Change::Updated => {
                    if let Some(r) = g.records.get_mut(&key) {
                        r.data = w.data;
                        r.data_hash = w.data_hash;
                        r.updated_at = now;
                    }
                }
                Change::Unchanged | Change::Deleted | Change::Ignored => {}
            }
            out.push((w.external_id, change));
        }
        Ok(out)
    }

    async fn delete_records(
        &self,
        connection: Uuid,
        model: &str,
        ids: Vec<(Uuid, String)>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        let mut g = self.lock();
        let mut out = Vec::with_capacity(ids.len());
        for (id, external) in ids {
            let key = (connection, model.to_owned(), id);
            let state = g.records.get(&key).map(|r| RecordState {
                data_hash: r.data_hash.clone(),
                deleted: r.deleted_at.is_some(),
            });
            let change = classify_delete(state.as_ref());
            if change == Change::Deleted
                && let Some(r) = g.records.get_mut(&key)
            {
                r.deleted_at = Some(now);
                r.updated_at = now;
            }
            out.push((external, change));
        }
        Ok(out)
    }

    async fn list_records(&self, connection: Uuid, query: &RecordQuery) -> Result<Vec<RecordRow>> {
        let g = self.lock();
        let mut rows: Vec<RecordRow> = g
            .records
            .iter()
            .filter(|((c, m, _), _)| *c == connection && *m == query.model)
            .map(|(_, r)| r)
            .filter(|r| record_matches(r, query))
            .cloned()
            .collect();
        rows.sort_by_key(|r| (r.updated_at.timestamp_micros(), r.id));
        rows.truncate(query.limit as usize + 1);
        Ok(rows)
    }

    async fn put_sync_state(&self, row: SyncStateRow) -> Result<()> {
        self.lock()
            .sync_states
            .insert((row.connection, row.sync_name.clone()), row);
        Ok(())
    }

    async fn get_sync_state(
        &self,
        connection: Uuid,
        sync_name: &str,
    ) -> Result<Option<SyncStateRow>> {
        Ok(self
            .lock()
            .sync_states
            .get(&(connection, sync_name.to_owned()))
            .cloned())
    }

    async fn list_sync_states(&self, connection: Uuid) -> Result<Vec<SyncStateRow>> {
        Ok(self
            .lock()
            .sync_states
            .iter()
            .filter(|((c, _), _)| *c == connection)
            .map(|(_, r)| r.clone())
            .collect())
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
