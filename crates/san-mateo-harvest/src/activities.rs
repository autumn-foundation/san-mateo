//! Harvest activities for San Mateo.
//!
//! Each activity reads [`SanMateo`] from the harvest state. Each input holds
//! ids and options only. San Mateo adds the credentials, so no secret goes
//! into the harvest history. No output holds a token.
//!
//! Each input type also has a method that runs the same work with a given
//! [`SanMateo`]. Use it in tests, or to run the work with no harvest worker.

use std::collections::BTreeMap;
use std::time::Duration;

use autumn_harvest::context::ActivityContext;
use autumn_harvest::failure::ActivityFailure;
use autumn_harvest::policy::RetryPolicy;
use autumn_harvest::prelude::activity;
use san_mateo::store::SyncCounts;
use san_mateo::{ConnectionKey, ProxyRequest, ProxyResponse, RefreshReport, SanMateo, SyncRunReport};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::proxy::san_mateo_from;

/// The default of [`RefreshDueInput::within_secs`]: 30 minutes.
pub const DEFAULT_REFRESH_WITHIN_SECS: u64 = 30 * 60;

/// The default of [`RefreshDueInput::limit`].
pub const DEFAULT_REFRESH_LIMIT: usize = 100;

/// The input of [`san_mateo_proxy`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyInput {
    /// The environment id.
    pub environment: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
    /// The HTTP method, for example `GET`.
    pub method: String,
    /// The path and query, for example `/v3/contacts`.
    pub endpoint: String,
    /// Headers to send upstream. Secret headers such as `Authorization` are
    /// not allowed, because the input goes into the harvest history.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// A JSON body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// Query parameters to add.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    /// Send the harvest idempotency key as `Idempotency-Key`.
    #[serde(default)]
    pub forward_idempotency_key: bool,
}

impl ProxyInput {
    /// Makes an input with no headers, no body and no query.
    #[must_use]
    pub fn new(
        environment: impl Into<String>,
        key: &ConnectionKey,
        method: &str,
        endpoint: impl Into<String>,
    ) -> Self {
        let _ = (environment, key, method, endpoint);
        todo!()
    }

    /// Adds a header.
    #[must_use]
    pub fn header(self, name: &str, value: impl Into<String>) -> Self {
        let _ = (name, value);
        todo!()
    }

    /// Sets a JSON body.
    #[must_use]
    pub fn json(self, body: Value) -> Self {
        let _ = body;
        todo!()
    }

    /// Adds a query parameter.
    #[must_use]
    pub fn query(self, name: &str, value: impl Into<String>) -> Self {
        let _ = (name, value);
        todo!()
    }

    /// Sets [`Self::forward_idempotency_key`].
    #[must_use]
    pub fn forward_idempotency_key(self, forward: bool) -> Self {
        let _ = (self, forward);
        todo!()
    }

    /// Returns the connection key.
    #[must_use]
    pub fn key(&self) -> ConnectionKey {
        todo!()
    }

    /// Returns the proxy request.
    ///
    /// # Errors
    ///
    /// A non-retryable `san_mateo.invalid_input` failure for a secret header.
    pub fn to_request(&self) -> Result<ProxyRequest, ActivityFailure> {
        todo!()
    }

    /// Sends the call with a given [`SanMateo`].
    ///
    /// # Errors
    ///
    /// The errors of [`Self::to_request`] and [`crate::proxy_with`].
    pub async fn send(
        &self,
        san: &SanMateo,
        ctx: &ActivityContext,
    ) -> Result<ProxyOutput, ActivityFailure> {
        let _ = (san, ctx);
        todo!()
    }
}

/// The output of [`san_mateo_proxy`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyOutput {
    /// The HTTP status.
    pub status: u16,
    /// The reply headers, with no secret headers such as `Set-Cookie`.
    pub headers: Vec<(String, String)>,
    /// The body. It is JSON when the reply is JSON, `null` when it is
    /// empty, and else a string.
    pub body: Value,
}

impl ProxyOutput {
    /// Makes the output of a proxy reply.
    #[must_use]
    pub fn from_response(response: &ProxyResponse) -> Self {
        let _ = response;
        todo!()
    }

    /// Returns `true` for a 2xx status.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        todo!()
    }
}

/// The input of [`san_mateo_refresh`]: one connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInput {
    /// The environment id.
    pub environment: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
}

impl ConnectionInput {
    /// Makes an input.
    #[must_use]
    pub fn new(environment: impl Into<String>, key: &ConnectionKey) -> Self {
        let _ = (environment, key);
        todo!()
    }

    /// Returns the connection key.
    #[must_use]
    pub fn key(&self) -> ConnectionKey {
        todo!()
    }

    /// Refreshes the token now with a given [`SanMateo`].
    ///
    /// # Errors
    ///
    /// The failure of [`crate::failure_for_error`] for a San Mateo error.
    pub async fn refresh(&self, san: &SanMateo) -> Result<RefreshOutput, ActivityFailure> {
        let _ = san;
        todo!()
    }
}

/// The output of [`san_mateo_refresh`]. It holds no token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshOutput {
    /// The new expiry time in RFC 3339 format, if the token expires.
    pub expires_at: Option<String>,
}

/// The input of [`san_mateo_run_sync`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSyncInput {
    /// The environment id.
    pub environment: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
    /// The sync name.
    pub sync_name: String,
}

impl RunSyncInput {
    /// Makes an input.
    #[must_use]
    pub fn new(
        environment: impl Into<String>,
        key: &ConnectionKey,
        sync_name: impl Into<String>,
    ) -> Self {
        let _ = (environment, key, sync_name);
        todo!()
    }

    /// Returns the connection key.
    #[must_use]
    pub fn key(&self) -> ConnectionKey {
        todo!()
    }

    /// Runs the sync with a given [`SanMateo`].
    ///
    /// # Errors
    ///
    /// A retryable `san_mateo.sync_failed` failure when the run fails. The
    /// failure of [`crate::failure_for_error`] when the run cannot start.
    pub async fn run(&self, san: &SanMateo) -> Result<SyncRunOutput, ActivityFailure> {
        let _ = san;
        todo!()
    }
}

/// The output of [`san_mateo_run_sync`]: a [`SyncRunReport`] as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRunOutput {
    /// The sync name.
    pub sync_name: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
    /// `true` when the run succeeded.
    pub success: bool,
    /// The record counts of the run.
    pub counts: SyncCounts,
    /// The error, when the run failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<SyncRunReport> for SyncRunOutput {
    fn from(report: SyncRunReport) -> Self {
        let _ = report;
        todo!()
    }
}

/// The input of [`san_mateo_run_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunActionInput {
    /// The environment id.
    pub environment: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
    /// The action name.
    pub action_name: String,
    /// The action input.
    #[serde(default)]
    pub input: Value,
}

impl RunActionInput {
    /// Makes an input.
    #[must_use]
    pub fn new(
        environment: impl Into<String>,
        key: &ConnectionKey,
        action_name: impl Into<String>,
        input: Value,
    ) -> Self {
        let _ = (environment, key, action_name, input);
        todo!()
    }

    /// Returns the connection key.
    #[must_use]
    pub fn key(&self) -> ConnectionKey {
        todo!()
    }

    /// Runs the action with a given [`SanMateo`] and returns its output.
    ///
    /// # Errors
    ///
    /// The failure of [`crate::failure_for_error`] for a San Mateo error.
    pub async fn run(&self, san: &SanMateo) -> Result<Value, ActivityFailure> {
        let _ = san;
        todo!()
    }
}

/// The input of [`san_mateo_refresh_due`]. All fields have defaults, so `{}`
/// is a valid input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshDueInput {
    /// Refresh the tokens that expire within this number of seconds.
    #[serde(default = "default_within_secs")]
    pub within_secs: u64,
    /// The largest number of refreshes in one run.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

const fn default_within_secs() -> u64 {
    DEFAULT_REFRESH_WITHIN_SECS
}

const fn default_limit() -> usize {
    DEFAULT_REFRESH_LIMIT
}

impl Default for RefreshDueInput {
    fn default() -> Self {
        Self {
            within_secs: DEFAULT_REFRESH_WITHIN_SECS,
            limit: DEFAULT_REFRESH_LIMIT,
        }
    }
}

impl RefreshDueInput {
    /// Refreshes the tokens that expire soon, with a given [`SanMateo`].
    ///
    /// # Errors
    ///
    /// The failure of [`crate::failure_for_error`] for a San Mateo error.
    pub async fn run(&self, san: &SanMateo) -> Result<RefreshDueOutput, ActivityFailure> {
        let _ = san;
        todo!()
    }
}

/// The output of [`san_mateo_refresh_due`]: counts only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshDueOutput {
    /// Connections with new tokens.
    pub refreshed: usize,
    /// Connections with a failed refresh.
    pub failed: usize,
    /// Connections that the sweep did not refresh.
    pub skipped: usize,
}

impl From<RefreshReport> for RefreshDueOutput {
    fn from(report: RefreshReport) -> Self {
        let _ = report;
        todo!()
    }
}

/// Sends one call through the San Mateo proxy.
///
/// A 4xx or 5xx reply is a failure (see [`crate::failure_for_status`]).
///
/// # Errors
///
/// The errors of [`ProxyInput::send`], and a `san_mateo.config` failure when
/// the harvest state has no [`SanMateo`].
#[activity(
    start_to_close = "60s",
    retry = RetryPolicy::exponential(5, Duration::from_secs(1))
)]
pub async fn san_mateo_proxy(
    ctx: &ActivityContext,
    input: ProxyInput,
) -> Result<ProxyOutput, ActivityFailure> {
    let _ = (ctx, input, san_mateo_from);
    todo!()
}

/// Refreshes the token of one connection now.
///
/// # Errors
///
/// The errors of [`ConnectionInput::refresh`], and a `san_mateo.config`
/// failure when the harvest state has no [`SanMateo`].
#[activity(
    start_to_close = "30s",
    retry = RetryPolicy::exponential(5, Duration::from_secs(2))
)]
pub async fn san_mateo_refresh(
    ctx: &ActivityContext,
    input: ConnectionInput,
) -> Result<RefreshOutput, ActivityFailure> {
    let _ = (ctx, input);
    todo!()
}

/// Runs one registered sync for one connection.
///
/// # Errors
///
/// The errors of [`RunSyncInput::run`], and a `san_mateo.config` failure
/// when the harvest state has no [`SanMateo`].
#[activity(
    start_to_close = "1h",
    retry = RetryPolicy::exponential(3, Duration::from_secs(10))
)]
pub async fn san_mateo_run_sync(
    ctx: &ActivityContext,
    input: RunSyncInput,
) -> Result<SyncRunOutput, ActivityFailure> {
    let _ = (ctx, input);
    todo!()
}

/// Runs one registered action for one connection.
///
/// # Errors
///
/// The errors of [`RunActionInput::run`], and a `san_mateo.config` failure
/// when the harvest state has no [`SanMateo`].
#[activity(
    start_to_close = "5m",
    retry = RetryPolicy::exponential(3, Duration::from_secs(2))
)]
pub async fn san_mateo_run_action(
    ctx: &ActivityContext,
    input: RunActionInput,
) -> Result<Value, ActivityFailure> {
    let _ = (ctx, input);
    todo!()
}

/// Refreshes the tokens that expire soon, in all environments.
///
/// # Errors
///
/// The errors of [`RefreshDueInput::run`], and a `san_mateo.config` failure
/// when the harvest state has no [`SanMateo`].
#[activity(
    start_to_close = "10m",
    retry = RetryPolicy::exponential(3, Duration::from_secs(5))
)]
pub async fn san_mateo_refresh_due(
    ctx: &ActivityContext,
    input: RefreshDueInput,
) -> Result<RefreshDueOutput, ActivityFailure> {
    let _ = (ctx, input);
    todo!()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    fn key() -> ConnectionKey {
        ConnectionKey::new("crm", "c-1")
    }

    #[test]
    fn proxy_input_builds_a_request() {
        let input = ProxyInput::new("dev", &key(), "post", "/v3/contacts")
            .header("X-Trace", "t-1")
            .json(json!({"name": "Ada"}))
            .query("limit", "10")
            .forward_idempotency_key(true);
        assert_eq!(input.method, "POST");
        assert_eq!(input.key(), key());
        assert!(input.forward_idempotency_key);
        let req = input.to_request().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.endpoint, "/v3/contacts");
        assert_eq!(req.retries, 0);
        assert!(req.headers.contains(&("x-trace".into(), "t-1".into())));
        assert!(
            req.headers
                .contains(&("content-type".into(), "application/json".into()))
        );
        assert_eq!(req.params, vec![("limit".to_owned(), "10".to_owned())]);
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body, json!({"name": "Ada"}));
    }

    #[test]
    fn proxy_input_json_shape() {
        let input: ProxyInput = serde_json::from_value(json!({
            "environment": "dev",
            "provider_config_key": "crm",
            "connection_id": "c-1",
            "method": "GET",
            "endpoint": "/v3/contacts"
        }))
        .unwrap();
        assert!(input.headers.is_empty());
        assert!(input.body.is_none());
        assert!(!input.forward_idempotency_key);
        let req = input.to_request().unwrap();
        assert!(req.body.is_empty());
        assert!(req.headers.is_empty());
        let back = serde_json::to_value(&input).unwrap();
        assert!(back.get("headers").is_none());
        assert!(back.get("body").is_none());
    }

    #[test]
    fn proxy_input_rejects_secret_headers() {
        for name in ["Authorization", "proxy-authorization", "Cookie", "X-Api-Key"] {
            let input = ProxyInput::new("dev", &key(), "GET", "/x").header(name, "secret-1");
            let f = input.to_request().unwrap_err();
            assert_eq!(f.error_type, crate::INVALID_INPUT_ERROR, "{name}");
            assert!(f.non_retryable);
            assert!(!f.message.contains("secret-1"), "{}", f.message);
        }
    }

    fn response(headers: &[(&str, &str)], body: &[u8]) -> ProxyResponse {
        ProxyResponse {
            status: 200,
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            body: body.to_vec(),
        }
    }

    #[test]
    fn proxy_output_body_kinds() {
        let out = ProxyOutput::from_response(&response(
            &[("content-type", "application/json; charset=utf-8")],
            br#"{"a":1}"#,
        ));
        assert_eq!(out.body, json!({"a": 1}));
        assert!(out.is_success());
        let out = ProxyOutput::from_response(&response(
            &[("content-type", "application/vnd.api+json")],
            b"[1,2]",
        ));
        assert_eq!(out.body, json!([1, 2]));
        let out = ProxyOutput::from_response(&response(&[("content-type", "text/plain")], b"42"));
        assert_eq!(out.body, json!("42"));
        let out = ProxyOutput::from_response(&response(&[], br#"{"no_type":true}"#));
        assert_eq!(out.body, json!({"no_type": true}));
        let out = ProxyOutput::from_response(&response(&[], b"plain words"));
        assert_eq!(out.body, json!("plain words"));
        let out = ProxyOutput::from_response(&response(
            &[("content-type", "application/json")],
            b"not json",
        ));
        assert_eq!(out.body, json!("not json"));
        let out = ProxyOutput::from_response(&response(&[], b""));
        assert_eq!(out.body, Value::Null);
    }

    #[test]
    fn proxy_output_drops_secret_headers() {
        let out = ProxyOutput::from_response(&response(
            &[
                ("content-type", "text/plain"),
                ("Set-Cookie", "session=secret-1"),
                ("authorization", "Bearer secret-2"),
                ("x-request-id", "r-1"),
            ],
            b"ok",
        ));
        assert_eq!(
            out.headers,
            vec![
                ("content-type".to_owned(), "text/plain".to_owned()),
                ("x-request-id".to_owned(), "r-1".to_owned()),
            ]
        );
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("secret"), "{text}");
        let mut not_ok = out;
        not_ok.status = 302;
        assert!(!not_ok.is_success());
    }

    #[test]
    fn connection_inputs_give_keys() {
        assert_eq!(ConnectionInput::new("dev", &key()).key(), key());
        let sync = RunSyncInput::new("dev", &key(), "contacts");
        assert_eq!(sync.key(), key());
        assert_eq!(sync.sync_name, "contacts");
        let action = RunActionInput::new("dev", &key(), "create", json!({"a": 1}));
        assert_eq!(action.key(), key());
        assert_eq!(action.input, json!({"a": 1}));
        let action: RunActionInput = serde_json::from_value(json!({
            "environment": "dev",
            "provider_config_key": "crm",
            "connection_id": "c-1",
            "action_name": "ping"
        }))
        .unwrap();
        assert_eq!(action.input, Value::Null);
    }

    #[test]
    fn refresh_due_input_defaults() {
        let input: RefreshDueInput = serde_json::from_value(json!({})).unwrap();
        assert_eq!(input, RefreshDueInput::default());
        assert_eq!(input.within_secs, DEFAULT_REFRESH_WITHIN_SECS);
        assert_eq!(input.limit, DEFAULT_REFRESH_LIMIT);
        let out = RefreshDueOutput::from(RefreshReport {
            refreshed: 2,
            failed: 1,
            skipped: 3,
        });
        assert_eq!(
            serde_json::to_value(out).unwrap(),
            json!({"refreshed": 2, "failed": 1, "skipped": 3})
        );
    }

    #[test]
    fn sync_run_output_from_report() {
        let out = SyncRunOutput::from(SyncRunReport {
            sync_name: "contacts".into(),
            connection: key(),
            success: true,
            counts: SyncCounts {
                added: 2,
                updated: 1,
                deleted: 0,
            },
            error: None,
        });
        assert_eq!(
            serde_json::to_value(&out).unwrap(),
            json!({
                "sync_name": "contacts",
                "provider_config_key": "crm",
                "connection_id": "c-1",
                "success": true,
                "counts": {"added": 2, "updated": 1, "deleted": 0}
            })
        );
    }

    #[test]
    fn activity_infos_have_timeouts_and_retries() {
        for info in [
            san_mateo_proxy_info(),
            san_mateo_refresh_info(),
            san_mateo_run_sync_info(),
            san_mateo_run_action_info(),
            san_mateo_refresh_due_info(),
        ] {
            assert!(info.default_start_to_close.is_some(), "{}", info.name);
            let retry = info.default_retry_policy.as_ref().unwrap();
            assert!(retry.max_attempts > 1, "{}", info.name);
            assert!(!info.is_local, "{}", info.name);
        }
        assert_eq!(san_mateo_proxy_info().name, "san_mateo_proxy");
    }
}
