//! `PgStore` against a real Postgres (AC18).
//!
//! The tests need `SAN_MATEO_TEST_DATABASE_URL`. With no URL, they print a
//! note and pass. CI sets the URL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use san_mateo::clock::ManualClock;
use san_mateo::records::RecordQuery;
use san_mateo::store::{ConnectionRow, RecordWrite, Store, conformance};
use san_mateo::transport::HttpResponse;
use san_mateo::{
    Catalog, ConnectOptions, ConnectSessionInput, EnvironmentConfig, GetOptions, KeyRing,
    MemoryTransport, NewIntegration, SanMateo, Secret,
};
use san_mateo_postgres::PgStore;
use serde_json::json;
use uuid::Uuid;

/// Returns a migrated store, or `None` when no database is set.
///
/// Each test makes its own pool: a pool connection lives on the runtime of
/// the test that opened it.
async fn store() -> Option<PgStore> {
    let Ok(url) = std::env::var("SAN_MATEO_TEST_DATABASE_URL") else {
        eprintln!("SAN_MATEO_TEST_DATABASE_URL is not set: Postgres tests do nothing");
        return None;
    };
    let s = PgStore::connect(&url, 16).unwrap();
    s.migrate().await.unwrap();
    Some(s)
}

#[tokio::test]
async fn pg_store_passes_the_conformance_suite() {
    let Some(s) = store().await else { return };
    conformance::run_all(|| {
        let s = s.clone();
        async move { Arc::new(s) as Arc<dyn Store> }
    })
    .await;
}

#[tokio::test]
async fn migrations_are_idempotent_and_concurrent_safe() {
    let Some(s) = store().await else { return };
    let (a, b) = tokio::join!(s.migrate(), s.migrate());
    assert!(a.unwrap().is_empty());
    assert!(b.unwrap().is_empty());
}

fn row(env: &str, cid: &str) -> ConnectionRow {
    ConnectionRow {
        id: Uuid::new_v4(),
        environment: env.to_owned(),
        provider_config_key: "gh".to_owned(),
        connection_id: cid.to_owned(),
        provider: "github".to_owned(),
        credentials: "sm1.k.x".to_owned(),
        credentials_expires_at: None,
        connection_config: json!({}),
        metadata: json!({}),
        tags: BTreeMap::new(),
        end_user: None,
        webhook_url_override: None,
        created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        updated_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        last_fetched_at: None,
        last_refresh_success: None,
        last_refresh_failure: None,
        refresh_attempts: 0,
        refresh_exhausted: false,
        version: 0,
    }
}

#[tokio::test]
async fn only_one_concurrent_writer_wins_a_version() {
    let Some(s) = store().await else { return };
    let env = format!("env-{}", Uuid::new_v4().simple());
    let stored = s.upsert_connection(row(&env, "c1")).await.unwrap().row;
    let writes = (0..8).map(|i| {
        let s = s.clone();
        let mut w = stored.clone();
        w.credentials = format!("sm1.k.w{i}");
        async move { s.update_connection(&w).await.unwrap() }
    });
    let results = futures::future::join_all(writes).await;
    assert_eq!(results.iter().filter(|r| r.is_some()).count(), 1);
    let now = s.get_connection(&env, "gh", "c1").await.unwrap().unwrap();
    assert_eq!(now.version, stored.version + 1);
}

#[tokio::test]
async fn only_one_concurrent_owner_gets_a_lease() {
    let Some(s) = store().await else { return };
    let name = format!("lease-{}", Uuid::new_v4());
    // A far-future time: the purge in the parallel conformance test does not
    // remove this lease.
    let now = Utc.timestamp_opt(4_100_000_000, 0).unwrap();
    let tries = (0..8).map(|i| {
        let s = s.clone();
        let name = name.clone();
        async move {
            s.try_lease(&name, &format!("owner-{i}"), now, 10_000)
                .await
                .unwrap()
        }
    });
    let got = futures::future::join_all(tries).await;
    assert_eq!(got.iter().filter(|g| **g).count(), 1);
}

#[tokio::test]
async fn concurrent_upserts_of_one_key_make_one_row() {
    let Some(s) = store().await else { return };
    let env = format!("env-{}", Uuid::new_v4().simple());
    let ups = (0..8).map(|_| {
        let s = s.clone();
        let r = row(&env, "same");
        async move { s.upsert_connection(r).await.unwrap() }
    });
    let results = futures::future::join_all(ups).await;
    assert_eq!(results.iter().filter(|u| u.created).count(), 1);
    let ids: std::collections::HashSet<Uuid> = results.iter().map(|u| u.row.id).collect();
    assert_eq!(ids.len(), 1);
}

/// Adds a connection in a new environment and returns its id.
async fn new_connection(s: &PgStore) -> Uuid {
    let env = format!("env-{}", Uuid::new_v4().simple());
    s.upsert_connection(row(&env, "c1")).await.unwrap().row.id
}

fn writes(conn: Uuid, prefix: &str, n: usize, hash: &str) -> Vec<RecordWrite> {
    (0..n)
        .map(|i| {
            let ext = format!("{prefix}{i}");
            RecordWrite {
                id: san_mateo::records::record_id(conn, "m", &ext),
                external_id: ext.clone(),
                data: format!("sm1.k.{ext}.{hash}"),
                data_hash: hash.to_owned(),
            }
        })
        .collect()
}

#[tokio::test]
async fn concurrent_record_batches_count_each_record_once() {
    let Some(s) = store().await else { return };
    let conn = new_connection(&s).await;
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let batches = (0..4).map(|_| {
        let s = s.clone();
        let rows: Vec<RecordWrite> = (0..20)
            .map(|i| RecordWrite {
                id: san_mateo::records::record_id(conn, "m", &i.to_string()),
                external_id: i.to_string(),
                data: format!("sm1.k.{i}"),
                data_hash: "h".to_owned(),
            })
            .collect();
        async move { s.upsert_records(conn, "m", rows, now).await.unwrap() }
    });
    let results = futures::future::join_all(batches).await;
    let added: usize = results
        .iter()
        .map(|r| {
            r.iter()
                .filter(|(_, c)| *c == san_mateo::records::Change::Added)
                .count()
        })
        .sum();
    assert_eq!(added, 20);
    let q = RecordQuery {
        model: "m".into(),
        limit: 100,
        ..RecordQuery::default()
    };
    assert_eq!(s.list_records(conn, &q).await.unwrap().len(), 20);
}

#[tokio::test]
async fn the_service_runs_on_postgres() {
    let Some(s) = store().await else { return };
    let http = MemoryTransport::new();
    let catalog = Catalog::from_yaml_str(
        "crm:\n  display_name: CRM\n  auth_mode: OAUTH2\n  authorization_url: https://auth.crm.example.com/a\n  token_url: https://auth.crm.example.com/t\n  proxy:\n    base_url: https://api.crm.example.com\n",
    )
    .unwrap();
    let env = format!("env-{}", Uuid::new_v4().simple());
    let san = SanMateo::builder()
        .server_url("https://sm.example.com")
        .environment(EnvironmentConfig::new(&env, format!("sk_{env}")))
        .catalog(catalog)
        .store(Arc::new(s.clone()))
        .key_ring(KeyRing::new("k1", [3; 32]).unwrap())
        .transport(Arc::new(http.clone()))
        .clock(Arc::new(ManualClock::new(
            Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        )))
        .build()
        .unwrap();
    san.create_integration(
        &env,
        NewIntegration {
            unique_key: "crm".into(),
            provider: "crm".into(),
            client_id: Some("id".into()),
            client_secret: Some(Secret::new("secret")),
            ..NewIntegration::default()
        },
    )
    .await
    .unwrap();
    let token = san
        .create_connect_session(&env, ConnectSessionInput::default())
        .await
        .unwrap()
        .token;
    let url = san
        .oauth_start(token.expose(), "crm", ConnectOptions::default())
        .await
        .unwrap();
    let state = url::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    http.reply(
        "POST",
        "https://auth.crm.example.com/t",
        HttpResponse::json(
            200,
            &json!({"access_token": "at", "refresh_token": "rt", "expires_in": 60}),
        ),
    );
    let cb = BTreeMap::from([
        ("code".to_owned(), "c".to_owned()),
        ("state".to_owned(), state),
    ]);
    let conn = san.oauth_callback(&cb).await.unwrap();
    http.reply(
        "POST",
        "https://auth.crm.example.com/t",
        HttpResponse::json(200, &json!({"access_token": "at2", "expires_in": 3600})),
    );
    let fresh = san
        .get_connection(&env, &conn.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(fresh.credentials.bearer_token(), Some("at2"));
    assert_eq!(fresh.credentials.refresh_token(), Some("rt"));
    let out = san
        .save_records(
            &env,
            &conn.key(),
            "M",
            vec![json!({"id": 1}), json!({"id": 2})],
        )
        .await
        .unwrap();
    assert_eq!(out.added.len(), 2);
    let page = san
        .list_records(
            &env,
            &conn.key(),
            san_mateo::RecordsRequest {
                model: "M".into(),
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(page.next_cursor.is_some());
    san.delete_connection(&env, &conn.key()).await.unwrap();
}

#[tokio::test]
async fn concurrent_batches_get_distinct_times() {
    let Some(s) = store().await else { return };
    let conn = new_connection(&s).await;
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let batches = (0..8).map(|b| {
        let s = s.clone();
        async move {
            s.upsert_records(conn, "m", writes(conn, &format!("b{b}-"), 5, "h"), now)
                .await
                .unwrap()
        }
    });
    futures::future::join_all(batches).await;
    let q = RecordQuery {
        model: "m".into(),
        limit: 100,
        ..RecordQuery::default()
    };
    let rows = s.list_records(conn, &q).await.unwrap();
    assert_eq!(rows.len(), 40);
    // One time for each batch, and no time for two batches: a reader with a
    // cursor in one batch cannot miss a batch that commits later.
    let mut times: BTreeMap<String, std::collections::BTreeSet<i64>> = BTreeMap::new();
    for r in &rows {
        let batch = r.external_id.split('-').next().unwrap().to_owned();
        times
            .entry(batch)
            .or_default()
            .insert(r.updated_at.timestamp_micros());
    }
    assert!(times.values().all(|t| t.len() == 1), "{times:?}");
    let all: std::collections::BTreeSet<i64> = times.values().flatten().copied().collect();
    assert_eq!(all.len(), 8, "{times:?}");
}

#[tokio::test]
async fn opposite_order_batches_do_not_deadlock() {
    let Some(s) = store().await else { return };
    let conn = new_connection(&s).await;
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    s.upsert_records(conn, "m", writes(conn, "r", 40, "h0"), now)
        .await
        .unwrap();
    for round in 0..5 {
        let forward = writes(conn, "r", 40, &format!("f{round}"));
        let mut backward = writes(conn, "r", 40, &format!("b{round}"));
        backward.reverse();
        let (a, b) = tokio::join!(
            s.upsert_records(conn, "m", forward, now),
            s.upsert_records(conn, "m", backward, now)
        );
        a.unwrap();
        b.unwrap();
    }
}

#[tokio::test]
async fn the_schema_cascades_connection_deletes() {
    use diesel_async::RunQueryDsl as _;
    let Some(s) = store().await else { return };
    let conn = new_connection(&s).await;
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    s.upsert_records(conn, "m", writes(conn, "r", 2, "h"), now)
        .await
        .unwrap();
    s.put_sync_state(san_mateo::store::SyncStateRow {
        connection: conn,
        sync_name: "s".into(),
        status: san_mateo::store::SyncStatus::Success,
        frequency_ms: 60_000,
        paused: false,
        last_started_at: None,
        last_finished_at: None,
        last_success_at: None,
        last_error: None,
        last_counts: san_mateo::store::SyncCounts::default(),
    })
    .await
    .unwrap();
    // A plain SQL delete, with no help from `PgStore`.
    let mut c = s.pool().get().await.unwrap();
    diesel::sql_query("DELETE FROM sm_connections WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(conn)
        .execute(&mut c)
        .await
        .unwrap();
    let q = RecordQuery {
        model: "m".into(),
        limit: 10,
        ..RecordQuery::default()
    };
    assert!(s.list_records(conn, &q).await.unwrap().is_empty());
    assert_eq!(s.get_sync_state(conn, "s").await.unwrap(), None);
}
