//! Connections and token refresh.
//!
//! A refresh has four guards:
//!
//! 1. Single flight: one refresh lock for each connection in this process.
//! 2. A store lease (`refresh:<env>:<integration>:<connection id>`) for other
//!    processes. A token call stops before the lease ends.
//! 3. A version check (compare and set) on each write.
//! 4. A fence: the write keeps the result only when the stored credentials
//!    are the credentials that the refresh started from.
//!
//! A refresh runs in a task of the service. When the caller goes away, the
//! task still writes the new tokens.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{
    Connection, ConnectionPatch, ConnectionSummary, GetOptions, RefreshReport, SanMateo, invalid,
};
use crate::credentials::{Credentials, OAuth2Credentials};
use crate::error::{Error, Result};
use crate::model::ConnectionKey;
use crate::policy::{self, Gate};
use crate::provider::{AuthMode, Provider};
use crate::store::{ConnectionFilter, ConnectionRow};
use crate::webhook::{AuthEvent, AuthOperation, ErrorPayload, WebhookEvent};

/// The largest number of compare-and-set tries for one write.
const CAS_TRIES: usize = 5;
/// The time between checks while another process holds the refresh lease.
const LEASE_POLL: Duration = Duration::from_millis(250);
/// The largest number of tries to write the result of a refresh.
const WRITE_TRIES: u32 = 10;
/// The first wait after a store error in a refresh write.
const WRITE_BACKOFF: Duration = Duration::from_millis(50);
/// The longest wait after a store error in a refresh write.
const WRITE_BACKOFF_MAX: Duration = Duration::from_secs(1);
/// The largest page of the refresh sweep.
const SWEEP_PAGE: usize = 100;

/// Why a refresh starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshMode {
    /// The token expires inside the provider buffer.
    IfExpired,
    /// The token expires before this instant (the sweep).
    Before(DateTime<Utc>),
    /// The provider did not accept the token (HTTP 401). The gate applies.
    Rejected,
    /// The caller asks for a refresh now. The gate does not apply.
    Force,
}

impl RefreshMode {
    /// Returns `true` for a mode that also refreshes a token that does not
    /// expire soon.
    const fn eager(self) -> bool {
        matches!(self, Self::Rejected | Self::Force)
    }
}

/// The result of one try to write the result of a refresh.
enum Write {
    /// The row is stored.
    Stored(Box<ConnectionRow>),
    /// Other credentials are stored now. The result is dropped.
    Superseded,
    /// Another write changed the row first. Try again.
    Lost,
}

/// Returns `true` when less than half of the token life has passed since the
/// last refresh. Such a token is valid for at least half of its life.
fn in_first_half(row: &ConnectionRow, exp_ms: u64, now_ms: u64) -> bool {
    row.last_refresh_success
        .map(crate::clock::to_ms)
        .is_some_and(|r| exp_ms > r && now_ms < r + (exp_ms - r) / 2)
}

fn as_map(v: &Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    }
}

/// Returns `true` when the credentials can get a new token.
fn refreshable(c: &Credentials) -> bool {
    match c {
        Credentials::OAuth2(o) => o.refresh_token.as_ref().is_some_and(|t| !t.is_empty()),
        Credentials::OAuth2ClientCredentials(_) => true,
        _ => false,
    }
}

impl SanMateo {
    pub(crate) fn to_connection(row: ConnectionRow, credentials: Credentials) -> Connection {
        Connection {
            id: row.id,
            environment: row.environment,
            provider_config_key: row.provider_config_key,
            connection_id: row.connection_id,
            provider: row.provider,
            credentials,
            connection_config: as_map(&row.connection_config),
            metadata: as_map(&row.metadata),
            tags: row.tags,
            end_user: row.end_user,
            webhook_url_override: row.webhook_url_override,
            created_at: row.created_at,
            updated_at: row.updated_at,
            last_fetched_at: row.last_fetched_at,
        }
    }

    /// Returns the template variables of a connection.
    pub(crate) fn vars_for(
        credentials: &Credentials,
        cfg: &Map<String, Value>,
        metadata: &Map<String, Value>,
    ) -> Value {
        let mut vars = credentials.template_vars();
        if let Value::Object(m) = &mut vars {
            m.insert("connectionConfig".to_owned(), Value::Object(cfg.clone()));
            m.insert("connection_config".to_owned(), Value::Object(cfg.clone()));
            m.insert("metadata".to_owned(), Value::Object(metadata.clone()));
        }
        vars
    }

    pub(crate) async fn connection_row(
        &self,
        env: &str,
        key: &ConnectionKey,
    ) -> Result<ConnectionRow> {
        self.inner
            .store
            .get_connection(env, &key.provider_config_key, &key.connection_id)
            .await?
            .ok_or_else(|| Error::UnknownConnection(key.connection_id.clone()))
    }

    pub(crate) fn open_credentials(&self, row: &ConnectionRow) -> Result<Credentials> {
        self.inner.keys.open_json(
            &row.credentials,
            &Self::connection_aad(
                &row.environment,
                &row.provider_config_key,
                &row.connection_id,
            ),
        )
    }

    /// Reads, changes and writes a connection with compare and set.
    async fn update_row(
        &self,
        env: &str,
        key: &ConnectionKey,
        mut change: impl FnMut(&mut ConnectionRow) -> Result<()>,
    ) -> Result<ConnectionRow> {
        for _ in 0..CAS_TRIES {
            let mut row = self.connection_row(env, key).await?;
            change(&mut row)?;
            if let Some(stored) = self.inner.store.update_connection(&row).await? {
                return Ok(stored);
            }
        }
        Err(Error::Store(
            "the connection changed too often during a write".to_owned(),
        ))
    }

    /// Lists connections with no credentials.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn list_connections(
        &self,
        env: &str,
        filter: &ConnectionFilter,
    ) -> Result<Vec<ConnectionSummary>> {
        let rows = self.inner.store.list_connections(env, filter).await?;
        Ok(rows
            .into_iter()
            .map(|r| ConnectionSummary {
                id: r.id,
                metadata: as_map(&r.metadata),
                provider_config_key: r.provider_config_key,
                connection_id: r.connection_id,
                provider: r.provider,
                tags: r.tags,
                end_user: r.end_user,
                created_at: r.created_at,
                refresh_exhausted: r.refresh_exhausted,
            })
            .collect())
    }

    /// Reads a connection with fresh credentials.
    ///
    /// When a refresh that is not forced fails, the current credentials come
    /// back while they are valid. The failure is stored as usual.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`], [`Error::RefreshCooldown`],
    /// [`Error::RefreshExhausted`], [`Error::Provider`] with `refresh_failed`.
    pub async fn get_connection(
        &self,
        env: &str,
        key: &ConnectionKey,
        options: GetOptions,
    ) -> Result<Connection> {
        let row = self.connection_row(env, key).await?;
        let creds = self.open_credentials(&row)?;
        let mode = if options.force_refresh {
            Some(RefreshMode::Force)
        } else if options.refresh_if_needed {
            Some(RefreshMode::IfExpired)
        } else {
            None
        };
        let (mut row, creds) = match mode {
            Some(m) if self.needs_refresh(&row, &creds, m)? => {
                match self.refresh(env, key, m).await {
                    Ok(fresh) => fresh,
                    Err(e)
                        if m == RefreshMode::IfExpired
                            && !matches!(e, Error::UnknownConnection(_))
                            && creds.expires_at().is_some_and(|t| t > self.now()) =>
                    {
                        tracing::debug!(
                            code = e.code(),
                            "refresh failed; the token is still valid"
                        );
                        (row, creds)
                    }
                    Err(e) => return Err(e),
                }
            }
            _ => (row, creds),
        };
        let now = self.now();
        // Best effort. The read time does not change the row version.
        if let Err(e) = self.inner.store.touch_connection(row.id, now).await {
            tracing::debug!(code = e.code(), "the read time was not stored");
        }
        row.last_fetched_at = Some(now);
        Ok(Self::to_connection(row, creds))
    }

    fn provider_of(&self, row: &ConnectionRow) -> Result<Arc<Provider>> {
        self.catalog().require(&row.provider)
    }

    fn needs_refresh(
        &self,
        row: &ConnectionRow,
        creds: &Credentials,
        mode: RefreshMode,
    ) -> Result<bool> {
        if !refreshable(creds) {
            return Ok(false);
        }
        let Some(expires_at) = creds.expires_at() else {
            return Ok(mode.eager());
        };
        let exp = crate::clock::to_ms(expires_at);
        let now = self.now_ms();
        Ok(match mode {
            RefreshMode::Rejected | RefreshMode::Force => true,
            RefreshMode::Before(t) => exp < crate::clock::to_ms(t) && !in_first_half(row, exp, now),
            RefreshMode::IfExpired => {
                let buffer = self.provider_of(row)?.refresh_buffer_ms();
                policy::token_expired(now, exp, buffer) && !in_first_half(row, exp, now)
            }
        })
    }

    /// Returns `true` when the row needs no refresh for this call. For an
    /// eager mode, a refresh that started after the call is enough.
    fn refresh_done(
        &self,
        row: &ConnectionRow,
        creds: &Credentials,
        mode: RefreshMode,
        started: DateTime<Utc>,
    ) -> Result<bool> {
        if mode.eager() {
            Ok(row.last_refresh_success.is_some_and(|t| t >= started))
        } else {
            Ok(!self.needs_refresh(row, creds, mode)?)
        }
    }

    fn flight(&self, name: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self
            .inner
            .flights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Drop locks that no task holds, so the map does not grow without end.
        map.retain(|_, l| Arc::strong_count(l) > 1);
        map.entry(name.to_owned()).or_default().clone()
    }

    /// Refreshes the credentials of a connection. See the module docs.
    pub(crate) async fn refresh(
        &self,
        env: &str,
        key: &ConnectionKey,
        mode: RefreshMode,
    ) -> Result<(ConnectionRow, Credentials)> {
        // Take the time before the lock, so that waiters can coalesce.
        let started = self.now();
        let san = self.clone();
        let (env, key) = (env.to_owned(), key.clone());
        self.inner
            .tasks
            .spawn(async move { san.refresh_task(&env, &key, mode, started).await })
            .await
            .map_err(|_| Error::Internal("the refresh task stopped".to_owned()))?
    }

    async fn refresh_task(
        &self,
        env: &str,
        key: &ConnectionKey,
        mode: RefreshMode,
        started: DateTime<Utc>,
    ) -> Result<(ConnectionRow, Credentials)> {
        let name = format!(
            "refresh:{env}:{}:{}",
            key.provider_config_key, key.connection_id
        );
        let flight = self.flight(&name);
        let _local = flight.lock().await;
        // Another task can have refreshed while this task waited.
        let row = self.connection_row(env, key).await?;
        let creds = self.open_credentials(&row)?;
        if self.refresh_done(&row, &creds, mode, started)? {
            return Ok((row, creds));
        }
        let gate = policy::refresh_gate(
            mode == RefreshMode::Force,
            row.refresh_exhausted,
            row.last_refresh_failure.map(crate::clock::to_ms),
            self.now_ms(),
            policy::REFRESH_COOLDOWN_MS,
        );
        match gate {
            Gate::Proceed => {}
            Gate::Cooldown => return Err(Error::RefreshCooldown),
            Gate::Exhausted => return Err(Error::RefreshExhausted),
        }
        let ttl =
            u64::try_from(self.inner.config.refresh_lease_ttl.as_millis()).unwrap_or(u64::MAX);
        if !self
            .inner
            .store
            .try_lease(&name, &self.inner.owner, self.now(), ttl)
            .await?
        {
            return self
                .wait_for_other(env, key, mode, started, &row.credentials)
                .await;
        }
        let result = self.refresh_locked(env, key, mode, started).await;
        if let Err(e) = self
            .inner
            .store
            .release_lease(&name, &self.inner.owner)
            .await
        {
            tracing::warn!(code = e.code(), "the refresh lease was not given back");
        }
        result
    }

    /// Waits while another process refreshes. Returns its result.
    async fn wait_for_other(
        &self,
        env: &str,
        key: &ConnectionKey,
        mode: RefreshMode,
        started: DateTime<Utc>,
        from: &str,
    ) -> Result<(ConnectionRow, Credentials)> {
        let deadline = tokio::time::Instant::now() + self.inner.config.refresh_lease_wait;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(LEASE_POLL).await;
            let row = self.connection_row(env, key).await?;
            let creds = self.open_credentials(&row)?;
            if row.credentials != from || self.refresh_done(&row, &creds, mode, started)? {
                return Ok((row, creds));
            }
        }
        Err(Error::RefreshCooldown)
    }

    /// Returns the time limit of a token call: 3/4 of the lease life.
    fn token_call_limit(&self) -> Duration {
        let ttl = self.inner.config.refresh_lease_ttl;
        ttl.saturating_sub(ttl / 4)
    }

    async fn current(
        &self,
        env: &str,
        key: &ConnectionKey,
    ) -> Result<(ConnectionRow, Credentials)> {
        let row = self.connection_row(env, key).await?;
        let creds = self.open_credentials(&row)?;
        Ok((row, creds))
    }

    async fn refresh_locked(
        &self,
        env: &str,
        key: &ConnectionKey,
        mode: RefreshMode,
        started: DateTime<Utc>,
    ) -> Result<(ConnectionRow, Credentials)> {
        // Read again: another process can have refreshed before the lease.
        let (row, creds) = self.current(env, key).await?;
        if self.refresh_done(&row, &creds, mode, started)? {
            return Ok((row, creds));
        }
        let provider = self.provider_of(&row)?;
        let reply = tokio::time::timeout(
            self.token_call_limit(),
            self.new_credentials(&row, &provider, &creds),
        )
        .await
        .unwrap_or_else(|_| {
            Err(Error::Provider {
                code: "refresh_failed",
                status: None,
                message: "the token call took too long".to_owned(),
            })
        });
        match reply {
            Ok(fresh) => {
                let now = self.now();
                let sealed = self.inner.keys.seal_json(
                    &fresh,
                    &Self::connection_aad(env, &key.provider_config_key, &key.connection_id),
                )?;
                let expires_at = fresh.expires_at();
                let stored = self
                    .write_refresh(env, key, &row.credentials, |r| {
                        r.credentials.clone_from(&sealed);
                        r.credentials_expires_at = expires_at;
                        // Calls that started before this time share this result.
                        r.last_refresh_success = Some(now);
                        r.last_refresh_failure = None;
                        r.refresh_attempts = 0;
                        r.refresh_exhausted = false;
                        r.updated_at = now;
                    })
                    .await?;
                match stored {
                    Some(stored) => Ok((stored, fresh)),
                    None => self.current(env, key).await,
                }
            }
            Err(e) => {
                let now = self.now();
                let today = self.now_ms() / policy::MS_PER_DAY;
                let stored = self
                    .write_refresh(env, key, &row.credentials, |r| {
                        let last_day = r
                            .last_refresh_failure
                            .map(|t| crate::clock::to_ms(t) / policy::MS_PER_DAY);
                        let (attempts, exhausted) = policy::record_failure(
                            r.refresh_attempts,
                            last_day,
                            today,
                            policy::MAX_FAILED_REFRESH_DAYS,
                        );
                        r.refresh_attempts = attempts;
                        r.refresh_exhausted = exhausted;
                        r.last_refresh_failure = Some(now);
                    })
                    .await;
                let stored = match stored {
                    Ok(Some(stored)) => stored,
                    // New credentials are stored. This is not their failure.
                    Ok(None) => return self.current(env, key).await,
                    Err(w) => {
                        tracing::warn!(code = w.code(), "a refresh failure was not stored");
                        return Err(e);
                    }
                };
                self.spawn_webhook(
                    env,
                    stored.webhook_url_override.clone(),
                    WebhookEvent::Auth(AuthEvent {
                        connection_id: stored.connection_id.clone(),
                        provider_config_key: stored.provider_config_key.clone(),
                        provider: stored.provider.clone(),
                        auth_mode: creds.auth_mode().as_str().to_owned(),
                        environment: env.to_owned(),
                        operation: AuthOperation::Refresh,
                        success: false,
                        error: Some(ErrorPayload {
                            kind: e.code().to_owned(),
                            description: e.to_string(),
                        }),
                        tags: stored.tags.clone(),
                        end_user: stored.end_user,
                    }),
                );
                Err(e)
            }
        }
    }

    /// Writes the result of a refresh with compare and set. Returns `None`
    /// when the stored credentials are not `from`: a reconnect or an import
    /// wrote newer credentials. A store error gives a wait and a new try.
    async fn write_refresh(
        &self,
        env: &str,
        key: &ConnectionKey,
        from: &str,
        change: impl Fn(&mut ConnectionRow) + Sync,
    ) -> Result<Option<ConnectionRow>> {
        let mut wait = WRITE_BACKOFF;
        for attempt in 1..=WRITE_TRIES {
            match self.try_write_refresh(env, key, from, &change).await {
                Ok(Write::Stored(row)) => return Ok(Some(*row)),
                Ok(Write::Superseded) => return Ok(None),
                Ok(Write::Lost) => {}
                Err(e @ Error::UnknownConnection(_)) => return Err(e),
                Err(e) if attempt == WRITE_TRIES => return Err(e),
                Err(e) => {
                    tracing::warn!(code = e.code(), "a refresh write failed; it tries again");
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(WRITE_BACKOFF_MAX);
                }
            }
        }
        Err(Error::Store(
            "the connection changed too often during a refresh write".to_owned(),
        ))
    }

    async fn try_write_refresh(
        &self,
        env: &str,
        key: &ConnectionKey,
        from: &str,
        change: &(impl Fn(&mut ConnectionRow) + Sync),
    ) -> Result<Write> {
        let mut row = self.connection_row(env, key).await?;
        if row.credentials != from {
            return Ok(Write::Superseded);
        }
        change(&mut row);
        let stored = self.inner.store.update_connection(&row).await?;
        Ok(stored.map_or(Write::Lost, |r| Write::Stored(Box::new(r))))
    }

    /// Asks the provider for new credentials.
    async fn new_credentials(
        &self,
        row: &ConnectionRow,
        provider: &Provider,
        creds: &Credentials,
    ) -> Result<Credentials> {
        let cfg = as_map(&row.connection_config);
        match creds {
            Credentials::OAuth2(old) => {
                let refresh_token = old
                    .refresh_token
                    .as_ref()
                    .ok_or_else(|| invalid("the connection has no refresh token"))?;
                let integration = self
                    .integration_row(&row.environment, &row.provider_config_key)
                    .await?;
                let vars = json!({ "connectionConfig": cfg });
                let url = provider
                    .refresh_url
                    .as_deref()
                    .or_else(|| provider.token_url_for(&AuthMode::OAuth2))
                    .ok_or_else(|| {
                        Error::Internal(format!("provider {} has no token URL", provider.name))
                    })?;
                let url = crate::template::render_url(url, &vars)?;
                crate::proxy::check_upstream(&url, None, false, &self.inner.config.proxy)?;
                let render = |m: &std::collections::BTreeMap<String, String>| -> Result<std::collections::BTreeMap<String, String>> {
                    m.iter().map(|(k, v)| Ok((k.clone(), crate::template::render(v, &vars)?))).collect()
                };
                let params = crate::oauth2::refresh_params(
                    refresh_token.expose(),
                    &render(&provider.refresh_params)?,
                    &render(&provider.token_params)?,
                );
                let client_id = integration.client_id.clone().unwrap_or_default();
                let client_secret = self.client_secret(&integration)?.unwrap_or_default();
                let request = crate::oauth2::token_request(
                    provider,
                    &url,
                    &client_id,
                    client_secret.expose(),
                    params,
                );
                let reply = self.token_call(request, provider, "refresh_failed").await?;
                Ok(Credentials::OAuth2(OAuth2Credentials {
                    access_token: reply.access_token,
                    // A provider that does not rotate the refresh token gives none.
                    refresh_token: reply.refresh_token.or_else(|| old.refresh_token.clone()),
                    expires_at: reply.expires_at,
                    raw: reply.raw,
                }))
            }
            Credentials::OAuth2ClientCredentials(old) => {
                let integration = self
                    .integration_row(&row.environment, &row.provider_config_key)
                    .await?;
                let fresh = self
                    .client_credentials_token(
                        provider,
                        // The same scopes as the first token request.
                        &Self::requested_scopes(&integration, provider),
                        &cfg,
                        &old.client_id,
                        &old.client_secret,
                        "refresh_failed",
                    )
                    .await?;
                Ok(Credentials::OAuth2ClientCredentials(fresh))
            }
            _ => Err(invalid("these credentials do not refresh")),
        }
    }

    /// Deletes a connection. Sends a `deletion` webhook.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`].
    pub async fn delete_connection(&self, env: &str, key: &ConnectionKey) -> Result<()> {
        let row = self
            .inner
            .store
            .delete_connection(env, &key.provider_config_key, &key.connection_id)
            .await?
            .ok_or_else(|| Error::UnknownConnection(key.connection_id.clone()))?;
        let auth_mode = self.provider_of(&row).map_or_else(
            |_| "unknown".to_owned(),
            |p| p.auth_mode.as_str().to_owned(),
        );
        self.spawn_webhook(
            env,
            row.webhook_url_override.clone(),
            WebhookEvent::Auth(AuthEvent {
                connection_id: row.connection_id,
                provider_config_key: row.provider_config_key,
                provider: row.provider,
                auth_mode,
                environment: env.to_owned(),
                operation: AuthOperation::Deletion,
                success: true,
                error: None,
                tags: row.tags,
                end_user: row.end_user,
            }),
        );
        Ok(())
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
        let now = self.now();
        let stored = self
            .update_row(env, key, |r| {
                let mut next = if merge {
                    as_map(&r.metadata)
                } else {
                    Map::new()
                };
                next.extend(metadata.clone());
                r.metadata = Value::Object(next);
                r.updated_at = now;
                Ok(())
            })
            .await?;
        Ok(as_map(&stored.metadata))
    }

    /// Changes the end user, tags or webhook URL of a connection.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`], [`Error::Invalid`] for a webhook URL that
    /// is not HTTPS.
    pub async fn patch_connection(
        &self,
        env: &str,
        key: &ConnectionKey,
        patch: ConnectionPatch,
    ) -> Result<()> {
        if let Some(Some(url)) = &patch.webhook_url_override {
            self.check_webhook_url(url)?;
        }
        let now = self.now();
        self.update_row(env, key, |r| {
            if let Some(u) = &patch.end_user {
                r.end_user = Some(u.clone());
            }
            if let Some(t) = &patch.tags {
                r.tags.clone_from(t);
            }
            if let Some(w) = &patch.webhook_url_override {
                r.webhook_url_override.clone_from(w);
            }
            r.updated_at = now;
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// Refreshes the tokens that expire before `now + within`. A token in
    /// the first half of its life since the last refresh waits.
    ///
    /// The sweep reads pages in `(expiry, id)` order. It goes on from the
    /// row where the last sweep stopped, so rows that it skips or that fail
    /// do not block later rows. `limit` counts the refresh tries.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn refresh_due(&self, within: Duration, limit: usize) -> Result<RefreshReport> {
        let mut report = RefreshReport::default();
        if limit == 0 {
            return Ok(report);
        }
        let before =
            self.now() + chrono::Duration::from_std(within).unwrap_or(chrono::Duration::days(1));
        let page = limit.min(SWEEP_PAGE);
        let start = *self.sweep_after();
        let mut after = start;
        // At the end of the rows, go on from the first row, up to `start`.
        let mut wrapped = start.is_none();
        let mut tries = 0;
        loop {
            let rows = self
                .inner
                .store
                .connections_expiring_before(before, after, page)
                .await?;
            if rows.is_empty() {
                if wrapped {
                    break;
                }
                wrapped = true;
                after = None;
                continue;
            }
            for row in rows {
                let Some(at) = row.credentials_expires_at.map(|t| (t, row.id)) else {
                    continue;
                };
                if wrapped && start.is_some_and(|s| at >= s) {
                    *self.sweep_after() = None;
                    return Ok(report);
                }
                after = Some(at);
                if self.sweep_one(&row, before, &mut report).await {
                    tries += 1;
                    if tries >= limit {
                        *self.sweep_after() = after;
                        return Ok(report);
                    }
                }
            }
        }
        *self.sweep_after() = None;
        Ok(report)
    }

    fn sweep_after(&self) -> std::sync::MutexGuard<'_, Option<(DateTime<Utc>, Uuid)>> {
        self.inner
            .sweep_after
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Refreshes one row of the sweep. Returns `true` when it tried a refresh.
    async fn sweep_one(
        &self,
        row: &ConnectionRow,
        before: DateTime<Utc>,
        report: &mut RefreshReport,
    ) -> bool {
        let mode = RefreshMode::Before(before);
        let due = self
            .open_credentials(row)
            .is_ok_and(|c| self.needs_refresh(row, &c, mode).unwrap_or(false));
        if !due {
            report.skipped += 1;
            return false;
        }
        match self.refresh(&row.environment, &row.key(), mode).await {
            Ok(_) => {
                report.refreshed += 1;
                true
            }
            Err(Error::RefreshCooldown | Error::RefreshExhausted) => {
                report.skipped += 1;
                false
            }
            Err(e) => {
                tracing::warn!(connection = %row.connection_id, code = e.code(), "token refresh failed");
                report.failed += 1;
                true
            }
        }
    }
}
