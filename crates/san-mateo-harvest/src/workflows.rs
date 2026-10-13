//! Harvest workflows for San Mateo.
//!
//! A harvest schedule is unique for each workflow name. Thus a schedule for
//! each connection is not possible. [`san_mateo_sync_loop`] is an entity
//! workflow for one (connection, sync). It runs the sync, waits with a
//! durable timer, and loops. It calls `continue_as_new` from time to time, so
//! its history stays small. [`san_mateo_refresh_sweep`] runs one refresh
//! sweep. Give it one harvest schedule.
//!
//! The workflow bodies use no wall clock and no network. All I/O is in the
//! activities.

use std::time::Duration;

use autumn_harvest::context::WorkflowContext;
use autumn_harvest::failure::WorkflowFailure;
use autumn_harvest::prelude::workflow;
use san_mateo::ConnectionKey;
use serde::{Deserialize, Serialize};

use crate::activities::{RefreshDueInput, RefreshDueOutput};

/// The shortest time between two sync runs. A shorter frequency becomes this
/// value. The San Mateo scheduler uses the same minimum.
pub const MIN_FREQUENCY_SECS: u64 = 30;

/// The default number of sync runs before `continue_as_new`.
pub const DEFAULT_ITERATIONS_BEFORE_CONTINUE: u32 = 100;

/// The prefix of the workflow id of a sync loop.
pub const SYNC_WORKFLOW_ID_PREFIX: &str = "san-mateo-sync:";

/// The input of [`san_mateo_sync_loop`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncLoopInput {
    /// The environment id.
    pub environment: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
    /// The sync name.
    pub sync_name: String,
    /// The time between two runs, in seconds.
    pub frequency_secs: u64,
    /// The number of runs before `continue_as_new`. Default:
    /// [`DEFAULT_ITERATIONS_BEFORE_CONTINUE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations_before_continue: Option<u32>,
    /// The key of the harvest concurrency limit: one for each (environment,
    /// connection, sync). [`SyncLoopInput::new`] sets it.
    #[serde(default)]
    pub sync_key: String,
    /// Successful runs in earlier executions of this loop.
    #[serde(default)]
    pub total_runs: u64,
    /// Failed runs in earlier executions of this loop.
    #[serde(default)]
    pub total_failures: u64,
}

impl SyncLoopInput {
    /// Makes an input. It sets [`Self::sync_key`].
    #[must_use]
    pub fn new(
        environment: impl Into<String>,
        key: &ConnectionKey,
        sync_name: impl Into<String>,
        frequency: Duration,
    ) -> Self {
        let _ = (environment, key, sync_name, frequency);
        todo!()
    }

    /// Sets [`Self::iterations_before_continue`].
    #[must_use]
    pub fn iterations_before_continue(self, iterations: u32) -> Self {
        let _ = (self, iterations);
        todo!()
    }

    /// Returns the connection key.
    #[must_use]
    pub fn key(&self) -> ConnectionKey {
        todo!()
    }

    /// Returns the sync key of this (environment, connection, sync).
    ///
    /// The key is a JSON array of the four ids, so two different syncs never
    /// have the same key.
    #[must_use]
    pub fn sync_key_of(&self) -> String {
        todo!()
    }

    /// Returns a stable workflow id for this loop.
    ///
    /// Start the loop with this id and a harvest id conflict policy that
    /// rejects a second live run. Then each (connection, sync) has one loop.
    #[must_use]
    pub fn workflow_id(&self) -> String {
        todo!()
    }
}

/// The output of [`san_mateo_sync_loop`] when the loop stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncLoopOutput {
    /// Successful runs in all executions of this loop.
    pub total_runs: u64,
    /// Failed runs in all executions of this loop.
    pub total_failures: u64,
    /// The error type that stopped the loop, if any.
    pub stopped_by: Option<String>,
}

/// Returns `true` when a run failure with this error type stops the sync
/// loop. These failures stay the same on each later run: the connection, the
/// integration, the environment or the sync is gone.
#[must_use]
pub fn stops_sync_loop(error_type: &str) -> bool {
    let _ = error_type;
    todo!()
}

/// Runs one sync for one connection in a durable loop.
///
/// Each iteration runs `san_mateo_run_sync`, then waits for the frequency
/// with a durable timer. A failed run does not stop the loop, except for the
/// failures of [`stops_sync_loop`]. After
/// [`SyncLoopInput::iterations_before_continue`] runs, or when harvest asks
/// for it, the loop calls `continue_as_new` with the totals. The harvest
/// concurrency limit on `sync_key` lets one task run at a time for each
/// (connection, sync).
///
/// # Errors
///
/// A non-retryable `san_mateo.invalid_input` failure for an input with an
/// empty id. A `san_mateo.workflow` failure for a harvest error that is not
/// an activity failure, for example a cancel.
#[workflow(concurrency(key = "input.sync_key", limit = 1))]
pub async fn san_mateo_sync_loop(
    ctx: &WorkflowContext,
    input: SyncLoopInput,
) -> Result<SyncLoopOutput, WorkflowFailure> {
    let _ = (ctx, input);
    todo!()
}

/// Runs one refresh sweep: `san_mateo_refresh_due`.
///
/// Give this workflow one harvest schedule, for example each 5 minutes. A
/// `null` input uses the defaults of [`RefreshDueInput`].
///
/// # Errors
///
/// A failure with the error type of the activity when the sweep fails.
#[workflow]
pub async fn san_mateo_refresh_sweep(
    ctx: &WorkflowContext,
    input: Option<RefreshDueInput>,
) -> Result<RefreshDueOutput, WorkflowFailure> {
    let _ = (ctx, input);
    todo!()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    fn input() -> SyncLoopInput {
        SyncLoopInput::new(
            "dev",
            &ConnectionKey::new("crm", "c-1"),
            "contacts",
            Duration::from_secs(300),
        )
    }

    #[test]
    fn new_sets_the_sync_key() {
        let i = input();
        assert_eq!(i.frequency_secs, 300);
        assert_eq!(i.iterations_before_continue, None);
        assert_eq!(i.sync_key, i.sync_key_of());
        assert_eq!(i.sync_key, r#"["dev","crm","c-1","contacts"]"#);
        assert_eq!(i.key(), ConnectionKey::new("crm", "c-1"));
        assert_eq!(i.total_runs, 0);
        let i = i.iterations_before_continue(7);
        assert_eq!(i.iterations_before_continue, Some(7));
    }

    #[test]
    fn workflow_ids_are_stable_and_distinct() {
        let a = input();
        assert_eq!(a.workflow_id(), input().workflow_id());
        assert!(a.workflow_id().starts_with(SYNC_WORKFLOW_ID_PREFIX));
        let b = SyncLoopInput::new(
            "dev",
            &ConnectionKey::new("crm", "c-1"),
            "deals",
            Duration::from_secs(300),
        );
        assert_ne!(a.workflow_id(), b.workflow_id());
        // Separators in ids do not make two keys the same.
        let c = SyncLoopInput::new(
            "dev",
            &ConnectionKey::new("crm/c", "1"),
            "contacts",
            Duration::from_secs(1),
        );
        let d = SyncLoopInput::new(
            "dev",
            &ConnectionKey::new("crm", "c/1"),
            "contacts",
            Duration::from_secs(1),
        );
        assert_ne!(c.sync_key, d.sync_key);
    }

    #[test]
    fn input_json_defaults() {
        let i: SyncLoopInput = serde_json::from_value(json!({
            "environment": "dev",
            "provider_config_key": "crm",
            "connection_id": "c-1",
            "sync_name": "contacts",
            "frequency_secs": 60
        }))
        .unwrap();
        assert_eq!(i.sync_key, "");
        assert_eq!(i.total_runs, 0);
        assert_eq!(i.total_failures, 0);
        assert_eq!(i.iterations_before_continue, None);
    }

    #[test]
    fn stopping_failures() {
        for t in [
            "san_mateo.unknown_connection",
            "san_mateo.unknown_function",
            "san_mateo.unknown_provider_config",
            "san_mateo.invalid_body",
            "san_mateo.invalid_input",
        ] {
            assert!(stops_sync_loop(t), "{t}");
        }
        for t in [
            "san_mateo.sync_failed",
            "san_mateo.conflict",
            "san_mateo.config",
            "san_mateo.upstream_error",
            "san_mateo.http_503",
            "Error",
            "CircuitOpen",
        ] {
            assert!(!stops_sync_loop(t), "{t}");
        }
    }

    #[test]
    fn workflow_infos() {
        let info = san_mateo_sync_loop_info();
        assert_eq!(info.name, "san_mateo_sync_loop");
        let c = info.concurrency.unwrap();
        assert_eq!(c.key_expr, "input.sync_key");
        assert_eq!(c.limit, 1);
        assert_eq!(san_mateo_refresh_sweep_info().name, "san_mateo_refresh_sweep");
    }
}
