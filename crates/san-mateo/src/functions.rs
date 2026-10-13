//! Syncs and actions as Rust functions.
//!
//! A sync reads data from a provider and saves records. An action does one
//! call with JSON input and gives JSON output. Register them in a
//! [`Registry`] on the [`crate::SanMateoBuilder`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::error::Result;
use crate::model::ConnectionKey;
use crate::paginate::PaginationConfig;
use crate::proxy::{ProxyRequest, ProxyResponse};
use crate::records::BatchOutcome;
use crate::service::SanMateo;
use crate::store::SyncCounts;

/// The description of a sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSpec {
    /// The sync name. It is unique for each integration.
    pub name: String,
    /// The integration key that the sync is for.
    pub integration: String,
    /// The models that the sync saves.
    pub models: Vec<String>,
    /// The time between runs. The minimum is 30 seconds.
    pub frequency: Duration,
    /// `true` when the scheduler runs the sync for each connection.
    pub auto_start: bool,
}

/// A sync function.
#[async_trait]
pub trait SyncFunction: Send + Sync + 'static {
    /// Returns the description.
    fn spec(&self) -> SyncSpec;
    /// Runs one time for one connection.
    async fn run(&self, ctx: &SyncContext) -> Result<()>;
}

/// The description of an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSpec {
    /// The action name. It is unique for each integration.
    pub name: String,
    /// The integration key that the action is for.
    pub integration: String,
}

/// An action function.
#[async_trait]
pub trait ActionFunction: Send + Sync + 'static {
    /// Returns the description.
    fn spec(&self) -> ActionSpec;
    /// Runs one time for one connection.
    async fn run(&self, ctx: &ActionContext, input: Value) -> Result<Value>;
}

/// Options for [`SyncContext::paginate`] and [`ActionContext::paginate`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaginateOptions {
    /// A config to use in place of the provider config.
    pub config: Option<PaginationConfig>,
    /// The page size to ask for.
    pub limit: Option<u64>,
    /// The largest number of pages. Default 100.
    pub max_pages: Option<u32>,
}

/// What a sync function can do.
pub struct SyncContext {
    pub(crate) san: SanMateo,
    pub(crate) environment: String,
    pub(crate) key: ConnectionKey,
    pub(crate) connection: Uuid,
    pub(crate) sync_name: String,
    pub(crate) last_success_at: Option<DateTime<Utc>>,
    pub(crate) outcome: Mutex<BatchOutcome>,
}

impl std::fmt::Debug for SyncContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncContext")
            .field("environment", &self.environment)
            .field("key", &self.key)
            .field("sync_name", &self.sync_name)
            .finish_non_exhaustive()
    }
}

impl SyncContext {
    /// Returns the connection key.
    #[must_use]
    pub const fn connection(&self) -> &ConnectionKey {
        &self.key
    }

    /// Returns the environment id.
    #[must_use]
    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// Returns the start of the last successful run, if any. Read changes
    /// since this time: changes during that run come after it.
    #[must_use]
    pub const fn last_sync_date(&self) -> Option<DateTime<Utc>> {
        self.last_success_at
    }

    /// Sends a call through the proxy.
    ///
    /// # Errors
    ///
    /// The errors of [`SanMateo::proxy`].
    pub async fn proxy(&self, request: ProxyRequest) -> Result<ProxyResponse> {
        self.san.proxy(&self.environment, &self.key, request).await
    }

    /// Sends a `GET` through the proxy.
    ///
    /// # Errors
    ///
    /// The errors of [`SanMateo::proxy`].
    pub async fn get(&self, endpoint: &str) -> Result<ProxyResponse> {
        self.proxy(ProxyRequest::get(endpoint)).await
    }

    /// Reads all pages and returns all records.
    ///
    /// # Errors
    ///
    /// The errors of [`SanMateo::paginate`].
    pub async fn paginate(
        &self,
        request: ProxyRequest,
        options: PaginateOptions,
    ) -> Result<Vec<Value>> {
        self.san
            .paginate(&self.environment, &self.key, request, options)
            .await
    }

    /// Saves records of `model`. Each record must have an `id`.
    ///
    /// # Errors
    ///
    /// The errors of [`SanMateo::save_records`].
    pub async fn batch_save(&self, model: &str, records: Vec<Value>) -> Result<BatchOutcome> {
        let out = self
            .san
            .save_records(&self.environment, &self.key, model, records)
            .await?;
        self.lock_outcome().extend(out.clone());
        Ok(out)
    }

    fn lock_outcome(&self) -> std::sync::MutexGuard<'_, BatchOutcome> {
        self.outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Marks records of `model` as deleted. Each record must have an `id`.
    ///
    /// # Errors
    ///
    /// The errors of [`SanMateo::delete_records`].
    pub async fn batch_delete(&self, model: &str, records: Vec<Value>) -> Result<BatchOutcome> {
        let out = self
            .san
            .delete_records(&self.environment, &self.key, model, records)
            .await?;
        self.lock_outcome().extend(out.clone());
        Ok(out)
    }

    /// Returns the connection metadata.
    ///
    /// # Errors
    ///
    /// [`crate::Error::UnknownConnection`] when the connection is gone.
    pub async fn metadata(&self) -> Result<Value> {
        let row = self
            .san
            .connection_row(&self.environment, &self.key)
            .await?;
        Ok(row.metadata)
    }

    /// Returns the counts of this run so far.
    #[must_use]
    pub fn counts(&self) -> SyncCounts {
        let o = self.lock_outcome();
        let n = |v: &Vec<String>| u64::try_from(v.len()).unwrap_or(u64::MAX);
        SyncCounts {
            added: n(&o.added),
            updated: n(&o.updated),
            deleted: n(&o.deleted),
        }
    }

    /// Returns the internal id of the connection.
    #[must_use]
    pub const fn connection_uuid(&self) -> Uuid {
        self.connection
    }
}

/// What an action function can do.
pub struct ActionContext {
    pub(crate) san: SanMateo,
    pub(crate) environment: String,
    pub(crate) key: ConnectionKey,
}

impl std::fmt::Debug for ActionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionContext")
            .field("environment", &self.environment)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl ActionContext {
    /// Returns the connection key.
    #[must_use]
    pub const fn connection(&self) -> &ConnectionKey {
        &self.key
    }

    /// Returns the environment id.
    #[must_use]
    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// Sends a call through the proxy.
    ///
    /// # Errors
    ///
    /// The errors of [`SanMateo::proxy`].
    pub async fn proxy(&self, request: ProxyRequest) -> Result<ProxyResponse> {
        self.san.proxy(&self.environment, &self.key, request).await
    }

    /// Reads all pages and returns all records.
    ///
    /// # Errors
    ///
    /// The errors of [`SanMateo::paginate`].
    pub async fn paginate(
        &self,
        request: ProxyRequest,
        options: PaginateOptions,
    ) -> Result<Vec<Value>> {
        self.san
            .paginate(&self.environment, &self.key, request, options)
            .await
    }

    /// Returns the connection metadata.
    ///
    /// # Errors
    ///
    /// [`crate::Error::UnknownConnection`] when the connection is gone.
    pub async fn metadata(&self) -> Result<Value> {
        let row = self
            .san
            .connection_row(&self.environment, &self.key)
            .await?;
        Ok(row.metadata)
    }
}

/// Registered syncs and actions, by (integration, name).
#[derive(Clone, Default)]
pub struct Registry {
    syncs: BTreeMap<(String, String), Arc<dyn SyncFunction>>,
    actions: BTreeMap<(String, String), Arc<dyn ActionFunction>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("syncs", &self.syncs.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Registry {
    /// Makes an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a sync. A sync with the same integration and name is replaced.
    #[must_use]
    pub fn sync(self, function: impl SyncFunction) -> Self {
        self.sync_arc(Arc::new(function))
    }

    /// Adds a shared sync.
    #[must_use]
    pub fn sync_arc(mut self, function: Arc<dyn SyncFunction>) -> Self {
        let spec = function.spec();
        self.syncs.insert((spec.integration, spec.name), function);
        self
    }

    /// Adds an action. An action with the same integration and name is replaced.
    #[must_use]
    pub fn action(self, function: impl ActionFunction) -> Self {
        self.action_arc(Arc::new(function))
    }

    /// Adds a shared action.
    #[must_use]
    pub fn action_arc(mut self, function: Arc<dyn ActionFunction>) -> Self {
        let spec = function.spec();
        self.actions.insert((spec.integration, spec.name), function);
        self
    }

    /// Returns a sync.
    #[must_use]
    pub fn get_sync(&self, integration: &str, name: &str) -> Option<Arc<dyn SyncFunction>> {
        self.syncs
            .get(&(integration.to_owned(), name.to_owned()))
            .cloned()
    }

    /// Returns an action.
    #[must_use]
    pub fn get_action(&self, integration: &str, name: &str) -> Option<Arc<dyn ActionFunction>> {
        self.actions
            .get(&(integration.to_owned(), name.to_owned()))
            .cloned()
    }

    /// Returns all syncs, sorted by (integration, name).
    #[must_use]
    pub fn syncs(&self) -> Vec<Arc<dyn SyncFunction>> {
        self.syncs.values().cloned().collect()
    }

    /// Returns the syncs of one integration, sorted by name.
    #[must_use]
    pub fn syncs_for(&self, integration: &str) -> Vec<Arc<dyn SyncFunction>> {
        self.syncs
            .iter()
            .filter(|((i, _), _)| i == integration)
            .map(|(_, f)| f.clone())
            .collect()
    }

    /// Returns all actions, sorted by (integration, name).
    #[must_use]
    pub fn actions(&self) -> Vec<Arc<dyn ActionFunction>> {
        self.actions.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct S(&'static str, &'static str);

    #[async_trait]
    impl SyncFunction for S {
        fn spec(&self) -> SyncSpec {
            SyncSpec {
                name: self.1.into(),
                integration: self.0.into(),
                models: vec!["M".into()],
                frequency: Duration::from_secs(60),
                auto_start: true,
            }
        }
        async fn run(&self, _ctx: &SyncContext) -> Result<()> {
            Ok(())
        }
    }

    struct A(&'static str);

    #[async_trait]
    impl ActionFunction for A {
        fn spec(&self) -> ActionSpec {
            ActionSpec {
                name: self.0.into(),
                integration: "gh".into(),
            }
        }
        async fn run(&self, _ctx: &ActionContext, input: Value) -> Result<Value> {
            Ok(input)
        }
    }

    #[test]
    fn registry_lookup() {
        let r = Registry::new()
            .sync(S("gh", "b"))
            .sync(S("gh", "a"))
            .sync(S("slack", "a"))
            .action(A("create"));
        assert!(r.get_sync("gh", "a").is_some());
        assert!(r.get_sync("gh", "zz").is_none());
        assert!(r.get_action("gh", "create").is_some());
        assert!(r.get_action("slack", "create").is_none());
        let names: Vec<String> = r.syncs_for("gh").iter().map(|s| s.spec().name).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(r.syncs().len(), 3);
        // The same key replaces the old one.
        let r = r.sync(S("gh", "a"));
        assert_eq!(r.syncs().len(), 3);
        assert!(format!("{r:?}").contains("create"));
    }
}
