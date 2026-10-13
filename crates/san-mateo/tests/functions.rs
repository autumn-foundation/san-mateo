//! Syncs and actions (AC15).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::*;
use san_mateo::clock::{ManualClock, OsEntropy};
use san_mateo::model::ConnectionKey;
use san_mateo::records::RecordQuery;
use san_mateo::store::Store;
use san_mateo::store::SyncStatus;
use san_mateo::transport::HttpResponse;
use san_mateo::{
    ActionContext, ActionFunction, ActionSpec, BackgroundSettings, Error, GetOptions,
    HttpTransport, PaginateOptions, ProxyRequest, Registry, Result, SanMateo, SyncContext,
    SyncFunction, SyncSpec,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

struct Contacts {
    runs: Arc<AtomicUsize>,
    gate: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait]
impl SyncFunction for Contacts {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "contacts".into(),
            integration: "crm".into(),
            models: vec!["Contact".into()],
            frequency: Duration::from_secs(3_600),
            auto_start: true,
        }
    }

    async fn run(&self, ctx: &SyncContext) -> Result<()> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if let Some(g) = &self.gate {
            g.notified().await;
        }
        let since = ctx.last_sync_date().map(|d| d.to_rfc3339());
        let mut req = ProxyRequest::get("/v3/contacts");
        if let Some(s) = since {
            req = req.query("since", s);
        }
        let records = ctx.paginate(req, PaginateOptions::default()).await?;
        ctx.batch_save("Contact", records).await?;
        let gone = ctx
            .get("/v3/contacts/deleted")
            .await?
            .json::<Vec<Value>>()?;
        ctx.batch_delete("Contact", gone).await?;
        let meta = ctx.metadata().await?;
        assert!(meta.is_object());
        assert_eq!(ctx.environment(), "dev");
        assert_eq!(ctx.connection().provider_config_key, "crm");
        Ok(())
    }
}

struct Broken;

#[async_trait]
impl SyncFunction for Broken {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "broken".into(),
            integration: "crm".into(),
            models: vec!["X".into()],
            frequency: Duration::from_secs(60),
            auto_start: false,
        }
    }

    async fn run(&self, _ctx: &SyncContext) -> Result<()> {
        Err(Error::Function("upstream shape changed".into()))
    }
}

struct CreateNote;

#[async_trait]
impl ActionFunction for CreateNote {
    fn spec(&self) -> ActionSpec {
        ActionSpec {
            name: "create-note".into(),
            integration: "crm".into(),
        }
    }

    async fn run(&self, ctx: &ActionContext, input: Value) -> Result<Value> {
        if input["text"].is_null() {
            return Err(Error::Invalid("text is required".into()));
        }
        let res = ctx
            .proxy(ProxyRequest::new("POST", "/v3/notes").json(&input))
            .await?;
        assert_eq!(ctx.connection().provider_config_key, "crm");
        assert_eq!(ctx.environment(), "dev");
        let _ = ctx.metadata().await?;
        let _ = ctx
            .paginate(
                ProxyRequest::get("/v3/contacts"),
                PaginateOptions::default(),
            )
            .await?;
        res.json()
    }
}

fn registry(runs: &Arc<AtomicUsize>, gate: Option<Arc<tokio::sync::Notify>>) -> Registry {
    Registry::new()
        .sync(Contacts {
            runs: runs.clone(),
            gate,
        })
        .sync(Broken)
        .action(CreateNote)
}

fn stub_crm(h: &Harness) {
    h.http.reply(
        "GET",
        "https://api.crm.example.com/v3/contacts",
        HttpResponse::json(
            200,
            &json!({"results": [{"id": "1", "n": "a"}, {"id": "2", "n": "b"}]}),
        ),
    );
    h.http.reply(
        "GET",
        "https://api.crm.example.com/v3/contacts/deleted",
        HttpResponse::json(200, &json!([{"id": "2"}])),
    );
}

#[tokio::test]
async fn a_sync_run_saves_records_and_reports() {
    let runs = Arc::new(AtomicUsize::new(0));
    let h = ready_with(registry(&runs, None)).await;
    let key = connect_crm(&h, "at", None, None).await.key();
    stub_crm(&h);
    let report = h.san.run_sync("dev", &key, "contacts").await.unwrap();
    assert!(report.success, "{report:?}");
    assert_eq!(
        (
            report.counts.added,
            report.counts.updated,
            report.counts.deleted
        ),
        (2, 0, 1)
    );
    h.clock.advance(chrono::Duration::minutes(1));
    let report = h.san.run_sync("dev", &key, "contacts").await.unwrap();
    assert_eq!(
        (
            report.counts.added,
            report.counts.updated,
            report.counts.deleted
        ),
        (1, 0, 1),
        "record 2 comes back, then goes again"
    );
    let second = h
        .http
        .requests_to("https://api.crm.example.com/v3/contacts")
        .pop()
        .unwrap();
    assert!(
        second.url.contains("since="),
        "the second run reads changes since the first: {}",
        second.url
    );

    let status = h
        .san
        .sync_status("dev", "crm", Some(&key.connection_id), &["contacts".into()])
        .await
        .unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].status, SyncStatus::Success);
    assert_eq!(status[0].frequency, Duration::from_secs(3_600));
    assert_eq!(
        status[0].next_scheduled_at,
        Some(now(&h) + chrono::Duration::hours(1))
    );

    h.san.flush().await;
    let syncs: Vec<_> = webhooks_sent(&h)
        .into_iter()
        .filter(|w| w["type"] == "sync")
        .collect();
    assert_eq!(syncs.len(), 2);
    assert_eq!(syncs[0]["syncName"], "contacts");
    assert_eq!(syncs[0]["responseResults"]["added"], 2);
    assert_eq!(syncs[0]["success"], true);
}

#[tokio::test]
async fn a_failed_sync_reports_the_error() {
    let runs = Arc::new(AtomicUsize::new(0));
    let h = ready_with(registry(&runs, None)).await;
    let key = connect_crm(&h, "at", None, None).await.key();
    let report = h.san.run_sync("dev", &key, "broken").await.unwrap();
    assert!(!report.success);
    assert!(report.error.unwrap().contains("upstream shape changed"));
    let status = h.san.sync_status("dev", "crm", None, &[]).await.unwrap();
    let broken = status.iter().find(|s| s.name == "broken").unwrap();
    assert_eq!(broken.status, SyncStatus::Error);
    h.san.flush().await;
    let hook = webhooks_sent(&h)
        .into_iter()
        .find(|w| w["type"] == "sync")
        .unwrap();
    assert_eq!(hook["success"], false);
    assert_eq!(hook["error"]["type"], "function_failed");
    assert!(matches!(
        h.san.run_sync("dev", &key, "nope").await,
        Err(Error::UnknownFunction(_))
    ));
    assert!(matches!(
        h.san
            .run_sync("dev", &ConnectionKey::new("crm", "nope"), "broken")
            .await,
        Err(Error::UnknownConnection(_))
    ));
}

#[tokio::test]
async fn one_run_at_a_time_for_each_connection_and_sync() {
    let runs = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(tokio::sync::Notify::new());
    let h = ready_with(registry(&runs, Some(gate.clone()))).await;
    let key = connect_crm(&h, "at", None, None).await.key();
    stub_crm(&h);
    let san = h.san.clone();
    let k2 = key.clone();
    let first = tokio::spawn(async move { san.run_sync("dev", &k2, "contacts").await });
    while runs.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        h.san.run_sync("dev", &key, "contacts").await,
        Err(Error::Conflict(_))
    ));
    let status = h
        .san
        .sync_status("dev", "crm", Some(&key.connection_id), &["contacts".into()])
        .await
        .unwrap();
    assert_eq!(status[0].status, SyncStatus::Running);
    gate.notify_one();
    assert!(first.await.unwrap().unwrap().success);
}

#[tokio::test]
async fn the_scheduler_runs_due_auto_start_syncs() {
    let runs = Arc::new(AtomicUsize::new(0));
    let h = ready_with(registry(&runs, None)).await;
    let a = connect_crm(&h, "at-a", None, None).await.key();
    let b = connect_crm(&h, "at-b", None, None).await.key();
    stub_crm(&h);
    let reports = h.san.run_due_syncs().await.unwrap();
    assert_eq!(
        reports.len(),
        2,
        "contacts for a and b; broken is not auto start"
    );
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert!(
        h.san.run_due_syncs().await.unwrap().is_empty(),
        "not due yet"
    );
    h.clock.advance(chrono::Duration::minutes(61));
    h.san
        .set_sync_paused(
            "dev",
            "crm",
            Some(&b.connection_id),
            &["contacts".into()],
            true,
        )
        .await
        .unwrap();
    let reports = h.san.run_due_syncs().await.unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].connection, a);
    let status = h
        .san
        .sync_status("dev", "crm", Some(&b.connection_id), &["contacts".into()])
        .await
        .unwrap();
    assert_eq!(status[0].status, SyncStatus::Paused);
    h.san
        .set_sync_paused("dev", "crm", None, &["contacts".into()], false)
        .await
        .unwrap();
    h.clock.advance(chrono::Duration::minutes(61));
    assert_eq!(h.san.run_due_syncs().await.unwrap().len(), 2);
    assert!(matches!(
        h.san
            .set_sync_paused("dev", "crm", None, &["nope".into()], true)
            .await,
        Err(Error::UnknownFunction(_))
    ));
}

#[tokio::test]
async fn trigger_runs_in_the_background() {
    let runs = Arc::new(AtomicUsize::new(0));
    let h = ready_with(registry(&runs, None)).await;
    let key = connect_crm(&h, "at", None, None).await.key();
    connect_crm(&h, "at-2", None, None).await;
    stub_crm(&h);
    assert_eq!(
        h.san
            .trigger_syncs("dev", "crm", Some(&key.connection_id), &["contacts".into()])
            .await
            .unwrap(),
        1
    );
    h.san.flush().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        h.san
            .trigger_syncs("dev", "crm", None, &["contacts".into()])
            .await
            .unwrap(),
        2
    );
    h.san.flush().await;
    assert_eq!(runs.load(Ordering::SeqCst), 3);
    assert!(matches!(
        h.san
            .trigger_syncs("dev", "crm", None, &["nope".into()])
            .await,
        Err(Error::UnknownFunction(_))
    ));
    assert!(matches!(
        h.san.trigger_syncs("dev", "nope", None, &[]).await,
        Err(Error::UnknownIntegration(_))
    ));
}

#[tokio::test]
async fn actions_run_and_return_output() {
    let runs = Arc::new(AtomicUsize::new(0));
    let h = ready_with(registry(&runs, None)).await;
    let key = connect_crm(&h, "at", None, None).await.key();
    stub_crm(&h);
    h.http.reply(
        "POST",
        "https://api.crm.example.com/v3/notes",
        HttpResponse::json(201, &json!({"id": "n1"})),
    );
    let out = h
        .san
        .run_action("dev", &key, "create-note", json!({"text": "hi"}))
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "n1"}));
    let sent = &h.http.requests_to("https://api.crm.example.com/v3/notes")[0];
    assert_eq!(sent.body, br#"{"text":"hi"}"#);
    assert!(matches!(
        h.san
            .run_action("dev", &key, "create-note", json!({}))
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        h.san.run_action("dev", &key, "nope", json!({})).await,
        Err(Error::UnknownFunction(_))
    ));
    assert!(matches!(
        h.san
            .run_action(
                "dev",
                &ConnectionKey::new("crm", "x"),
                "create-note",
                json!({"text": "a"})
            )
            .await,
        Err(Error::UnknownConnection(_))
    ));
}

/// A sync that waits at a gate.
struct Slow {
    started: Arc<AtomicUsize>,
    gate: Arc<Notify>,
}

#[async_trait]
impl SyncFunction for Slow {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "slow".into(),
            integration: "crm".into(),
            models: vec!["S".into()],
            frequency: Duration::from_secs(3_600),
            auto_start: true,
        }
    }

    async fn run(&self, _ctx: &SyncContext) -> Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.gate.notified().await;
        Ok(())
    }
}

async fn slow_harness() -> (Harness, Arc<AtomicUsize>, Arc<Notify>) {
    let started = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let h = ready_with(Registry::new().sync(Slow {
        started: started.clone(),
        gate: gate.clone(),
    }))
    .await;
    (h, started, gate)
}

async fn until_started(started: &AtomicUsize) {
    while started.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_slow_sync_does_not_stall_the_background_loops() {
    let (h, started, gate) = slow_harness().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    h.http.reply(
        "POST",
        "https://auth.crm.example.com/token",
        token_reply("at-bg", Some("rt-bg"), Some(3_600)),
    );
    let bg = h.san.start_background(BackgroundSettings {
        tick: Duration::from_secs(1),
        ..BackgroundSettings::default()
    });
    until_started(&started).await;
    // The token expires soon now. A later tick must refresh it.
    h.clock.advance(chrono::Duration::minutes(50));
    tokio::time::sleep(Duration::from_secs(3)).await;
    let got = h
        .san
        .get_connection(
            "dev",
            &conn.key(),
            GetOptions {
                refresh_if_needed: false,
                force_refresh: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-bg"));
    assert_eq!(started.load(Ordering::SeqCst), 1, "one run at a time");
    gate.notify_waiters();
    bg.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stop_and_flush_do_not_wait_for_ever() {
    let (h, started, gate) = slow_harness().await;
    connect_crm(&h, "at-1", None, None).await;
    let bg = h.san.start_background(BackgroundSettings {
        tick: Duration::from_secs(1),
        ..BackgroundSettings::default()
    });
    until_started(&started).await;
    let stopped = tokio::time::timeout(Duration::from_secs(60), bg.stop()).await;
    assert!(stopped.is_ok(), "stop must not wait for a slow sync");
    assert!(
        !h.san.flush_within(Duration::from_secs(1)).await,
        "the slow sync still runs"
    );
    gate.notify_waiters();
    assert!(h.san.flush_within(Duration::from_secs(5)).await);
}

/// A sync that notes `last_sync_date` and takes ten minutes.
struct Timed {
    clock: Arc<OnceLock<Arc<ManualClock>>>,
    seen: Arc<Mutex<Vec<Option<DateTime<Utc>>>>>,
}

#[async_trait]
impl SyncFunction for Timed {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "timed".into(),
            integration: "crm".into(),
            models: vec!["T".into()],
            frequency: Duration::from_secs(60),
            auto_start: false,
        }
    }

    async fn run(&self, ctx: &SyncContext) -> Result<()> {
        self.seen.lock().unwrap().push(ctx.last_sync_date());
        self.clock
            .get()
            .unwrap()
            .advance(chrono::Duration::minutes(10));
        Ok(())
    }
}

#[tokio::test]
async fn last_sync_date_is_the_start_of_the_last_successful_run() {
    let clock = Arc::new(OnceLock::new());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let h = ready_with(Registry::new().sync(Timed {
        clock: clock.clone(),
        seen: seen.clone(),
    }))
    .await;
    clock.set(h.clock.clone()).unwrap();
    let key = connect_crm(&h, "at", None, None).await.key();
    let first_start = now(&h);
    h.san.run_sync("dev", &key, "timed").await.unwrap();
    h.san.run_sync("dev", &key, "timed").await.unwrap();
    // Changes made during the first run are after its start.
    assert_eq!(*seen.lock().unwrap(), vec![None, Some(first_start)]);
}

#[tokio::test]
async fn two_processes_do_not_run_one_sync_at_once() {
    let runs = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let h = ready_with(registry(&runs, Some(gate.clone()))).await;
    let key = connect_crm(&h, "at", None, None).await.key();
    stub_crm(&h);
    let other = builder(
        h.store.clone(),
        Arc::new(h.http.clone()) as Arc<dyn HttpTransport>,
        h.clock.clone(),
        registry(&runs, None),
    )
    .entropy(Arc::new(OsEntropy))
    .build()
    .unwrap();
    let san = h.san.clone();
    let k2 = key.clone();
    let first = tokio::spawn(async move { san.run_sync("dev", &k2, "contacts").await });
    while runs.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let second = other.run_sync("dev", &key, "contacts").await;
    assert!(matches!(second, Err(Error::Conflict(_))), "{second:?}");
    gate.notify_one();
    assert!(first.await.unwrap().unwrap().success);
    // The lease is free after the run.
    assert!(
        other
            .run_sync("dev", &key, "contacts")
            .await
            .unwrap()
            .success
    );
}

/// A sync that deletes its own connection during the run.
struct SelfDelete {
    san: Arc<OnceLock<SanMateo>>,
}

#[async_trait]
impl SyncFunction for SelfDelete {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "self-delete".into(),
            integration: "crm".into(),
            models: vec!["X".into()],
            frequency: Duration::from_secs(60),
            auto_start: false,
        }
    }

    async fn run(&self, ctx: &SyncContext) -> Result<()> {
        ctx.batch_save("X", vec![json!({"id": "1"})]).await?;
        let san = self.san.get().unwrap();
        san.delete_connection(ctx.environment(), ctx.connection())
            .await?;
        ctx.batch_save("X", vec![json!({"id": "2"})]).await?;
        Ok(())
    }
}

#[tokio::test]
async fn deleting_a_connection_during_a_sync_leaves_no_rows() {
    let handle = Arc::new(OnceLock::new());
    let h = ready_with(Registry::new().sync(SelfDelete {
        san: handle.clone(),
    }))
    .await;
    handle.set(h.san.clone()).unwrap();
    let conn = connect_crm(&h, "at", None, None).await;
    let got = h.san.run_sync("dev", &conn.key(), "self-delete").await;
    assert!(matches!(got, Err(Error::UnknownConnection(_))), "{got:?}");
    assert_eq!(
        h.store
            .get_sync_state(conn.id, "self-delete")
            .await
            .unwrap(),
        None,
        "no sync state for a deleted connection"
    );
    let q = RecordQuery {
        model: "X".into(),
        limit: 10,
        ..RecordQuery::default()
    };
    assert!(h.store.list_records(conn.id, &q).await.unwrap().is_empty());
}
