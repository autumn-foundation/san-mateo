//! Token refresh under concurrency: leases, fences, lost callers, store
//! errors, the sweep, 401 storms and short tokens.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::*;
use san_mateo::clock::OsEntropy;
use san_mateo::credentials::OAuth2Credentials;
use san_mateo::records::{Change, RecordQuery};
use san_mateo::store::{
    ConnectSessionRow, ConnectionFilter, ConnectionRow, IntegrationRow, OAuthSessionRow, RecordRow,
    RecordWrite, SyncStateRow, Upserted,
};
use san_mateo::transport::{HttpRequest, HttpResponse, TransportError};
use san_mateo::{
    Credentials, Error, GetOptions, HttpTransport, ImportConnection, MemoryStore, MemoryTransport,
    ProxyRequest, Registry, Result, SanMateo, Store,
};
use serde_json::json;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

const TOKEN_URL: &str = "https://auth.crm.example.com/token";
const API: &str = "https://api.crm.example.com";

/// The shared state of [`Net`].
struct NetState {
    /// Hold refresh calls until the test adds permits to `open`.
    gate: AtomicBool,
    entered: Notify,
    open: Semaphore,
    refresh_calls: AtomicUsize,
    /// The API gives 401 for this access token.
    stale: Mutex<Option<String>>,
    /// The token URL gives 400 for these refresh tokens.
    rejected: Mutex<Vec<String>>,
}

/// A transport on top of [`MemoryTransport`] with a gate for refresh calls.
#[derive(Clone)]
struct Net {
    http: MemoryTransport,
    state: Arc<NetState>,
}

impl Net {
    fn new(http: MemoryTransport) -> Self {
        Self {
            http,
            state: Arc::new(NetState {
                gate: AtomicBool::new(false),
                entered: Notify::new(),
                open: Semaphore::new(0),
                refresh_calls: AtomicUsize::new(0),
                stale: Mutex::new(None),
                rejected: Mutex::new(Vec::new()),
            }),
        }
    }

    fn hold_refreshes(&self) {
        self.state.gate.store(true, Ordering::SeqCst);
    }

    fn let_go(&self, n: usize) {
        self.state.open.add_permits(n);
    }

    async fn entered(&self) {
        self.state.entered.notified().await;
    }

    fn refresh_calls(&self) -> usize {
        self.state.refresh_calls.load(Ordering::SeqCst)
    }

    fn reject_access_token(&self, token: &str) {
        *self.state.stale.lock().unwrap() = Some(token.to_owned());
    }

    fn reject_refresh_token(&self, token: &str) {
        self.state.rejected.lock().unwrap().push(token.to_owned());
    }
}

#[async_trait]
impl HttpTransport for Net {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> std::result::Result<HttpResponse, TransportError> {
        let form: BTreeMap<String, String> = url::form_urlencoded::parse(&request.body)
            .into_owned()
            .collect();
        let refresh = request.url == TOKEN_URL
            && form.get("grant_type").map(String::as_str) == Some("refresh_token");
        if refresh {
            self.state.refresh_calls.fetch_add(1, Ordering::SeqCst);
            if self.state.gate.load(Ordering::SeqCst) {
                self.state.entered.notify_one();
                self.state.open.acquire().await.unwrap().forget();
            }
            let token = form.get("refresh_token").cloned().unwrap_or_default();
            if self.state.rejected.lock().unwrap().contains(&token) {
                return Ok(HttpResponse::json(400, &json!({"error": "invalid_grant"})));
            }
        }
        let stale = self.state.stale.lock().unwrap().clone();
        if let Some(stale) = stale
            && request.url.starts_with(API)
            && request.header_value("authorization") == Some(format!("Bearer {stale}").as_str())
        {
            return Ok(HttpResponse::new(401, "expired"));
        }
        self.http.send(request).await
    }
}

/// A store that fails some connection writes.
struct Flaky {
    inner: Arc<MemoryStore>,
    fail_updates: AtomicUsize,
}

impl Flaky {
    fn fail_next_updates(&self, n: usize) {
        self.fail_updates.store(n, Ordering::SeqCst);
    }
}

#[async_trait]
impl Store for Flaky {
    async fn insert_integration(&self, row: IntegrationRow) -> Result<()> {
        self.inner.insert_integration(row).await
    }
    async fn update_integration(&self, row: IntegrationRow) -> Result<bool> {
        self.inner.update_integration(row).await
    }
    async fn get_integration(&self, env: &str, key: &str) -> Result<Option<IntegrationRow>> {
        self.inner.get_integration(env, key).await
    }
    async fn list_integrations(&self, env: &str) -> Result<Vec<IntegrationRow>> {
        self.inner.list_integrations(env).await
    }
    async fn delete_integration(&self, env: &str, key: &str) -> Result<bool> {
        self.inner.delete_integration(env, key).await
    }
    async fn upsert_connection(&self, row: ConnectionRow) -> Result<Upserted> {
        self.inner.upsert_connection(row).await
    }
    async fn get_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        self.inner.get_connection(env, key, connection_id).await
    }
    async fn list_connections(
        &self,
        env: &str,
        filter: &ConnectionFilter,
    ) -> Result<Vec<ConnectionRow>> {
        self.inner.list_connections(env, filter).await
    }
    async fn update_connection(&self, row: &ConnectionRow) -> Result<Option<ConnectionRow>> {
        let left = self.fail_updates.load(Ordering::SeqCst);
        if left > 0 {
            self.fail_updates.store(left - 1, Ordering::SeqCst);
            return Err(Error::Store("the database is gone for a moment".into()));
        }
        self.inner.update_connection(row).await
    }
    async fn touch_connection(&self, id: Uuid, at: DateTime<Utc>) -> Result<()> {
        self.inner.touch_connection(id, at).await
    }
    async fn delete_connection(
        &self,
        env: &str,
        key: &str,
        connection_id: &str,
    ) -> Result<Option<ConnectionRow>> {
        self.inner.delete_connection(env, key, connection_id).await
    }
    async fn connections_expiring_before(
        &self,
        before: DateTime<Utc>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
    ) -> Result<Vec<ConnectionRow>> {
        self.inner
            .connections_expiring_before(before, after, limit)
            .await
    }
    async fn try_lease(
        &self,
        name: &str,
        owner: &str,
        now: DateTime<Utc>,
        ttl_ms: u64,
    ) -> Result<bool> {
        self.inner.try_lease(name, owner, now, ttl_ms).await
    }
    async fn release_lease(&self, name: &str, owner: &str) -> Result<()> {
        self.inner.release_lease(name, owner).await
    }
    async fn insert_connect_session(&self, row: ConnectSessionRow) -> Result<()> {
        self.inner.insert_connect_session(row).await
    }
    async fn get_connect_session(&self, token_hash: &str) -> Result<Option<ConnectSessionRow>> {
        self.inner.get_connect_session(token_hash).await
    }
    async fn get_connect_session_by_id(&self, id: Uuid) -> Result<Option<ConnectSessionRow>> {
        self.inner.get_connect_session_by_id(id).await
    }
    async fn delete_connect_session(&self, id: Uuid) -> Result<bool> {
        self.inner.delete_connect_session(id).await
    }
    async fn insert_oauth_session(&self, row: OAuthSessionRow) -> Result<()> {
        self.inner.insert_oauth_session(row).await
    }
    async fn take_oauth_session(&self, state_hash: &str) -> Result<Option<OAuthSessionRow>> {
        self.inner.take_oauth_session(state_hash).await
    }
    async fn purge_expired(&self, now: DateTime<Utc>) -> Result<u64> {
        self.inner.purge_expired(now).await
    }
    async fn upsert_records(
        &self,
        connection: Uuid,
        model: &str,
        rows: Vec<RecordWrite>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        self.inner
            .upsert_records(connection, model, rows, now)
            .await
    }
    async fn delete_records(
        &self,
        connection: Uuid,
        model: &str,
        ids: Vec<(Uuid, String)>,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Change)>> {
        self.inner.delete_records(connection, model, ids, now).await
    }
    async fn list_records(&self, connection: Uuid, query: &RecordQuery) -> Result<Vec<RecordRow>> {
        self.inner.list_records(connection, query).await
    }
    async fn put_sync_state(&self, row: SyncStateRow) -> Result<()> {
        self.inner.put_sync_state(row).await
    }
    async fn get_sync_state(
        &self,
        connection: Uuid,
        sync_name: &str,
    ) -> Result<Option<SyncStateRow>> {
        self.inner.get_sync_state(connection, sync_name).await
    }
    async fn list_sync_states(&self, connection: Uuid) -> Result<Vec<SyncStateRow>> {
        self.inner.list_sync_states(connection).await
    }
}

/// A harness that sends through a [`Net`].
async fn net_harness() -> (Harness, Net) {
    let mut net = None;
    let h = harness_on(
        Registry::new(),
        |http| {
            let n = Net::new(http);
            net = Some(n.clone());
            Arc::new(n) as Arc<dyn HttpTransport>
        },
        |memory| memory as Arc<dyn Store>,
    );
    add_integrations(&h).await;
    (h, net.unwrap())
}

/// A second process on the same store, clock and network.
fn other_process(h: &Harness, net: &Net) -> SanMateo {
    builder(
        h.store.clone(),
        Arc::new(net.clone()),
        h.clock.clone(),
        Registry::new(),
    )
    .entropy(Arc::new(OsEntropy))
    .build()
    .unwrap()
}

/// Returns the stored row and credentials of a `crm` connection.
async fn stored(h: &Harness, id: &str) -> (ConnectionRow, Credentials) {
    let row = h
        .store
        .get_connection("dev", "crm", id)
        .await
        .unwrap()
        .unwrap();
    let creds = h
        .keys
        .open_json(
            &row.credentials,
            &san_mateo::crypto::aad(&["connection", "dev", "crm", id]),
        )
        .unwrap();
    (row, creds)
}

fn new_token(h: &Harness) {
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply("at-2", Some("rt-2"), Some(3_600)),
    );
}

fn failed_refresh_hooks(h: &Harness) -> usize {
    webhooks_sent(h)
        .into_iter()
        .filter(|w| w["operation"] == "refresh" && w["success"] == false)
        .count()
}

// ------------------------------------------------------------ H1: lease life

#[tokio::test(start_paused = true)]
async fn a_slow_token_call_keeps_the_lease_from_other_processes() {
    let (h, net) = net_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    new_token(&h);
    net.hold_refreshes();
    let a = {
        let san = h.san.clone();
        let key = conn.key();
        tokio::spawn(async move { san.get_connection("dev", &key, GetOptions::default()).await })
    };
    net.entered().await;
    // The token call is slow: 30 seconds pass. A process that takes the
    // lease now sends the old refresh token a second time.
    h.clock.advance(chrono::Duration::seconds(30));
    let b = other_process(&h, &net);
    let other = tokio::time::timeout(
        Duration::from_secs(60),
        b.get_connection("dev", &conn.key(), GetOptions::default()),
    )
    .await;
    assert_eq!(net.refresh_calls(), 1, "one refresh call with rt-1");
    assert!(
        matches!(other, Ok(Err(Error::RefreshCooldown))),
        "the other process waits, then gives up: {other:?}"
    );
    net.let_go(10);
    let got = a.await.unwrap().unwrap();
    assert_eq!(got.credentials.refresh_token(), Some("rt-2"));
}

#[tokio::test(start_paused = true)]
async fn the_token_call_ends_before_the_lease_ends() {
    let (h, net) = net_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    net.hold_refreshes();
    let ttl = h.san.config().refresh_lease_ttl;
    assert!(
        ttl > Duration::from_secs(70),
        "the lease must outlive the transport timeouts (60 s + 10 s connect)"
    );
    let got = tokio::time::timeout(
        ttl,
        h.san
            .get_connection("dev", &conn.key(), GetOptions::default()),
    )
    .await;
    assert!(
        matches!(
            got,
            Ok(Err(Error::Provider {
                code: "refresh_failed",
                ..
            }))
        ),
        "{got:?}"
    );
}

// ------------------------------------------------------ H2: fence on creds

fn import(h: &Harness, id: &str) -> ImportConnection {
    ImportConnection {
        connection_id: Some(id.to_owned()),
        provider_config_key: "crm".into(),
        credentials: Credentials::OAuth2(OAuth2Credentials {
            access_token: "at-new".into(),
            refresh_token: Some("rt-new".into()),
            expires_at: Some(now(h) + chrono::Duration::hours(1)),
            raw: json!({}),
        }),
        connection_config: serde_json::Map::new(),
        metadata: serde_json::Map::new(),
        tags: BTreeMap::new(),
        end_user: None,
        webhook_url_override: None,
    }
}

#[tokio::test]
async fn a_reconnect_during_a_refresh_keeps_the_new_credentials() {
    let (h, net) = net_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    new_token(&h);
    net.hold_refreshes();
    let a = {
        let san = h.san.clone();
        let key = conn.key();
        tokio::spawn(async move { san.get_connection("dev", &key, GetOptions::default()).await })
    };
    net.entered().await;
    h.san
        .import_connection("dev", import(&h, &conn.connection_id))
        .await
        .unwrap();
    net.let_go(1);
    let got = a.await.unwrap().unwrap();
    let (_, creds) = stored(&h, &conn.connection_id).await;
    assert_eq!(creds.bearer_token(), Some("at-new"), "the import wins");
    assert_eq!(creds.refresh_token(), Some("rt-new"));
    assert_eq!(got.credentials.bearer_token(), Some("at-new"));
}

#[tokio::test]
async fn a_failed_refresh_after_a_reconnect_records_no_failure() {
    let (h, net) = net_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    net.reject_refresh_token("rt-1");
    net.hold_refreshes();
    let a = {
        let san = h.san.clone();
        let key = conn.key();
        tokio::spawn(async move { san.get_connection("dev", &key, GetOptions::default()).await })
    };
    net.entered().await;
    h.san
        .import_connection("dev", import(&h, &conn.connection_id))
        .await
        .unwrap();
    net.let_go(1);
    let got = a.await.unwrap();
    let (row, creds) = stored(&h, &conn.connection_id).await;
    assert_eq!(creds.bearer_token(), Some("at-new"));
    assert_eq!(row.last_refresh_failure, None);
    assert_eq!(row.refresh_attempts, 0);
    assert_eq!(
        got.unwrap().credentials.bearer_token(),
        Some("at-new"),
        "the new credentials are valid"
    );
    h.san.flush().await;
    assert_eq!(failed_refresh_hooks(&h), 0);
}

// ------------------------------------------- H3: lost callers, writes, reads

#[tokio::test]
async fn a_caller_that_goes_away_does_not_lose_rotated_tokens() {
    let (h, net) = net_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    new_token(&h);
    net.hold_refreshes();
    let key = conn.key();
    let mut call = Box::pin(h.san.get_connection("dev", &key, GetOptions::default()));
    tokio::select! {
        r = &mut call => panic!("the call must wait at the gate: {r:?}"),
        () = net.entered() => {}
    }
    // The client goes away while the provider rotates the refresh token.
    drop(call);
    net.let_go(1);
    h.san.flush().await;
    let (_, creds) = stored(&h, &conn.connection_id).await;
    assert_eq!(
        creds.refresh_token(),
        Some("rt-2"),
        "the new tokens are kept"
    );
    assert_eq!(creds.bearer_token(), Some("at-2"));
}

#[tokio::test]
async fn reads_do_not_change_the_row_version() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    let before = h
        .store
        .get_connection("dev", "crm", &conn.connection_id)
        .await
        .unwrap()
        .unwrap();
    h.http.reply(
        "GET",
        &format!("{API}/v3/deals"),
        HttpResponse::new(200, "[]"),
    );
    for _ in 0..3 {
        h.clock.advance(chrono::Duration::seconds(1));
        h.san
            .get_connection("dev", &conn.key(), GetOptions::default())
            .await
            .unwrap();
        h.san
            .proxy("dev", &conn.key(), ProxyRequest::get("/v3/deals"))
            .await
            .unwrap();
    }
    let after = h
        .store
        .get_connection("dev", "crm", &conn.connection_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.version, before.version, "a read is not a CAS write");
    assert_eq!(after.last_fetched_at, Some(now(&h)));
}

#[tokio::test(start_paused = true)]
async fn a_refresh_write_survives_store_errors() {
    let mut flaky = None;
    let h = harness_on(
        Registry::new(),
        |http| Arc::new(http) as Arc<dyn HttpTransport>,
        |memory| {
            let f = Arc::new(Flaky {
                inner: memory,
                fail_updates: AtomicUsize::new(0),
            });
            flaky = Some(f.clone());
            f as Arc<dyn Store>
        },
    );
    let flaky = flaky.unwrap();
    add_integrations(&h).await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    new_token(&h);
    flaky.fail_next_updates(2);
    let got = h
        .san
        .get_connection("dev", &conn.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.refresh_token(), Some("rt-2"));
    let (_, creds) = stored(&h, &conn.connection_id).await;
    assert_eq!(creds.refresh_token(), Some("rt-2"));
}

// ----------------------------------------------------- M4: sweep starvation

#[tokio::test]
async fn the_sweep_reaches_due_rows_after_skipped_ones() {
    let h = ready().await;
    // Two tokens with no refresh token expire first.
    connect_crm(&h, "at-a", None, Some(60)).await;
    connect_crm(&h, "at-b", None, Some(120)).await;
    let c = connect_crm(&h, "at-c", Some("rt-c"), Some(600)).await;
    new_token(&h);
    let report = h
        .san
        .refresh_due(Duration::from_secs(30 * 60), 1)
        .await
        .unwrap();
    assert_eq!((report.refreshed, report.skipped), (1, 2), "{report:?}");
    let (_, creds) = stored(&h, &c.connection_id).await;
    assert_eq!(creds.bearer_token(), Some("at-2"));
}

#[tokio::test]
async fn failing_rows_do_not_starve_the_sweep() {
    let (h, net) = net_harness().await;
    connect_crm(&h, "at-f1", Some("rt-f1"), Some(60)).await;
    connect_crm(&h, "at-f2", Some("rt-f2"), Some(120)).await;
    let good = connect_crm(&h, "at-g", Some("rt-g"), Some(600)).await;
    net.reject_refresh_token("rt-f1");
    net.reject_refresh_token("rt-f2");
    new_token(&h);
    for _ in 0..3 {
        h.san
            .refresh_due(Duration::from_secs(30 * 60), 1)
            .await
            .unwrap();
        // Each tick is after the cooldown of the last failure.
        h.clock.advance(chrono::Duration::seconds(31));
    }
    let (_, creds) = stored(&h, &good.connection_id).await;
    assert_eq!(
        creds.bearer_token(),
        Some("at-2"),
        "the good row is reached"
    );
}

// --------------------------------------------------- M5: short-lived tokens

#[tokio::test]
async fn a_short_token_is_not_refreshed_on_every_tick() {
    let (h, net) = net_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(300)).await;
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply("at-n", Some("rt-n"), Some(300)),
    );
    for tick in 0..10 {
        h.san
            .refresh_due(Duration::from_secs(30 * 60), 100)
            .await
            .unwrap();
        let (row, _) = stored(&h, &conn.connection_id).await;
        assert!(
            row.credentials_expires_at.unwrap() > now(&h),
            "always valid"
        );
        if tick == 1 {
            // A read does not refresh a token in the first half of its life.
            h.san
                .get_connection("dev", &conn.key(), GetOptions::default())
                .await
                .unwrap();
        }
        h.clock.advance(chrono::Duration::seconds(30));
    }
    // Ticks at 0 s to 270 s: a refresh at 0 s and one at half life (150 s).
    assert_eq!(net.refresh_calls(), 2);
}

// ------------------------------------------------ M6: forced refresh storms

#[tokio::test]
async fn concurrent_401s_share_one_refresh() {
    let (h, net) = net_harness().await;
    let key = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600))
        .await
        .key();
    net.reject_access_token("at-1");
    new_token(&h);
    h.http.reply(
        "GET",
        &format!("{API}/v3/deals"),
        HttpResponse::new(200, "[]"),
    );
    net.hold_refreshes();
    let calls = {
        let san = h.san.clone();
        tokio::spawn(async move {
            let calls = (0..20).map(|_| san.proxy("dev", &key, ProxyRequest::get("/v3/deals")));
            futures::future::join_all(calls).await
        })
    };
    // All calls get a 401 while the first refresh waits at the gate.
    net.entered().await;
    net.let_go(20);
    let results = calls.await.unwrap();
    assert!(results.iter().all(|r| r.as_ref().unwrap().status == 200));
    assert_eq!(net.refresh_calls(), 1);
}

#[tokio::test]
async fn concurrent_forced_refreshes_share_one_refresh() {
    let (h, net) = net_harness().await;
    let key = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600))
        .await
        .key();
    new_token(&h);
    let force = GetOptions {
        force_refresh: true,
        ..GetOptions::default()
    };
    let calls = (0..10).map(|_| h.san.get_connection("dev", &key, force));
    let results = futures::future::join_all(calls).await;
    assert!(
        results
            .iter()
            .all(|r| r.as_ref().unwrap().credentials.bearer_token() == Some("at-2"))
    );
    assert_eq!(net.refresh_calls(), 1);
}

#[tokio::test]
async fn a_401_respects_cooldown_and_exhaustion() {
    let (h, net) = net_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    let key = conn.key();
    net.reject_access_token("at-1");
    net.reject_refresh_token("rt-1");
    let deals = || h.san.proxy("dev", &key, ProxyRequest::get("/v3/deals"));
    assert_eq!(deals().await.unwrap().status, 401);
    assert_eq!(net.refresh_calls(), 1);
    // In the cooldown, a 401 does not call the provider.
    assert_eq!(deals().await.unwrap().status, 401);
    assert_eq!(net.refresh_calls(), 1);
    // An exhausted connection does not refresh on a 401.
    let (mut row, _) = stored(&h, &conn.connection_id).await;
    row.refresh_exhausted = true;
    h.store.update_connection(&row).await.unwrap().unwrap();
    h.clock.advance(chrono::Duration::minutes(5));
    assert_eq!(deals().await.unwrap().status, 401);
    assert_eq!(net.refresh_calls(), 1);
    h.san.flush().await;
    assert_eq!(failed_refresh_hooks(&h), 1);
}

// ---------------------------------------- L12: a valid token survives errors

#[tokio::test]
async fn a_failed_refresh_keeps_a_token_that_is_still_valid() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    let key = conn.key();
    // 10 minutes remain: inside the 15 minute buffer.
    h.clock.advance(chrono::Duration::minutes(50));
    h.http.reply("POST", TOKEN_URL, HttpResponse::new(503, ""));
    let got = h
        .san
        .get_connection("dev", &key, GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-1"));
    let (row, _) = stored(&h, &conn.connection_id).await;
    assert_eq!(
        row.last_refresh_failure,
        Some(now(&h)),
        "the failure counts"
    );
    // In the cooldown, the valid token comes back with no provider call.
    let calls = h.http.requests_to(TOKEN_URL).len();
    let got = h
        .san
        .get_connection("dev", &key, GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-1"));
    assert_eq!(h.http.requests_to(TOKEN_URL).len(), calls);
    // A forced refresh and the sweep report the failure.
    let forced = h
        .san
        .get_connection(
            "dev",
            &key,
            GetOptions {
                force_refresh: true,
                ..GetOptions::default()
            },
        )
        .await;
    assert!(matches!(forced, Err(Error::Provider { .. })), "{forced:?}");
    h.clock.advance(chrono::Duration::seconds(31));
    let report = h
        .san
        .refresh_due(Duration::from_secs(30 * 60), 100)
        .await
        .unwrap();
    assert_eq!(report.failed, 1);
    // An expired token gives the error.
    h.clock.advance(chrono::Duration::minutes(11));
    assert!(
        h.san
            .get_connection("dev", &key, GetOptions::default())
            .await
            .is_err()
    );
}
