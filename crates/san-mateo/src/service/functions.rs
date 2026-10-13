//! Sync and action runs, the in-process scheduler and the background loops.
//!
//! In-process runs are not durable. Use `san-mateo-harvest` for durable syncs.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt as _;
use serde_json::Value;
use uuid::Uuid;

use super::{Background, BackgroundSettings, SanMateo, SyncRunReport, SyncStatusEntry};
use crate::error::{Error, Result};
use crate::functions::{ActionContext, SyncContext, SyncFunction};
use crate::model::ConnectionKey;
use crate::policy;
use crate::records::BatchOutcome;
use crate::store::{ConnectionFilter, ConnectionRow, SyncCounts, SyncStateRow, SyncStatus};
use crate::webhook::{ErrorPayload, SyncEvent, WebhookEvent};

/// Removes a running sync from the set when the run ends, also on a panic.
struct Running<'a> {
    set: &'a Mutex<std::collections::HashSet<(Uuid, String)>>,
    key: (Uuid, String),
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

fn counts(outcome: &BatchOutcome) -> SyncCounts {
    let n = |v: &Vec<String>| u64::try_from(v.len()).unwrap_or(u64::MAX);
    SyncCounts {
        added: n(&outcome.added),
        updated: n(&outcome.updated),
        deleted: n(&outcome.deleted),
    }
}

fn frequency_ms(sync: &dyn SyncFunction) -> u64 {
    policy::sync_frequency_ms(u64::try_from(sync.spec().frequency.as_millis()).unwrap_or(u64::MAX))
}

fn panic_text() -> Error {
    Error::Function("the function panicked".to_owned())
}

impl SanMateo {
    fn sync_fn(&self, integration: &str, name: &str) -> Result<Arc<dyn SyncFunction>> {
        self.registry()
            .get_sync(integration, name)
            .ok_or_else(|| Error::UnknownFunction(format!("{integration}/{name}")))
    }

    fn default_state(connection: Uuid, sync: &dyn SyncFunction) -> SyncStateRow {
        SyncStateRow {
            connection,
            sync_name: sync.spec().name,
            status: SyncStatus::Stopped,
            frequency_ms: frequency_ms(sync),
            paused: false,
            last_started_at: None,
            last_finished_at: None,
            last_success_at: None,
            last_error: None,
            last_counts: SyncCounts::default(),
        }
    }

    async fn state_of(&self, conn: Uuid, sync: &dyn SyncFunction) -> Result<SyncStateRow> {
        Ok(self
            .inner
            .store
            .get_sync_state(conn, &sync.spec().name)
            .await?
            .unwrap_or_else(|| Self::default_state(conn, sync)))
    }

    /// Runs one sync for one connection now, and waits for the end.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownFunction`], [`Error::UnknownConnection`],
    /// [`Error::Conflict`] when the sync runs already.
    pub async fn run_sync(&self, env: &str, key: &ConnectionKey, sync_name: &str) -> Result<SyncRunReport> {
        let sync = self.sync_fn(&key.provider_config_key, sync_name)?;
        let conn = self.connection_row(env, key).await?;
        self.run_sync_row(env, &conn, sync).await
    }

    async fn run_sync_row(&self, env: &str, conn: &ConnectionRow, sync: Arc<dyn SyncFunction>) -> Result<SyncRunReport> {
        let spec = sync.spec();
        let run_key = (conn.id, spec.name.clone());
        {
            let mut running = self.inner.running_syncs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !running.insert(run_key.clone()) {
                return Err(Error::Conflict(format!("sync {} runs already", spec.name)));
            }
        }
        let _guard = Running { set: &self.inner.running_syncs, key: run_key };
        let mut state = self.state_of(conn.id, sync.as_ref()).await?;
        let started = self.now();
        state.status = SyncStatus::Running;
        state.frequency_ms = frequency_ms(sync.as_ref());
        state.last_started_at = Some(started);
        self.inner.store.put_sync_state(state.clone()).await?;

        let ctx = SyncContext {
            san: self.clone(),
            environment: env.to_owned(),
            key: conn.key(),
            connection: conn.id,
            sync_name: spec.name.clone(),
            last_success_at: state.last_success_at,
            outcome: Mutex::new(BatchOutcome::default()),
        };
        let result = AssertUnwindSafe(sync.run(&ctx)).catch_unwind().await.unwrap_or_else(|_| Err(panic_text()));
        let run_counts = counts(&ctx.outcome.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let finished = self.now();
        state.status = if result.is_ok() { SyncStatus::Success } else { SyncStatus::Error };
        state.last_finished_at = Some(finished);
        if result.is_ok() {
            state.last_success_at = Some(finished);
        }
        state.last_error = result.as_ref().err().map(ToString::to_string);
        state.last_counts = run_counts;
        // Keep a pause that a caller set during the run.
        if let Some(current) = self.inner.store.get_sync_state(conn.id, &spec.name).await? {
            state.paused = current.paused;
        }
        self.inner.store.put_sync_state(state).await?;
        self.spawn_webhook(
            env,
            conn.webhook_url_override.clone(),
            WebhookEvent::Sync(SyncEvent {
                connection_id: conn.connection_id.clone(),
                provider_config_key: conn.provider_config_key.clone(),
                sync_name: spec.name.clone(),
                models: spec.models.clone(),
                success: result.is_ok(),
                modified_after: started,
                counts: run_counts,
                error: result.as_ref().err().map(|e| ErrorPayload { kind: e.code().to_owned(), description: e.to_string() }),
            }),
        );
        Ok(SyncRunReport {
            sync_name: spec.name,
            connection: conn.key(),
            success: result.is_ok(),
            counts: run_counts,
            error: result.err().map(|e| e.to_string()),
        })
    }

    /// Returns the syncs by name, or all syncs of the integration.
    fn pick_syncs(&self, integration: &str, names: &[String]) -> Result<Vec<Arc<dyn SyncFunction>>> {
        if names.is_empty() || names.iter().any(|n| n == "*") {
            return Ok(self.registry().syncs_for(integration));
        }
        names.iter().map(|n| self.sync_fn(integration, n)).collect()
    }

    async fn pick_connections(&self, env: &str, integration: &str, connection_id: Option<&str>) -> Result<Vec<ConnectionRow>> {
        self.integration_row(env, integration).await?;
        match connection_id {
            Some(id) => Ok(vec![self.connection_row(env, &ConnectionKey::new(integration, id)).await?]),
            None => {
                let filter = ConnectionFilter {
                    provider_config_key: Some(integration.to_owned()),
                    ..ConnectionFilter::default()
                };
                self.inner.store.list_connections(env, &filter).await
            }
        }
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
        let conns = self.pick_connections(env, integration, connection_id).await?;
        let picked = self.pick_syncs(integration, syncs)?;
        let mut started = 0;
        for conn in conns {
            for sync in &picked {
                let san = self.clone();
                let env = env.to_owned();
                let conn = conn.clone();
                let sync = sync.clone();
                self.inner.tasks.spawn(async move {
                    if let Err(e) = san.run_sync_row(&env, &conn, sync).await {
                        tracing::warn!(code = e.code(), "a triggered sync did not run");
                    }
                });
                started += 1;
            }
        }
        Ok(started)
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
        let picked = self.pick_syncs(integration, syncs)?;
        for conn in self.pick_connections(env, integration, connection_id).await? {
            for sync in &picked {
                let mut state = self.state_of(conn.id, sync.as_ref()).await?;
                state.paused = paused;
                self.inner.store.put_sync_state(state).await?;
            }
        }
        Ok(())
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
        let picked = self.pick_syncs(integration, syncs)?;
        let mut out = Vec::new();
        for conn in self.pick_connections(env, integration, connection_id).await? {
            for sync in &picked {
                let state = self.state_of(conn.id, sync.as_ref()).await?;
                let frequency = Duration::from_millis(state.frequency_ms);
                let next = (sync.spec().auto_start && !state.paused).then(|| {
                    state.last_started_at.map_or_else(
                        || self.now(),
                        |t| t + chrono::Duration::from_std(frequency).unwrap_or(chrono::Duration::zero()),
                    )
                });
                out.push(SyncStatusEntry {
                    name: state.sync_name.clone(),
                    connection_id: conn.connection_id.clone(),
                    status: if state.paused { SyncStatus::Paused } else { state.status },
                    frequency,
                    finished_at: state.last_finished_at,
                    next_scheduled_at: next,
                    latest_result: state.last_counts,
                });
            }
        }
        Ok(out)
    }

    /// Runs the auto-start syncs that are due, in all environments.
    ///
    /// # Errors
    ///
    /// Store errors.
    pub async fn run_due_syncs(&self) -> Result<Vec<SyncRunReport>> {
        let now_ms = self.now_ms();
        let mut due = Vec::new();
        for env in &self.inner.config.environments {
            for sync in self.registry().syncs() {
                let spec = sync.spec();
                if !spec.auto_start || self.inner.store.get_integration(&env.id, &spec.integration).await?.is_none() {
                    continue;
                }
                for conn in self.pick_connections(&env.id, &spec.integration, None).await? {
                    let state = self.state_of(conn.id, sync.as_ref()).await?;
                    let next = state
                        .last_started_at
                        .map_or(0, |t| crate::clock::to_ms(t).saturating_add(frequency_ms(sync.as_ref())));
                    if !state.paused && now_ms >= next {
                        due.push((env.id.clone(), conn, sync.clone()));
                    }
                }
            }
        }
        let runs = due.into_iter().map(|(env, conn, sync)| async move { self.run_sync_row(&env, &conn, sync).await });
        Ok(futures::future::join_all(runs).await.into_iter().filter_map(Result::ok).collect())
    }

    /// Runs an action and returns its output.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownFunction`], [`Error::UnknownConnection`], and the
    /// errors of the action.
    pub async fn run_action(&self, env: &str, key: &ConnectionKey, action: &str, input: Value) -> Result<Value> {
        let f = self
            .registry()
            .get_action(&key.provider_config_key, action)
            .ok_or_else(|| Error::UnknownFunction(format!("{}/{action}", key.provider_config_key)))?;
        self.connection_row(env, key).await?;
        let ctx = ActionContext {
            san: self.clone(),
            environment: env.to_owned(),
            key: key.clone(),
        };
        AssertUnwindSafe(f.run(&ctx, input)).catch_unwind().await.unwrap_or_else(|_| Err(panic_text()))
    }

    /// Starts the background loops: refresh sweep, sync scheduler and session
    /// purge. Stop them with [`Background::stop`].
    #[must_use]
    pub fn start_background(&self, settings: BackgroundSettings) -> Background {
        let stop = tokio_util::sync::CancellationToken::new();
        let token = stop.clone();
        let san = self.clone();
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(settings.tick);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = token.cancelled() => break,
                    _ = ticker.tick() => san.tick(settings).await,
                }
            }
        });
        Background { stop, task: Some(task) }
    }

    /// One tick of the background loops. Errors go to the log.
    async fn tick(&self, settings: BackgroundSettings) {
        if let Err(e) = self.refresh_due(settings.refresh_within, settings.refresh_batch).await {
            tracing::warn!(code = e.code(), "the refresh sweep failed");
        }
        if settings.run_syncs {
            if let Err(e) = self.run_due_syncs().await {
                tracing::warn!(code = e.code(), "the sync scheduler failed");
            }
        }
        if let Err(e) = self.inner.store.purge_expired(self.now()).await {
            tracing::warn!(code = e.code(), "the session purge failed");
        }
    }
}
