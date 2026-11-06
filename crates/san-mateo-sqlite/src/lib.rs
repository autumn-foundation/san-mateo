//! A SQLite store for San Mateo. It keeps all data in one file, so an app
//! can embed San Mateo with no database server.
//!
//! ```no_run
//! use std::sync::Arc;
//! use san_mateo_sqlite::SqliteStore;
//!
//! # async fn demo() -> san_mateo::Result<()> {
//! let store = SqliteStore::open("sanmateo.db")?;
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
//! # Concurrency
//!
//! A store holds one connection. Each operation runs on the blocking thread
//! pool of tokio and holds the connection from start to end. So the
//! operations of one store are atomic and run one after the other.
//!
//! Each write that has more than one statement runs in `BEGIN IMMEDIATE`.
//! That takes the write lock of the file at the start. Two stores on one
//! file, in one process or in two, then also see atomic operations. A store
//! waits up to [`BUSY_TIMEOUT`] for the lock. The file uses WAL mode, so a
//! read does not wait for a write.
//!
//! # Format
//!
//! The tables are the tables of the Postgres store, with the `sm_` prefix.
//! A time is an integer count of microseconds since the Unix epoch, as in
//! Postgres. A UUID is hyphenated lower-case text, so text order is the
//! byte order that Postgres uses. A JSON value is text.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::types::{ToSql, Type};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use san_mateo::records::{
    Change, LastAction, RecordQuery, RecordState, classify_delete, classify_upsert, next_write_time,
};
use san_mateo::store::{
    ConnectSessionRow, ConnectionFilter, ConnectionRow, IntegrationRow, OAuthSessionRow, RecordRow,
    RecordWrite, Store, SyncStateRow, Upserted,
};
use san_mateo::{Error, Result};
use uuid::Uuid;

mod rows;

use rows::{ConnectionDb, SessionData, SyncData};

/// The embedded migrations: (version, SQL), in order.
pub const MIGRATIONS: &[(&str, &str)] = &[(
    "20261106200622_san_mateo_init",
    include_str!("../migrations/20261106200622_san_mateo_init/up.sql"),
)];

/// The longest wait for the write lock of the file.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Returns all migrations as one SQL text. Tests and tools can run it.
#[must_use]
pub fn full_migrations_sql() -> String {
    MIGRATIONS
        .iter()
        .map(|(_, sql)| *sql)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A San Mateo store in SQLite.
///
/// A clone shares the connection of the original.
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore").finish_non_exhaustive()
    }
}

/// Maps a database error. The message stays in the store error, which the
/// API does not show to callers.
fn db(e: impl std::fmt::Display) -> Error {
    Error::Store(format!("sqlite: {e}"))
}

fn unknown_connection(id: Uuid) -> Error {
    Error::UnknownConnection(id.to_string())
}

/// The columns of `sm_connections`, in the order of [`read_connection`].
const CONNECTION_COLUMNS: &str = "id, environment, provider_config_key, connection_id, provider, \
     credentials, credentials_expires_at, connection_config, metadata, tags, end_user, \
     end_user_id, webhook_url_override, created_at, updated_at, last_fetched_at, \
     last_refresh_success, last_refresh_failure, refresh_attempts, refresh_exhausted, version";

/// The columns of `sm_records`, in the order of [`read_record`].
const RECORD_COLUMNS: &str = "id, external_id, data, data_hash, created_at, updated_at, deleted_at";

/// The columns of `sm_oauth_sessions`, in the order of [`read_oauth`].
const OAUTH_COLUMNS: &str = "state_hash, environment, provider_config_key, connection_id, \
     connect_session_id, code_verifier, connection_config, redirect_uri, created_at, expires_at";

/// The columns of `sm_connect_sessions`, in the order of [`read_session`].
const SESSION_COLUMNS: &str = "id, environment, token_hash, data, created_at, expires_at";

/// The columns of `sm_integrations`, in the order of [`read_integration`].
const INTEGRATION_COLUMNS: &str = "environment, unique_key, provider, display_name, client_id, \
     client_secret, scopes, custom, created_at, updated_at";

/// Returns a time as microseconds since the Unix epoch.
const fn us(t: DateTime<Utc>) -> i64 {
    t.timestamp_micros()
}

fn conversion(
    idx: usize,
    ty: Type,
    e: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(idx, ty, Box::new(e))
}

/// A stored time is out of the range of [`DateTime`].
#[derive(Debug)]
struct TimeOutOfRange;

impl std::fmt::Display for TimeOutOfRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a stored time is out of range")
    }
}

impl std::error::Error for TimeOutOfRange {}

fn time_of(idx: usize, v: i64) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_micros(v)
        .ok_or_else(|| conversion(idx, Type::Integer, TimeOutOfRange))
}

fn get_time(row: &Row<'_>, idx: usize) -> rusqlite::Result<DateTime<Utc>> {
    time_of(idx, row.get(idx)?)
}

fn get_opt_time(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<DateTime<Utc>>> {
    row.get::<_, Option<i64>>(idx)?
        .map(|v| time_of(idx, v))
        .transpose()
}

fn uuid_of(idx: usize, v: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(v).map_err(|e| conversion(idx, Type::Text, e))
}

fn get_uuid(row: &Row<'_>, idx: usize) -> rusqlite::Result<Uuid> {
    uuid_of(idx, &row.get::<_, String>(idx)?)
}

fn get_json<T: serde::de::DeserializeOwned>(row: &Row<'_>, idx: usize) -> rusqlite::Result<T> {
    let text: String = row.get(idx)?;
    serde_json::from_str(&text).map_err(|e| conversion(idx, Type::Text, e))
}

fn get_opt_json<T: serde::de::DeserializeOwned>(
    row: &Row<'_>,
    idx: usize,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(idx)?
        .map(|text| serde_json::from_str(&text).map_err(|e| conversion(idx, Type::Text, e)))
        .transpose()
}

fn read_integration(r: &Row<'_>) -> rusqlite::Result<IntegrationRow> {
    Ok(IntegrationRow {
        environment: r.get(0)?,
        unique_key: r.get(1)?,
        provider: r.get(2)?,
        display_name: r.get(3)?,
        client_id: r.get(4)?,
        client_secret: r.get(5)?,
        scopes: get_json(r, 6)?,
        custom: get_json(r, 7)?,
        created_at: get_time(r, 8)?,
        updated_at: get_time(r, 9)?,
    })
}

fn read_connection(r: &Row<'_>) -> rusqlite::Result<ConnectionRow> {
    Ok(ConnectionRow {
        id: get_uuid(r, 0)?,
        environment: r.get(1)?,
        provider_config_key: r.get(2)?,
        connection_id: r.get(3)?,
        provider: r.get(4)?,
        credentials: r.get(5)?,
        credentials_expires_at: get_opt_time(r, 6)?,
        connection_config: get_json(r, 7)?,
        metadata: get_json(r, 8)?,
        tags: get_json(r, 9)?,
        end_user: get_opt_json(r, 10)?,
        // Column 11 is `end_user_id`, a copy of `end_user.id` for the filter.
        webhook_url_override: r.get(12)?,
        created_at: get_time(r, 13)?,
        updated_at: get_time(r, 14)?,
        last_fetched_at: get_opt_time(r, 15)?,
        last_refresh_success: get_opt_time(r, 16)?,
        last_refresh_failure: get_opt_time(r, 17)?,
        refresh_attempts: r.get(18)?,
        refresh_exhausted: r.get(19)?,
        version: r.get(20)?,
    })
}

fn read_session(r: &Row<'_>) -> rusqlite::Result<ConnectSessionRow> {
    let d: SessionData = get_json(r, 3)?;
    Ok(d.into_row(
        get_uuid(r, 0)?,
        r.get(1)?,
        r.get(2)?,
        get_time(r, 4)?,
        get_time(r, 5)?,
    ))
}

fn read_oauth(r: &Row<'_>) -> rusqlite::Result<OAuthSessionRow> {
    Ok(OAuthSessionRow {
        state_hash: r.get(0)?,
        environment: r.get(1)?,
        provider_config_key: r.get(2)?,
        connection_id: r.get(3)?,
        connect_session_id: r
            .get::<_, Option<String>>(4)?
            .map(|v| uuid_of(4, &v))
            .transpose()?,
        code_verifier: r.get(5)?,
        connection_config: get_json(r, 6)?,
        redirect_uri: r.get(7)?,
        created_at: get_time(r, 8)?,
        expires_at: get_time(r, 9)?,
    })
}

fn read_record(r: &Row<'_>) -> rusqlite::Result<RecordRow> {
    Ok(RecordRow {
        id: get_uuid(r, 0)?,
        external_id: r.get(1)?,
        data: r.get(2)?,
        data_hash: r.get(3)?,
        created_at: get_time(r, 4)?,
        updated_at: get_time(r, 5)?,
        deleted_at: get_opt_time(r, 6)?,
    })
}

/// Opens a transaction that takes the write lock at the start.
fn write_tx(c: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    c.transaction_with_behavior(TransactionBehavior::Immediate)
}

impl SqliteStore {
    /// Opens or makes the database file at `path`. Call
    /// [`SqliteStore::migrate`] before the first use.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Store`] when SQLite cannot open the file, for
    /// example because its directory does not exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).map_err(db)?;
        Self::setup(conn, true)
    }

    /// Makes a database in memory. Data is lost when the last clone of the
    /// store drops. Use it for tests.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Store`] when SQLite fails.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(db)?;
        Self::setup(conn, false)
    }

    fn setup(conn: Connection, wal: bool) -> Result<Self> {
        conn.busy_timeout(BUSY_TIMEOUT).map_err(db)?;
        // SQLite checks foreign keys only when a connection turns them on.
        conn.pragma_update(None, "foreign_keys", true).map_err(db)?;
        if wal {
            let mode: String = conn
                .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
                .map_err(db)?;
            if !mode.eq_ignore_ascii_case("wal") {
                return Err(db(format!("the journal mode is {mode}, not wal")));
            }
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` with the connection, on the blocking thread pool.
    async fn run<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            // A panic in `f` drops its transaction, which rolls back. So the
            // connection is still good after a poison.
            let mut c = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut c)
        })
        .await
        .map_err(db)?
        .map_err(db)
    }

    /// Applies the migrations that are not applied yet. Returns the versions
    /// that it applied. The write lock makes concurrent calls safe, also
    /// from two processes.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Store`] when a migration fails.
    pub async fn migrate(&self) -> Result<Vec<&'static str>> {
        self.run(|c| {
            let tx = write_tx(c)?;
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS sm_schema_migrations \
                 (version TEXT PRIMARY KEY NOT NULL, applied_at INTEGER NOT NULL) STRICT",
            )?;
            let applied: Vec<String> = tx
                .prepare("SELECT version FROM sm_schema_migrations")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let mut done = Vec::new();
            for (version, sql) in MIGRATIONS {
                if applied.iter().any(|a| a == version) {
                    continue;
                }
                tx.execute_batch(sql)?;
                tx.execute(
                    "INSERT INTO sm_schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    params![version, us(Utc::now())],
                )?;
                done.push(*version);
            }
            tx.commit()?;
            Ok(done)
        })
        .await
    }
}

/// Returns `true` when the connection exists.
fn connection_exists(c: &Connection, id: Uuid) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT 1 FROM sm_connections WHERE id = ?1",
        [id.to_string()],
        |_| Ok(()),
    )
    .optional()
    .map(|f| f.is_some())
}

/// Deletes connections and their records and sync states. The foreign keys
/// do the same. These statements make the rule clear in the code.
fn delete_connections(c: &Connection, ids: &[String]) -> rusqlite::Result<()> {
    for id in ids {
        c.execute("DELETE FROM sm_records WHERE connection = ?1", [id])?;
        c.execute("DELETE FROM sm_sync_states WHERE connection = ?1", [id])?;
        c.execute("DELETE FROM sm_connections WHERE id = ?1", [id])?;
    }
    Ok(())
}

fn select_connection_by_key(
    c: &Connection,
    env: &str,
    key: &str,
    cid: &str,
) -> rusqlite::Result<Option<ConnectionRow>> {
    c.query_row(
        &format!(
            "SELECT {CONNECTION_COLUMNS} FROM sm_connections \
             WHERE environment = ?1 AND provider_config_key = ?2 AND connection_id = ?3"
        ),
        params![env, key, cid],
        read_connection,
    )
    .optional()
}

/// Returns the write time of a record batch. See [`next_write_time`].
fn write_time(
    c: &Connection,
    connection: &str,
    model: &str,
    now: DateTime<Utc>,
) -> rusqlite::Result<DateTime<Utc>> {
    let newest: Option<i64> = c.query_row(
        "SELECT max(updated_at) FROM sm_records WHERE connection = ?1 AND model = ?2",
        params![connection, model],
        |r| r.get(0),
    )?;
    let newest = newest.map(|v| time_of(0, v)).transpose()?;
    Ok(next_write_time(now, newest))
}

/// Reads the state of a record.
fn record_state(
    c: &Connection,
    connection: &str,
    model: &str,
    id: &str,
) -> rusqlite::Result<Option<RecordState>> {
    c.query_row(
        "SELECT data_hash, deleted_at FROM sm_records \
         WHERE connection = ?1 AND model = ?2 AND id = ?3",
        params![connection, model, id],
        |r| {
            Ok(RecordState {
                data_hash: r.get(0)?,
                deleted: r.get::<_, Option<i64>>(1)?.is_some(),
            })
        },
    )
    .optional()
}

/// Writes one record. Returns its change. The caller holds the write lock.
fn upsert_record(
    c: &Connection,
    connection: &str,
    model: &str,
    w: &RecordWrite,
    at: i64,
) -> rusqlite::Result<Change> {
    let id = w.id.to_string();
    let state = record_state(c, connection, model, &id)?;
    let change = classify_upsert(state.as_ref(), &w.data_hash);
    match (change, state) {
        (Change::Added, None) => {
            c.execute(
                "INSERT INTO sm_records \
                 (connection, model, id, external_id, data, data_hash, created_at, updated_at, deleted_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, NULL)",
                params![connection, model, id, w.external_id, w.data, w.data_hash, at],
            )?;
        }
        // A record that comes back after a delete is new again.
        (Change::Added, Some(_)) => {
            c.execute(
                "UPDATE sm_records SET external_id = ?4, data = ?5, data_hash = ?6, \
                 created_at = ?7, updated_at = ?7, deleted_at = NULL \
                 WHERE connection = ?1 AND model = ?2 AND id = ?3",
                params![
                    connection,
                    model,
                    id,
                    w.external_id,
                    w.data,
                    w.data_hash,
                    at
                ],
            )?;
        }
        (Change::Updated, _) => {
            c.execute(
                "UPDATE sm_records SET data = ?4, data_hash = ?5, updated_at = ?6 \
                 WHERE connection = ?1 AND model = ?2 AND id = ?3",
                params![connection, model, id, w.data, w.data_hash, at],
            )?;
        }
        _ => {}
    }
    Ok(change)
}

/// Adds a sync state, or changes the stored one with `merge`. Returns
/// `false` when the connection does not exist.
fn write_sync_state(
    c: &mut Connection,
    row: &SyncStateRow,
    merge: impl FnOnce(SyncData) -> SyncData,
) -> rusqlite::Result<bool> {
    let tx = write_tx(c)?;
    let conn = row.connection.to_string();
    if !connection_exists(&tx, row.connection)? {
        return Ok(false);
    }
    let stored: Option<SyncData> = tx
        .query_row(
            "SELECT data FROM sm_sync_states WHERE connection = ?1 AND sync_name = ?2",
            params![conn, row.sync_name],
            |r| get_json(r, 0),
        )
        .optional()?;
    let data = stored.map_or_else(|| SyncData::of(row), merge);
    let text = serde_json::to_string(&data).map_err(|e| conversion(0, Type::Text, e))?;
    tx.execute(
        "INSERT INTO sm_sync_states (connection, sync_name, data) VALUES (?1, ?2, ?3) \
         ON CONFLICT (connection, sync_name) DO UPDATE SET data = excluded.data",
        params![conn, row.sync_name, text],
    )?;
    tx.commit()?;
    Ok(true)
}

#[async_trait]
impl Store for SqliteStore {
    async fn insert_integration(&self, row: IntegrationRow) -> Result<()> {
        let (scopes, custom) = (rows::json(&row.scopes)?, rows::json(&row.custom)?);
        let key = row.unique_key.clone();
        let n = self
            .run(move |c| {
                c.execute(
                    &format!(
                        "INSERT INTO sm_integrations ({INTEGRATION_COLUMNS}) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT DO NOTHING"
                    ),
                    params![
                        row.environment,
                        row.unique_key,
                        row.provider,
                        row.display_name,
                        row.client_id,
                        row.client_secret,
                        scopes,
                        custom,
                        us(row.created_at),
                        us(row.updated_at),
                    ],
                )
            })
            .await?;
        if n == 0 {
            return Err(Error::Conflict(format!("integration {key} exists")));
        }
        Ok(())
    }

    async fn update_integration(&self, row: IntegrationRow) -> Result<bool> {
        let (scopes, custom) = (rows::json(&row.scopes)?, rows::json(&row.custom)?);
        let n = self
            .run(move |c| {
                c.execute(
                    "UPDATE sm_integrations SET provider = ?3, display_name = ?4, client_id = ?5, \
                     client_secret = ?6, scopes = ?7, custom = ?8, created_at = ?9, updated_at = ?10 \
                     WHERE environment = ?1 AND unique_key = ?2",
                    params![
                        row.environment,
                        row.unique_key,
                        row.provider,
                        row.display_name,
                        row.client_id,
                        row.client_secret,
                        scopes,
                        custom,
                        us(row.created_at),
                        us(row.updated_at),
                    ],
                )
            })
            .await?;
        Ok(n > 0)
    }

    async fn get_integration(&self, env: &str, key: &str) -> Result<Option<IntegrationRow>> {
        let (env, key) = (env.to_owned(), key.to_owned());
        self.run(move |c| {
            c.query_row(
                &format!(
                    "SELECT {INTEGRATION_COLUMNS} FROM sm_integrations \
                     WHERE environment = ?1 AND unique_key = ?2"
                ),
                params![env, key],
                read_integration,
            )
            .optional()
        })
        .await
    }

    async fn list_integrations(&self, env: &str) -> Result<Vec<IntegrationRow>> {
        let env = env.to_owned();
        self.run(move |c| {
            c.prepare(&format!(
                "SELECT {INTEGRATION_COLUMNS} FROM sm_integrations \
                 WHERE environment = ?1 ORDER BY unique_key"
            ))?
            .query_map([env], read_integration)?
            .collect()
        })
        .await
    }

    async fn delete_integration(&self, env: &str, key: &str) -> Result<bool> {
        let (env, key) = (env.to_owned(), key.to_owned());
        self.run(move |c| {
            let tx = write_tx(c)?;
            let n = tx.execute(
                "DELETE FROM sm_integrations WHERE environment = ?1 AND unique_key = ?2",
                params![env, key],
            )?;
            if n == 0 {
                return Ok(false);
            }
            let ids: Vec<String> = tx
                .prepare(
                    "SELECT id FROM sm_connections \
                     WHERE environment = ?1 AND provider_config_key = ?2",
                )?
                .query_map(params![env, key], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            delete_connections(&tx, &ids)?;
            tx.commit()?;
            Ok(true)
        })
        .await
    }

    async fn upsert_connection(&self, row: ConnectionRow) -> Result<Upserted> {
        let new = ConnectionDb::from_row(&row)?;
        self.run(move |c| {
            let tx = write_tx(c)?;
            // The write lock holds from here to the commit, so this check
            // and the write see the same row.
            let created = select_connection_by_key(
                &tx,
                &new.environment,
                &new.provider_config_key,
                &new.connection_id,
            )?
            .is_none();
            // A replace keeps `id` and `created_at`, and adds one to
            // `version`.
            let stored = tx.query_row(
                &format!(
                    "INSERT INTO sm_connections ({CONNECTION_COLUMNS}) VALUES \
                     (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                     ?17, ?18, ?19, ?20, ?21) \
                     ON CONFLICT (environment, provider_config_key, connection_id) DO UPDATE \
                     SET {}, version = sm_connections.version + 1 \
                     RETURNING {CONNECTION_COLUMNS}",
                    rows::connection_change(true)
                ),
                new.params().as_slice(),
                read_connection,
            )?;
            tx.commit()?;
            Ok(Upserted {
                row: stored,
                created,
            })
        })
        .await
    }

    async fn get_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        let (env, key, cid) = (env.to_owned(), key.to_owned(), connection_id.to_owned());
        self.run(move |c| select_connection_by_key(c, &env, &key, &cid))
            .await
    }

    async fn list_connections(
        &self,
        env: &str,
        filter: &ConnectionFilter,
    ) -> Result<Vec<ConnectionRow>> {
        let mut sql =
            format!("SELECT {CONNECTION_COLUMNS} FROM sm_connections WHERE environment = ?");
        let mut args: Vec<String> = vec![env.to_owned()];
        for (column, value) in [
            ("provider_config_key", &filter.provider_config_key),
            ("connection_id", &filter.connection_id),
            ("end_user_id", &filter.end_user_id),
        ] {
            if let Some(v) = value {
                // `column` is fixed text, so this SQL has no caller input.
                sql.push_str(" AND ");
                sql.push_str(column);
                sql.push_str(" = ?");
                args.push(v.clone());
            }
        }
        sql.push_str(" ORDER BY created_at, id");
        self.run(move |c| {
            c.prepare(&sql)?
                .query_map(rusqlite::params_from_iter(args), read_connection)?
                .collect()
        })
        .await
    }

    async fn update_connection(&self, row: &ConnectionRow) -> Result<Option<ConnectionRow>> {
        let new = ConnectionDb::from_row(row)?;
        self.run(move |c| {
            // `?21` is the version that the caller read. Parameter `?14`
            // (`created_at`) is not used: a write keeps it.
            c.query_row(
                &format!(
                    "UPDATE sm_connections SET {}, version = ?21 + 1 \
                     WHERE id = ?1 AND version = ?21 RETURNING {CONNECTION_COLUMNS}",
                    rows::connection_change(false)
                ),
                new.params().as_slice(),
                read_connection,
            )
            .optional()
        })
        .await
    }

    async fn touch_connection(&self, id: Uuid, at: DateTime<Utc>) -> Result<()> {
        self.run(move |c| {
            c.execute(
                "UPDATE sm_connections SET last_fetched_at = ?2 WHERE id = ?1",
                params![id.to_string(), us(at)],
            )
        })
        .await?;
        Ok(())
    }

    async fn delete_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        let (env, key, cid) = (env.to_owned(), key.to_owned(), connection_id.to_owned());
        self.run(move |c| {
            let tx = write_tx(c)?;
            let found = select_connection_by_key(&tx, &env, &key, &cid)?;
            if let Some(r) = &found {
                delete_connections(&tx, &[r.id.to_string()])?;
            }
            tx.commit()?;
            Ok(found)
        })
        .await
    }

    async fn connections_expiring_before(
        &self,
        before: DateTime<Utc>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
    ) -> Result<Vec<ConnectionRow>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.run(move |c| {
            let base = format!(
                "SELECT {CONNECTION_COLUMNS} FROM sm_connections \
                 WHERE refresh_exhausted = 0 AND credentials_expires_at < ?1"
            );
            let order = "ORDER BY credentials_expires_at, id LIMIT ?2";
            if let Some((t, id)) = after {
                c.prepare(&format!(
                    "{base} AND (credentials_expires_at > ?3 \
                     OR (credentials_expires_at = ?3 AND id > ?4)) {order}"
                ))?
                .query_map(
                    params![us(before), limit, us(t), id.to_string()],
                    read_connection,
                )?
                .collect()
            } else {
                c.prepare(&format!("{base} {order}"))?
                    .query_map(params![us(before), limit], read_connection)?
                    .collect()
            }
        })
        .await
    }

    async fn try_lease(
        &self,
        name: &str,
        owner: &str,
        now: DateTime<Utc>,
        ttl_ms: u64,
    ) -> Result<bool> {
        let ttl = chrono::Duration::milliseconds(i64::try_from(ttl_ms).unwrap_or(i64::MAX));
        let until = now
            .checked_add_signed(ttl)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        let (name, owner) = (name.to_owned(), owner.to_owned());
        self.run(move |c| {
            c.query_row(
                "INSERT INTO sm_leases (name, owner, until) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (name) DO UPDATE SET owner = excluded.owner, until = excluded.until \
                 WHERE sm_leases.owner = excluded.owner OR sm_leases.until <= ?4 \
                 RETURNING name",
                params![name, owner, us(until), us(now)],
                |_| Ok(()),
            )
            .optional()
            .map(|got| got.is_some())
        })
        .await
    }

    async fn release_lease(&self, name: &str, owner: &str) -> Result<()> {
        let (name, owner) = (name.to_owned(), owner.to_owned());
        self.run(move |c| {
            c.execute(
                "DELETE FROM sm_leases WHERE name = ?1 AND owner = ?2",
                params![name, owner],
            )
        })
        .await?;
        Ok(())
    }

    async fn insert_connect_session(&self, row: ConnectSessionRow) -> Result<()> {
        let data = rows::json(&SessionData::of(&row))?;
        self.run(move |c| {
            c.execute(
                &format!(
                    "INSERT INTO sm_connect_sessions ({SESSION_COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
                ),
                params![
                    row.id.to_string(),
                    row.environment,
                    row.token_hash,
                    data,
                    us(row.created_at),
                    us(row.expires_at),
                ],
            )
        })
        .await?;
        Ok(())
    }

    async fn get_connect_session(&self, token_hash: &str) -> Result<Option<ConnectSessionRow>> {
        let token_hash = token_hash.to_owned();
        self.run(move |c| {
            c.query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sm_connect_sessions WHERE token_hash = ?1"),
                [token_hash],
                read_session,
            )
            .optional()
        })
        .await
    }

    async fn get_connect_session_by_id(&self, id: Uuid) -> Result<Option<ConnectSessionRow>> {
        self.run(move |c| {
            c.query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sm_connect_sessions WHERE id = ?1"),
                [id.to_string()],
                read_session,
            )
            .optional()
        })
        .await
    }

    async fn delete_connect_session(&self, id: Uuid) -> Result<bool> {
        let n = self
            .run(move |c| {
                c.execute(
                    "DELETE FROM sm_connect_sessions WHERE id = ?1",
                    [id.to_string()],
                )
            })
            .await?;
        Ok(n > 0)
    }

    async fn insert_oauth_session(&self, row: OAuthSessionRow) -> Result<()> {
        let config = rows::json(&row.connection_config)?;
        self.run(move |c| {
            c.execute(
                &format!(
                    "INSERT INTO sm_oauth_sessions ({OAUTH_COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
                ),
                params![
                    row.state_hash,
                    row.environment,
                    row.provider_config_key,
                    row.connection_id,
                    row.connect_session_id.map(|id| id.to_string()),
                    row.code_verifier,
                    config,
                    row.redirect_uri,
                    us(row.created_at),
                    us(row.expires_at),
                ],
            )
        })
        .await?;
        Ok(())
    }

    async fn take_oauth_session(&self, state_hash: &str) -> Result<Option<OAuthSessionRow>> {
        let state_hash = state_hash.to_owned();
        self.run(move |c| {
            c.query_row(
                &format!(
                    "DELETE FROM sm_oauth_sessions WHERE state_hash = ?1 RETURNING {OAUTH_COLUMNS}"
                ),
                [state_hash],
                read_oauth,
            )
            .optional()
        })
        .await
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64> {
        let n = self
            .run(move |c| {
                let tx = write_tx(c)?;
                let now = us(now);
                let a = tx.execute(
                    "DELETE FROM sm_connect_sessions WHERE expires_at <= ?1",
                    [now],
                )?;
                let b = tx.execute(
                    "DELETE FROM sm_oauth_sessions WHERE expires_at <= ?1",
                    [now],
                )?;
                tx.execute("DELETE FROM sm_leases WHERE until <= ?1", [now])?;
                tx.commit()?;
                Ok(a + b)
            })
            .await?;
        Ok(u64::try_from(n).unwrap_or(u64::MAX))
    }

    async fn upsert_records(
        &self,
        connection: Uuid,
        model: &str,
        rows: Vec<RecordWrite>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        let model = model.to_owned();
        self.run(move |c| {
            let tx = write_tx(c)?;
            if !connection_exists(&tx, connection)? {
                return Ok(None);
            }
            let conn = connection.to_string();
            let at = us(write_time(&tx, &conn, &model, now)?);
            let mut done = Vec::with_capacity(rows.len());
            for w in &rows {
                let change = upsert_record(&tx, &conn, &model, w, at)?;
                done.push((w.external_id.clone(), change));
            }
            tx.commit()?;
            Ok(Some(done))
        })
        .await?
        .ok_or_else(|| unknown_connection(connection))
    }

    async fn delete_records(
        &self,
        connection: Uuid,
        model: &str,
        ids: Vec<(Uuid, String)>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        let model = model.to_owned();
        self.run(move |c| {
            let tx = write_tx(c)?;
            if !connection_exists(&tx, connection)? {
                return Ok(None);
            }
            let conn = connection.to_string();
            let at = us(write_time(&tx, &conn, &model, now)?);
            let mut done = Vec::with_capacity(ids.len());
            for (id, external) in ids {
                let id = id.to_string();
                let change = classify_delete(record_state(&tx, &conn, &model, &id)?.as_ref());
                if change == Change::Deleted {
                    tx.execute(
                        "UPDATE sm_records SET deleted_at = ?4, updated_at = ?4 \
                         WHERE connection = ?1 AND model = ?2 AND id = ?3",
                        params![conn, model, id, at],
                    )?;
                }
                done.push((external, change));
            }
            tx.commit()?;
            Ok(Some(done))
        })
        .await?
        .ok_or_else(|| unknown_connection(connection))
    }

    async fn list_records(&self, connection: Uuid, query: &RecordQuery) -> Result<Vec<RecordRow>> {
        let mut sql =
            format!("SELECT {RECORD_COLUMNS} FROM sm_records WHERE connection = ? AND model = ?");
        let mut args: Vec<Box<dyn ToSql + Send>> = vec![
            Box::new(connection.to_string()),
            Box::new(query.model.clone()),
        ];
        if let Some(cur) = query.cursor {
            sql.push_str(" AND (updated_at > ? OR (updated_at = ? AND id > ?))");
            args.push(Box::new(cur.updated_at_us));
            args.push(Box::new(cur.updated_at_us));
            args.push(Box::new(cur.id.to_string()));
        }
        if let Some(t) = query.modified_after_us {
            sql.push_str(" AND updated_at >= ?");
            args.push(Box::new(t));
        }
        if !query.external_ids.is_empty() {
            let marks = vec!["?"; query.external_ids.len()].join(", ");
            sql.push_str(" AND external_id IN (");
            sql.push_str(&marks);
            sql.push(')');
            for e in &query.external_ids {
                args.push(Box::new(e.clone()));
            }
        }
        if !query.filter.is_empty() {
            // The parts are fixed text, so this SQL has no caller input.
            let parts: Vec<&str> = query
                .filter
                .iter()
                .map(|a| match a {
                    LastAction::Deleted => "(deleted_at IS NOT NULL)",
                    LastAction::Added => "(deleted_at IS NULL AND created_at = updated_at)",
                    LastAction::Updated => "(deleted_at IS NULL AND created_at <> updated_at)",
                })
                .collect();
            sql.push_str(" AND (");
            sql.push_str(&parts.join(" OR "));
            sql.push(')');
        }
        sql.push_str(" ORDER BY updated_at, id LIMIT ?");
        args.push(Box::new(i64::from(query.limit) + 1));
        self.run(move |c| {
            c.prepare(&sql)?
                .query_map(
                    rusqlite::params_from_iter(args.iter().map(|a| a.as_ref() as &dyn ToSql)),
                    read_record,
                )?
                .collect()
        })
        .await
    }

    async fn put_sync_state(&self, row: SyncStateRow) -> Result<()> {
        let id = row.connection;
        let written = self
            .run(move |c| {
                // `paused` belongs to `set_sync_paused`: keep the stored value.
                write_sync_state(c, &row, |old| SyncData {
                    paused: old.paused,
                    ..SyncData::of(&row)
                })
            })
            .await?;
        if written {
            Ok(())
        } else {
            Err(unknown_connection(id))
        }
    }

    async fn set_sync_paused(&self, row: SyncStateRow) -> Result<()> {
        let id = row.connection;
        let written = self
            .run(move |c| {
                let paused = row.paused;
                write_sync_state(c, &row, |old| SyncData { paused, ..old })
            })
            .await?;
        if written {
            Ok(())
        } else {
            Err(unknown_connection(id))
        }
    }

    async fn get_sync_state(
        &self,
        connection: Uuid,
        sync_name: &str,
    ) -> Result<Option<SyncStateRow>> {
        let name = sync_name.to_owned();
        self.run(move |c| {
            c.query_row(
                "SELECT data FROM sm_sync_states WHERE connection = ?1 AND sync_name = ?2",
                params![connection.to_string(), name],
                |r| get_json::<SyncData>(r, 0),
            )
            .optional()
            .map(|d| d.map(|d| d.into_row(connection, name)))
        })
        .await
    }

    async fn list_sync_states(&self, connection: Uuid) -> Result<Vec<SyncStateRow>> {
        self.run(move |c| {
            c.prepare(
                "SELECT sync_name, data FROM sm_sync_states \
                 WHERE connection = ?1 ORDER BY sync_name",
            )?
            .query_map([connection.to_string()], |r| {
                let name: String = r.get(0)?;
                let d: SyncData = get_json(r, 1)?;
                Ok(d.into_row(connection, name))
            })?
            .collect()
        })
        .await
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
        for table in [
            "sm_integrations",
            "sm_connections",
            "sm_leases",
            "sm_connect_sessions",
            "sm_oauth_sessions",
            "sm_records",
            "sm_sync_states",
        ] {
            assert!(
                sql.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
                "{table}"
            );
        }
    }

    #[test]
    fn records_and_sync_states_cascade_from_connections() {
        let sql = full_migrations_sql();
        for table in ["sm_records", "sm_sync_states"] {
            let at = sql
                .find(&format!("CREATE TABLE IF NOT EXISTS {table}"))
                .unwrap();
            let end = at + sql[at..].find(';').unwrap();
            assert!(
                sql[at..end].contains("REFERENCES sm_connections (id) ON DELETE CASCADE"),
                "{table}"
            );
        }
    }

    #[test]
    fn each_change_parameter_is_its_column() {
        let columns: Vec<&str> = CONNECTION_COLUMNS.split(", ").collect();
        assert_eq!(columns.len(), 21);
        for (column, n) in rows::CHANGE_COLUMNS {
            assert_eq!(columns[usize::from(n) - 1], column);
        }
        for (n, column) in [(1, "id"), (14, "created_at"), (21, "version")] {
            assert_eq!(columns[n - 1], column);
        }
    }

    #[test]
    fn uuid_text_order_is_byte_order() {
        let mut ids: Vec<Uuid> = (0..64).map(|_| Uuid::new_v4()).collect();
        ids.push(Uuid::nil());
        ids.push(Uuid::max());
        let mut by_bytes = ids.clone();
        by_bytes.sort_unstable();
        let mut by_text = ids;
        by_text.sort_unstable_by_key(Uuid::to_string);
        assert_eq!(by_bytes, by_text);
    }

    #[test]
    fn a_time_out_of_range_is_an_error() {
        assert!(time_of(0, i64::MAX).is_err());
        assert!(time_of(0, 0).is_ok());
    }
}
