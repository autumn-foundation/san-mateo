//! autumn-harvest activities and workflows for San Mateo: proxy calls, token
//! refresh and durable syncs.
//!
//! The activities read [`SanMateo`] from the harvest state. Register the
//! workflows, the activities and the service in one call:
//!
//! ```no_run
//! use autumn_harvest::HarvestBuilder;
//! use san_mateo::{EnvironmentConfig, KeyRing, SanMateo};
//!
//! # fn demo() -> san_mateo::Result<()> {
//! let san = SanMateo::builder()
//!     .server_url("https://sm.example.com")
//!     .environment(EnvironmentConfig::new("prod", "sk_live_..."))
//!     .key_ring(KeyRing::from_base64("k1", "<32 random bytes, base64>")?)
//!     .build()?;
//! let harvest = HarvestBuilder::new()
//!     .workflows(san_mateo_harvest::workflows())
//!     .activities(san_mateo_harvest::activities())
//!     .state(san)
//!     .build();
//! # let _ = harvest;
//! # Ok(())
//! # }
//! ```
//!
//! With `autumn-harvest-plugin`, the plugin has the same three calls:
//!
//! ```ignore
//! let plugin = HarvestPlugin::new()
//!     .workflows(san_mateo_harvest::workflows())
//!     .activities(san_mateo_harvest::activities())
//!     .state(san);
//! ```
//!
//! # Activities
//!
//! | Activity | Input | Output |
//! |---|---|---|
//! | `san_mateo_proxy` | [`ProxyInput`] | [`ProxyOutput`] |
//! | `san_mateo_refresh` | [`ConnectionInput`] | [`RefreshOutput`] |
//! | `san_mateo_run_sync` | [`RunSyncInput`] | [`SyncRunOutput`] |
//! | `san_mateo_run_action` | [`RunActionInput`] | the action output |
//! | `san_mateo_refresh_due` | [`RefreshDueInput`] | [`RefreshDueOutput`] |
//!
//! Inputs hold ids and options only. San Mateo adds the credentials, so no
//! secret goes into the harvest history. No output holds a token.
//!
//! # Failures
//!
//! An activity returns a typed harvest failure. HTTP 5xx, 408, 425 and 429
//! replies are retryable. Other 4xx replies are not. A San Mateo error is
//! retryable when [`san_mateo::Error::is_retryable`] is `true`. See
//! [`failure_for_status`] and [`failure_for_error`]. In your own activities,
//! use [`proxy_from_activity`] to get the same mapping.
//!
//! # Workflows
//!
//! A harvest schedule is unique for each workflow name. Thus a sync for each
//! connection is an entity workflow: [`san_mateo_sync_loop`]. Start one loop
//! for each (connection, sync) with the id from
//! [`SyncLoopInput::workflow_id`]. [`san_mateo_refresh_sweep`] runs one
//! refresh sweep. Give it one harvest schedule.

pub mod activities;
pub mod failure;
pub mod proxy;
pub mod workflows;

use autumn_harvest::{ActivityInfo, WorkflowInfo};
pub use san_mateo::SanMateo;

pub use activities::{
    ConnectionInput, ProxyInput, ProxyOutput, RefreshDueInput, RefreshDueOutput, RefreshOutput,
    RunActionInput, RunSyncInput, SyncRunOutput, san_mateo_proxy, san_mateo_proxy_info,
    san_mateo_refresh, san_mateo_refresh_due, san_mateo_refresh_due_info, san_mateo_refresh_info,
    san_mateo_run_action, san_mateo_run_action_info, san_mateo_run_sync, san_mateo_run_sync_info,
};
pub use failure::{
    CONFIG_ERROR, INVALID_INPUT_ERROR, SYNC_FAILED_ERROR, failure_for_error, failure_for_status,
    is_retryable_status,
};
pub use proxy::{IDEMPOTENCY_KEY_HEADER, proxy_from_activity, proxy_with, san_mateo_from};
pub use workflows::{
    SyncLoopInput, SyncLoopOutput, san_mateo_refresh_sweep, san_mateo_refresh_sweep_info,
    san_mateo_sync_loop, san_mateo_sync_loop_info, stops_sync_loop,
};

/// Returns all workflows of this crate, for `workflows(..)` on a harvest
/// builder or plugin.
#[must_use]
pub fn workflows() -> Vec<WorkflowInfo> {
    todo!()
}

/// Returns all activities of this crate, for `activities(..)` on a harvest
/// builder or plugin.
#[must_use]
pub fn activities() -> Vec<ActivityInfo> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_lists() {
        let names: Vec<&str> = workflows().iter().map(|w| w.name).collect();
        assert_eq!(names, ["san_mateo_sync_loop", "san_mateo_refresh_sweep"]);
        let names: Vec<&str> = activities().iter().map(|a| a.name).collect();
        assert_eq!(
            names,
            [
                "san_mateo_proxy",
                "san_mateo_refresh",
                "san_mateo_run_sync",
                "san_mateo_run_action",
                "san_mateo_refresh_due",
            ]
        );
    }
}
