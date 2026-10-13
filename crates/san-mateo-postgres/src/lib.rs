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
use diesel::prelude::*;
use diesel::sql_types::{Bool, Text, Timestamptz};
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection};
use san_mateo::records::{
    Change, LastAction, RecordQuery, RecordState, classify_delete, classify_upsert,
};
use san_mateo::store::{
    ConnectSessionRow, ConnectionFilter, ConnectionRow, IntegrationRow, OAuthSessionRow, RecordRow,
    RecordWrite, Store, SyncStateRow, Upserted,
};
use san_mateo::{Error, Result};
use serde_json::Value;
use uuid::Uuid;

mod rows;
mod schema;

use rows::RecordDb;

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
    MIGRATIONS
        .iter()
        .map(|(_, sql)| *sql)
        .collect::<Vec<_>>()
        .join("\n")
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

/// Maps a database error. The message stays in the store error, which the
/// API does not show to callers.
fn db(e: impl std::fmt::Display) -> Error {
    Error::Store(format!("postgres: {e}"))
}

fn from_micros(us: i64) -> Result<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_micros(us)
        .ok_or_else(|| Error::Invalid("a time is out of range".to_owned()))
}

/// The advisory lock key of the migration runner.
const MIGRATION_LOCK: i64 = 0x5341_4e4d_4154_454f;

/// One row of the lease statement. Diesel reads the column; the code only
/// counts the rows.
#[derive(QueryableByName)]
struct LeaseName {
    #[diesel(sql_type = Text)]
    #[allow(dead_code)]
    name: String,
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
    /// [`san_mateo::Error::Store`] when the URL is not a Postgres URL or the
    /// pool does not build.
    pub fn connect(database_url: &str, max_size: usize) -> Result<Self> {
        let parsed =
            url::Url::parse(database_url).map_err(|_| db("the database URL is not valid"))?;
        if !matches!(parsed.scheme(), "postgres" | "postgresql") {
            return Err(db("the database URL must use postgres://"));
        }
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url);
        let pool = Pool::builder(manager)
            .max_size(max_size.max(1))
            .build()
            .map_err(db)?;
        Ok(Self { pool })
    }

    /// Returns the pool.
    #[must_use]
    pub const fn pool(&self) -> &Pool {
        &self.pool
    }

    async fn conn(
        &self,
    ) -> Result<deadpool::managed::Object<AsyncDieselConnectionManager<AsyncPgConnection>>> {
        self.pool.get().await.map_err(db)
    }

    /// Applies the migrations that are not applied yet. Returns the versions
    /// that it applied. An advisory lock makes concurrent calls safe.
    ///
    /// # Errors
    ///
    /// [`san_mateo::Error::Store`] when a migration fails.
    pub async fn migrate(&self) -> Result<Vec<&'static str>> {
        use schema::sm_schema_migrations::dsl as m;
        let mut conn = self.conn().await?;
        conn.transaction(async |c| {
            c.batch_execute(&format!("SELECT pg_advisory_xact_lock({MIGRATION_LOCK})")).await?;
            c.batch_execute(
                "CREATE TABLE IF NOT EXISTS sm_schema_migrations (version TEXT PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
            )
            .await?;
            let applied: Vec<String> = m::sm_schema_migrations.select(m::version).load(c).await?;
            let mut done = Vec::new();
            for (version, sql) in MIGRATIONS {
                if applied.iter().any(|a| a == version) {
                    continue;
                }
                c.batch_execute(sql).await?;
                diesel::insert_into(m::sm_schema_migrations)
                    .values(m::version.eq(*version))
                    .execute(c)
                    .await?;
                done.push(*version);
            }
            Ok::<_, diesel::result::Error>(done)
        })
        .await
        .map_err(db)
    }
}

/// Deletes connections and their records and sync states, in one transaction.
async fn delete_connections(c: &mut AsyncPgConnection, ids: &[Uuid]) -> diesel::QueryResult<()> {
    use schema::{sm_connections, sm_records, sm_sync_states};
    if ids.is_empty() {
        return Ok(());
    }
    diesel::delete(sm_records::table.filter(sm_records::connection.eq_any(ids)))
        .execute(c)
        .await?;
    diesel::delete(sm_sync_states::table.filter(sm_sync_states::connection.eq_any(ids)))
        .execute(c)
        .await?;
    diesel::delete(sm_connections::table.filter(sm_connections::id.eq_any(ids)))
        .execute(c)
        .await?;
    Ok(())
}

/// Reads the state of a record and locks its row.
async fn record_state(
    c: &mut AsyncPgConnection,
    connection: Uuid,
    model: &str,
    id: Uuid,
) -> diesel::QueryResult<Option<RecordState>> {
    use schema::sm_records::dsl as r;
    let found: Option<(String, Option<DateTime<Utc>>)> = r::sm_records
        .filter(r::connection.eq(connection))
        .filter(r::model.eq(model))
        .filter(r::id.eq(id))
        .select((r::data_hash, r::deleted_at))
        .for_update()
        .first(c)
        .await
        .optional()?;
    Ok(found.map(|(data_hash, deleted_at)| RecordState {
        data_hash,
        deleted: deleted_at.is_some(),
    }))
}

/// Writes one record in a transaction. Returns its change.
async fn upsert_record(
    c: &mut AsyncPgConnection,
    connection: Uuid,
    model: &str,
    w: &RecordWrite,
    now: DateTime<Utc>,
) -> diesel::QueryResult<Change> {
    use schema::sm_records::dsl as r;
    let target = r::sm_records
        .filter(r::connection.eq(connection))
        .filter(r::model.eq(model))
        .filter(r::id.eq(w.id));
    // Two tries: a concurrent insert of the same record can win the first.
    for _ in 0..2 {
        let state = record_state(c, connection, model, w.id).await?;
        let change = classify_upsert(state.as_ref(), &w.data_hash);
        match (change, state) {
            (Change::Added, None) => {
                let row = RecordDb {
                    connection,
                    model: model.to_owned(),
                    id: w.id,
                    external_id: w.external_id.clone(),
                    data: w.data.clone(),
                    data_hash: w.data_hash.clone(),
                    created_at: now,
                    updated_at: now,
                    deleted_at: None,
                };
                let n = diesel::insert_into(r::sm_records)
                    .values(&row)
                    .on_conflict_do_nothing()
                    .execute(c)
                    .await?;
                if n == 1 {
                    return Ok(Change::Added);
                }
            }
            (Change::Added, Some(_)) => {
                diesel::update(target)
                    .set((
                        r::external_id.eq(&w.external_id),
                        r::data.eq(&w.data),
                        r::data_hash.eq(&w.data_hash),
                        r::created_at.eq(now),
                        r::updated_at.eq(now),
                        r::deleted_at.eq(None::<DateTime<Utc>>),
                    ))
                    .execute(c)
                    .await?;
                return Ok(Change::Added);
            }
            (Change::Updated, _) => {
                diesel::update(target)
                    .set((
                        r::data.eq(&w.data),
                        r::data_hash.eq(&w.data_hash),
                        r::updated_at.eq(now),
                    ))
                    .execute(c)
                    .await?;
                return Ok(Change::Updated);
            }
            (other, _) => return Ok(other),
        }
    }
    Err(diesel::result::Error::RollbackTransaction)
}

#[async_trait]
impl Store for PgStore {
    async fn insert_integration(&self, row: IntegrationRow) -> Result<()> {
        use schema::sm_integrations::dsl as i;
        let mut c = self.conn().await?;
        let n = diesel::insert_into(i::sm_integrations)
            .values(rows::IntegrationDb::from_row(&row)?)
            .on_conflict_do_nothing()
            .execute(&mut c)
            .await
            .map_err(db)?;
        if n == 0 {
            return Err(Error::Conflict(format!(
                "integration {} exists",
                row.unique_key
            )));
        }
        Ok(())
    }

    async fn update_integration(&self, row: IntegrationRow) -> Result<bool> {
        use schema::sm_integrations::dsl as i;
        let mut c = self.conn().await?;
        let n = diesel::update(
            i::sm_integrations
                .filter(i::environment.eq(&row.environment))
                .filter(i::unique_key.eq(&row.unique_key)),
        )
        .set(rows::IntegrationDb::from_row(&row)?)
        .execute(&mut c)
        .await
        .map_err(db)?;
        Ok(n > 0)
    }

    async fn get_integration(&self, env: &str, key: &str) -> Result<Option<IntegrationRow>> {
        use schema::sm_integrations::dsl as i;
        let mut c = self.conn().await?;
        i::sm_integrations
            .filter(i::environment.eq(env))
            .filter(i::unique_key.eq(key))
            .select(rows::IntegrationDb::as_select())
            .first(&mut c)
            .await
            .optional()
            .map_err(db)?
            .map(rows::IntegrationDb::into_row)
            .transpose()
    }

    async fn list_integrations(&self, env: &str) -> Result<Vec<IntegrationRow>> {
        use schema::sm_integrations::dsl as i;
        let mut c = self.conn().await?;
        let rows: Vec<rows::IntegrationDb> = i::sm_integrations
            .filter(i::environment.eq(env))
            .order(i::unique_key.asc())
            .select(rows::IntegrationDb::as_select())
            .load(&mut c)
            .await
            .map_err(db)?;
        rows.into_iter()
            .map(rows::IntegrationDb::into_row)
            .collect()
    }

    async fn delete_integration(&self, env: &str, key: &str) -> Result<bool> {
        use schema::{sm_connections::dsl as cn, sm_integrations::dsl as i};
        let mut c = self.conn().await?;
        let (env, key) = (env.to_owned(), key.to_owned());
        c.transaction(async |c| {
            let n = diesel::delete(
                i::sm_integrations
                    .filter(i::environment.eq(&env))
                    .filter(i::unique_key.eq(&key)),
            )
            .execute(c)
            .await?;
            if n == 0 {
                return Ok(false);
            }
            let ids: Vec<Uuid> = cn::sm_connections
                .filter(cn::environment.eq(&env))
                .filter(cn::provider_config_key.eq(&key))
                .select(cn::id)
                .load(c)
                .await?;
            delete_connections(c, &ids).await?;
            Ok::<_, diesel::result::Error>(true)
        })
        .await
        .map_err(db)
    }

    async fn upsert_connection(&self, row: ConnectionRow) -> Result<Upserted> {
        use schema::sm_connections::dsl as cn;
        let new = rows::ConnectionDb::from_row(&row)?;
        let mut c = self.conn().await?;
        // Two tries: a concurrent delete can remove the row between the steps.
        for _ in 0..2 {
            let inserted: Option<rows::ConnectionDb> = diesel::insert_into(cn::sm_connections)
                .values(&new)
                .on_conflict((cn::environment, cn::provider_config_key, cn::connection_id))
                .do_nothing()
                .returning(rows::ConnectionDb::as_returning())
                .get_result(&mut c)
                .await
                .optional()
                .map_err(db)?;
            if let Some(r) = inserted {
                return Ok(Upserted {
                    row: r.into_row()?,
                    created: true,
                });
            }
            let updated: Option<rows::ConnectionDb> = diesel::update(
                cn::sm_connections
                    .filter(cn::environment.eq(&new.environment))
                    .filter(cn::provider_config_key.eq(&new.provider_config_key))
                    .filter(cn::connection_id.eq(&new.connection_id)),
            )
            .set((new.change(), cn::version.eq(cn::version + 1)))
            .returning(rows::ConnectionDb::as_returning())
            .get_result(&mut c)
            .await
            .optional()
            .map_err(db)?;
            if let Some(r) = updated {
                return Ok(Upserted {
                    row: r.into_row()?,
                    created: false,
                });
            }
        }
        Err(Error::Store(
            "the connection changed during an upsert".to_owned(),
        ))
    }

    async fn get_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        use schema::sm_connections::dsl as cn;
        let mut c = self.conn().await?;
        cn::sm_connections
            .filter(cn::environment.eq(env))
            .filter(cn::provider_config_key.eq(key))
            .filter(cn::connection_id.eq(connection_id))
            .select(rows::ConnectionDb::as_select())
            .first(&mut c)
            .await
            .optional()
            .map_err(db)?
            .map(rows::ConnectionDb::into_row)
            .transpose()
    }

    async fn list_connections(
        &self,
        env: &str,
        filter: &ConnectionFilter,
    ) -> Result<Vec<ConnectionRow>> {
        use schema::sm_connections::dsl as cn;
        let mut c = self.conn().await?;
        let mut q = cn::sm_connections
            .filter(cn::environment.eq(env))
            .into_boxed();
        if let Some(k) = &filter.provider_config_key {
            q = q.filter(cn::provider_config_key.eq(k));
        }
        if let Some(id) = &filter.connection_id {
            q = q.filter(cn::connection_id.eq(id));
        }
        if let Some(u) = &filter.end_user_id {
            q = q.filter(cn::end_user_id.eq(u));
        }
        let rows: Vec<rows::ConnectionDb> = q
            .order((cn::created_at.asc(), cn::id.asc()))
            .select(rows::ConnectionDb::as_select())
            .load(&mut c)
            .await
            .map_err(db)?;
        rows.into_iter().map(rows::ConnectionDb::into_row).collect()
    }

    async fn update_connection(&self, row: &ConnectionRow) -> Result<Option<ConnectionRow>> {
        use schema::sm_connections::dsl as cn;
        let new = rows::ConnectionDb::from_row(row)?;
        let mut c = self.conn().await?;
        diesel::update(
            cn::sm_connections
                .filter(cn::id.eq(row.id))
                .filter(cn::version.eq(row.version)),
        )
        .set((new.change(), cn::version.eq(row.version + 1)))
        .returning(rows::ConnectionDb::as_returning())
        .get_result(&mut c)
        .await
        .optional()
        .map_err(db)?
        .map(rows::ConnectionDb::into_row)
        .transpose()
    }

    async fn touch_connection(&self, id: Uuid, at: DateTime<Utc>) -> Result<()> {
        use schema::sm_connections::dsl as cn;
        let mut c = self.conn().await?;
        diesel::update(cn::sm_connections.filter(cn::id.eq(id)))
            .set(cn::last_fetched_at.eq(Some(at)))
            .execute(&mut c)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn delete_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        use schema::sm_connections::dsl as cn;
        let mut c = self.conn().await?;
        let (env, key, cid) = (env.to_owned(), key.to_owned(), connection_id.to_owned());
        let found = c
            .transaction(async |c| {
                let found: Option<rows::ConnectionDb> = cn::sm_connections
                    .filter(cn::environment.eq(&env))
                    .filter(cn::provider_config_key.eq(&key))
                    .filter(cn::connection_id.eq(&cid))
                    .select(rows::ConnectionDb::as_select())
                    .for_update()
                    .first(c)
                    .await
                    .optional()?;
                if let Some(r) = &found {
                    delete_connections(c, &[r.id]).await?;
                }
                Ok::<_, diesel::result::Error>(found)
            })
            .await
            .map_err(db)?;
        found.map(rows::ConnectionDb::into_row).transpose()
    }

    async fn connections_expiring_before(
        &self,
        before: DateTime<Utc>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
    ) -> Result<Vec<ConnectionRow>> {
        use schema::sm_connections::dsl as cn;
        let mut c = self.conn().await?;
        let mut q = cn::sm_connections
            .filter(cn::refresh_exhausted.eq(false))
            .filter(cn::credentials_expires_at.lt(before))
            .into_boxed();
        if let Some((t, id)) = after {
            q = q.filter(
                cn::credentials_expires_at
                    .gt(t)
                    .or(cn::credentials_expires_at.eq(t).and(cn::id.gt(id))),
            );
        }
        let rows: Vec<rows::ConnectionDb> = q
            .order((cn::credentials_expires_at.asc(), cn::id.asc()))
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .select(rows::ConnectionDb::as_select())
            .load(&mut c)
            .await
            .map_err(db)?;
        rows.into_iter().map(rows::ConnectionDb::into_row).collect()
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
        let mut c = self.conn().await?;
        let got: Vec<LeaseName> = diesel::sql_query(
            "INSERT INTO sm_leases (name, owner, until) VALUES ($1, $2, $3) \
             ON CONFLICT (name) DO UPDATE SET owner = EXCLUDED.owner, until = EXCLUDED.until \
             WHERE sm_leases.owner = EXCLUDED.owner OR sm_leases.until <= $4 \
             RETURNING name",
        )
        .bind::<Text, _>(name)
        .bind::<Text, _>(owner)
        .bind::<Timestamptz, _>(until)
        .bind::<Timestamptz, _>(now)
        .load(&mut c)
        .await
        .map_err(db)?;
        Ok(!got.is_empty())
    }

    async fn release_lease(&self, name: &str, owner: &str) -> Result<()> {
        use schema::sm_leases::dsl as l;
        let mut c = self.conn().await?;
        diesel::delete(
            l::sm_leases
                .filter(l::name.eq(name))
                .filter(l::owner.eq(owner)),
        )
        .execute(&mut c)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn insert_connect_session(&self, row: ConnectSessionRow) -> Result<()> {
        use schema::sm_connect_sessions::dsl as s;
        let mut c = self.conn().await?;
        diesel::insert_into(s::sm_connect_sessions)
            .values(rows::SessionDb::from_row(&row)?)
            .execute(&mut c)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn get_connect_session(&self, token_hash: &str) -> Result<Option<ConnectSessionRow>> {
        use schema::sm_connect_sessions::dsl as s;
        let mut c = self.conn().await?;
        s::sm_connect_sessions
            .filter(s::token_hash.eq(token_hash))
            .select(rows::SessionDb::as_select())
            .first(&mut c)
            .await
            .optional()
            .map_err(db)?
            .map(rows::SessionDb::into_row)
            .transpose()
    }

    async fn get_connect_session_by_id(&self, id: Uuid) -> Result<Option<ConnectSessionRow>> {
        use schema::sm_connect_sessions::dsl as s;
        let mut c = self.conn().await?;
        s::sm_connect_sessions
            .filter(s::id.eq(id))
            .select(rows::SessionDb::as_select())
            .first(&mut c)
            .await
            .optional()
            .map_err(db)?
            .map(rows::SessionDb::into_row)
            .transpose()
    }

    async fn delete_connect_session(&self, id: Uuid) -> Result<bool> {
        use schema::sm_connect_sessions::dsl as s;
        let mut c = self.conn().await?;
        let n = diesel::delete(s::sm_connect_sessions.filter(s::id.eq(id)))
            .execute(&mut c)
            .await
            .map_err(db)?;
        Ok(n > 0)
    }

    async fn insert_oauth_session(&self, row: OAuthSessionRow) -> Result<()> {
        use schema::sm_oauth_sessions::dsl as o;
        let mut c = self.conn().await?;
        diesel::insert_into(o::sm_oauth_sessions)
            .values(rows::OAuthDb::from_row(row))
            .execute(&mut c)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn take_oauth_session(&self, state_hash: &str) -> Result<Option<OAuthSessionRow>> {
        use schema::sm_oauth_sessions::dsl as o;
        let mut c = self.conn().await?;
        let row: Option<rows::OAuthDb> =
            diesel::delete(o::sm_oauth_sessions.filter(o::state_hash.eq(state_hash)))
                .returning(rows::OAuthDb::as_returning())
                .get_result(&mut c)
                .await
                .optional()
                .map_err(db)?;
        Ok(row.map(rows::OAuthDb::into_row))
    }

    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64> {
        use schema::{
            sm_connect_sessions::dsl as s, sm_leases::dsl as l, sm_oauth_sessions::dsl as o,
        };
        let mut c = self.conn().await?;
        let a = diesel::delete(s::sm_connect_sessions.filter(s::expires_at.le(now)))
            .execute(&mut c)
            .await
            .map_err(db)?;
        let b = diesel::delete(o::sm_oauth_sessions.filter(o::expires_at.le(now)))
            .execute(&mut c)
            .await
            .map_err(db)?;
        diesel::delete(l::sm_leases.filter(l::until.le(now)))
            .execute(&mut c)
            .await
            .map_err(db)?;
        Ok(u64::try_from(a + b).unwrap_or(u64::MAX))
    }

    async fn upsert_records(
        &self,
        connection: Uuid,
        model: &str,
        rows: Vec<RecordWrite>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        let mut c = self.conn().await?;
        let model = model.to_owned();
        c.transaction(async |c| {
            let mut out = Vec::with_capacity(rows.len());
            for w in &rows {
                out.push((
                    w.external_id.clone(),
                    upsert_record(c, connection, &model, w, now).await?,
                ));
            }
            Ok::<_, diesel::result::Error>(out)
        })
        .await
        .map_err(db)
    }

    async fn delete_records(
        &self,
        connection: Uuid,
        model: &str,
        ids: Vec<(Uuid, String)>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        use schema::sm_records::dsl as r;
        let mut c = self.conn().await?;
        let model = model.to_owned();
        c.transaction(async |c| {
            let mut out = Vec::with_capacity(ids.len());
            for (id, external) in &ids {
                let state = record_state(c, connection, &model, *id).await?;
                let change = classify_delete(state.as_ref());
                if change == Change::Deleted {
                    diesel::update(
                        r::sm_records
                            .filter(r::connection.eq(connection))
                            .filter(r::model.eq(&model))
                            .filter(r::id.eq(*id)),
                    )
                    .set((r::deleted_at.eq(Some(now)), r::updated_at.eq(now)))
                    .execute(c)
                    .await?;
                }
                out.push((external.clone(), change));
            }
            Ok::<_, diesel::result::Error>(out)
        })
        .await
        .map_err(db)
    }

    async fn list_records(&self, connection: Uuid, query: &RecordQuery) -> Result<Vec<RecordRow>> {
        use schema::sm_records::dsl as r;
        let mut q = r::sm_records
            .filter(r::connection.eq(connection))
            .filter(r::model.eq(&query.model))
            .into_boxed();
        if let Some(cur) = query.cursor {
            let ts = from_micros(cur.updated_at_us)?;
            q = q.filter(
                r::updated_at
                    .gt(ts)
                    .or(r::updated_at.eq(ts).and(r::id.gt(cur.id))),
            );
        }
        if let Some(us) = query.modified_after_us {
            q = q.filter(r::updated_at.ge(from_micros(us)?));
        }
        if !query.external_ids.is_empty() {
            q = q.filter(r::external_id.eq_any(&query.external_ids));
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
            q = q.filter(diesel::dsl::sql::<Bool>(&parts.join(" OR ")));
        }
        let mut c = self.conn().await?;
        let rows: Vec<rows::RecordDb> = q
            .order((r::updated_at.asc(), r::id.asc()))
            .limit(i64::from(query.limit) + 1)
            .select(rows::RecordDb::as_select())
            .load(&mut c)
            .await
            .map_err(db)?;
        Ok(rows.into_iter().map(rows::RecordDb::into_row).collect())
    }

    async fn put_sync_state(&self, row: SyncStateRow) -> Result<()> {
        use schema::sm_sync_states::dsl as s;
        let data = rows::sync_data(&row)?;
        let mut c = self.conn().await?;
        diesel::insert_into(s::sm_sync_states)
            .values((
                s::connection.eq(row.connection),
                s::sync_name.eq(&row.sync_name),
                s::data.eq(&data),
            ))
            .on_conflict((s::connection, s::sync_name))
            .do_update()
            .set(s::data.eq(&data))
            .execute(&mut c)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn set_sync_paused(&self, _row: SyncStateRow) -> Result<()> {
        Ok(())
    }

    async fn get_sync_state(
        &self,
        connection: Uuid,
        sync_name: &str,
    ) -> Result<Option<SyncStateRow>> {
        use schema::sm_sync_states::dsl as s;
        let mut c = self.conn().await?;
        let found: Option<Value> = s::sm_sync_states
            .filter(s::connection.eq(connection))
            .filter(s::sync_name.eq(sync_name))
            .select(s::data)
            .first(&mut c)
            .await
            .optional()
            .map_err(db)?;
        found
            .map(|d| rows::sync_row(connection, sync_name.to_owned(), d))
            .transpose()
    }

    async fn list_sync_states(&self, connection: Uuid) -> Result<Vec<SyncStateRow>> {
        use schema::sm_sync_states::dsl as s;
        let mut c = self.conn().await?;
        let found: Vec<(String, Value)> = s::sm_sync_states
            .filter(s::connection.eq(connection))
            .order(s::sync_name.asc())
            .select((s::sync_name, s::data))
            .load(&mut c)
            .await
            .map_err(db)?;
        found
            .into_iter()
            .map(|(n, d)| rows::sync_row(connection, n, d))
            .collect()
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
    fn connect_rejects_a_bad_url() {
        assert!(PgStore::connect("not a url", 2).is_err());
    }
}
