//! A test suite that each [`Store`] must pass.
//!
//! Each test uses a new environment id and new UUIDs, so tests can share
//! one database. Times are whole seconds, so a store can keep microseconds.

#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use super::{
    ConnectSessionRow, ConnectionFilter, ConnectionRow, IntegrationRow, OAuthSessionRow,
    RecordWrite, Store, SyncCounts, SyncStateRow, SyncStatus,
};
use crate::error::Error;
use crate::model::{ConnectionKey, EndUser, IntegrationDefaults, Organization};
use crate::records::{Change, Cursor, LastAction, RecordQuery};

/// Runs every test. `make` gives a store for each test.
pub async fn run_all<F, Fut>(make: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = Arc<dyn Store>>,
{
    integrations_crud(make().await).await;
    integration_conflict(make().await).await;
    connection_upsert_and_get(make().await).await;
    connection_compare_and_set(make().await).await;
    connection_list_filters(make().await).await;
    connection_delete_cascades(make().await).await;
    integration_delete_cascades(make().await).await;
    expiring_connections(make().await).await;
    leases(make().await).await;
    connect_sessions(make().await).await;
    oauth_sessions_are_single_use(make().await).await;
    purge_expired(make().await).await;
    record_upsert_classes(make().await).await;
    record_deletes(make().await).await;
    record_listing(make().await).await;
    sync_states(make().await).await;
    environments_are_isolated(make().await).await;
}

fn env() -> String {
    format!("env-{}", Uuid::new_v4().simple())
}

fn t(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
}

fn integration(env: &str, key: &str) -> IntegrationRow {
    IntegrationRow {
        environment: env.to_owned(),
        unique_key: key.to_owned(),
        provider: "github".to_owned(),
        display_name: Some("GitHub".to_owned()),
        client_id: Some("cid".to_owned()),
        client_secret: Some("sm1.k.sealed".to_owned()),
        scopes: vec!["repo".to_owned(), "user".to_owned()],
        custom: BTreeMap::from([("a".to_owned(), "b".to_owned())]),
        created_at: t(0),
        updated_at: t(0),
    }
}

fn connection(env: &str, key: &str, id: &str) -> ConnectionRow {
    ConnectionRow {
        id: Uuid::new_v4(),
        environment: env.to_owned(),
        provider_config_key: key.to_owned(),
        connection_id: id.to_owned(),
        provider: "github".to_owned(),
        credentials: "sm1.k.creds".to_owned(),
        credentials_expires_at: Some(t(3_600)),
        connection_config: json!({"subdomain": "acme"}),
        metadata: json!({}),
        tags: BTreeMap::from([("team".to_owned(), "a".to_owned())]),
        end_user: Some(EndUser {
            id: "u1".to_owned(),
            email: Some("u1@example.com".to_owned()),
            display_name: None,
            tags: BTreeMap::new(),
            organization: Some(Organization {
                id: "o1".to_owned(),
                display_name: None,
            }),
        }),
        webhook_url_override: None,
        created_at: t(0),
        updated_at: t(0),
        last_fetched_at: None,
        last_refresh_success: None,
        last_refresh_failure: None,
        refresh_attempts: 0,
        refresh_exhausted: false,
        version: 0,
    }
}

pub async fn integrations_crud(s: Arc<dyn Store>) {
    let e = env();
    assert_eq!(s.get_integration(&e, "gh").await.unwrap(), None);
    s.insert_integration(integration(&e, "gh")).await.unwrap();
    s.insert_integration(integration(&e, "a-first")).await.unwrap();
    assert_eq!(s.get_integration(&e, "gh").await.unwrap(), Some(integration(&e, "gh")));
    let keys: Vec<String> = s.list_integrations(&e).await.unwrap().into_iter().map(|i| i.unique_key).collect();
    assert_eq!(keys, vec!["a-first", "gh"]);
    let mut changed = integration(&e, "gh");
    changed.scopes = vec!["read".to_owned()];
    changed.client_secret = None;
    changed.updated_at = t(5);
    assert!(s.update_integration(changed.clone()).await.unwrap());
    assert_eq!(s.get_integration(&e, "gh").await.unwrap(), Some(changed));
    assert!(!s.update_integration(integration(&e, "nope")).await.unwrap());
    assert!(s.delete_integration(&e, "gh").await.unwrap());
    assert!(!s.delete_integration(&e, "gh").await.unwrap());
    assert_eq!(s.get_integration(&e, "gh").await.unwrap(), None);
}

pub async fn integration_conflict(s: Arc<dyn Store>) {
    let e = env();
    s.insert_integration(integration(&e, "gh")).await.unwrap();
    let err = s.insert_integration(integration(&e, "gh")).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
}

pub async fn connection_upsert_and_get(s: Arc<dyn Store>) {
    let e = env();
    let first = connection(&e, "gh", "c1");
    let up = s.upsert_connection(first.clone()).await.unwrap();
    assert!(up.created);
    assert_eq!(up.row, first);
    assert_eq!(s.get_connection(&e, "gh", "c1").await.unwrap(), Some(first.clone()));
    assert_eq!(s.get_connection(&e, "gh", "c2").await.unwrap(), None);
    assert_eq!(s.get_connection(&e, "other", "c1").await.unwrap(), None);

    let mut again = connection(&e, "gh", "c1");
    again.credentials = "sm1.k.new".to_owned();
    again.created_at = t(100);
    again.updated_at = t(100);
    let up = s.upsert_connection(again).await.unwrap();
    assert!(!up.created);
    assert_eq!(up.row.id, first.id);
    assert_eq!(up.row.created_at, first.created_at);
    assert_eq!(up.row.updated_at, t(100));
    assert_eq!(up.row.credentials, "sm1.k.new");
    assert_eq!(up.row.version, 1);
    assert_eq!(s.get_connection(&e, "gh", "c1").await.unwrap(), Some(up.row));
}

pub async fn connection_compare_and_set(s: Arc<dyn Store>) {
    let e = env();
    let row = s.upsert_connection(connection(&e, "gh", "c1")).await.unwrap().row;
    let mut w1 = row.clone();
    w1.credentials = "sm1.k.w1".to_owned();
    w1.refresh_attempts = 2;
    w1.last_refresh_failure = Some(t(9));
    let stored = s.update_connection(&w1).await.unwrap().unwrap();
    assert_eq!(stored.version, row.version + 1);
    assert_eq!(stored.credentials, "sm1.k.w1");
    // A second writer with the old version loses.
    let mut w2 = row.clone();
    w2.credentials = "sm1.k.w2".to_owned();
    assert_eq!(s.update_connection(&w2).await.unwrap(), None);
    let now = s.get_connection(&e, "gh", "c1").await.unwrap().unwrap();
    assert_eq!(now.credentials, "sm1.k.w1");
    assert_eq!(now.refresh_attempts, 2);
    assert_eq!(now.last_refresh_failure, Some(t(9)));
    // A write to a missing row gives `None`.
    let mut gone = connection(&e, "gh", "zz");
    gone.version = 0;
    assert_eq!(s.update_connection(&gone).await.unwrap(), None);
}

pub async fn connection_list_filters(s: Arc<dyn Store>) {
    let e = env();
    let mut a = connection(&e, "gh", "c1");
    a.created_at = t(1);
    let mut b = connection(&e, "gh", "c2");
    b.created_at = t(2);
    b.end_user = None;
    let mut c = connection(&e, "slack", "c3");
    c.created_at = t(3);
    for r in [c.clone(), a.clone(), b.clone()] {
        s.upsert_connection(r).await.unwrap();
    }
    let ids = |rows: Vec<ConnectionRow>| rows.into_iter().map(|r| r.connection_id).collect::<Vec<_>>();
    assert_eq!(ids(s.list_connections(&e, &ConnectionFilter::default()).await.unwrap()), vec!["c1", "c2", "c3"]);
    let by_key = ConnectionFilter { provider_config_key: Some("gh".to_owned()), ..ConnectionFilter::default() };
    assert_eq!(ids(s.list_connections(&e, &by_key).await.unwrap()), vec!["c1", "c2"]);
    let by_user = ConnectionFilter { end_user_id: Some("u1".to_owned()), ..ConnectionFilter::default() };
    assert_eq!(ids(s.list_connections(&e, &by_user).await.unwrap()), vec!["c1", "c3"]);
    let by_id = ConnectionFilter { connection_id: Some("c2".to_owned()), ..ConnectionFilter::default() };
    assert_eq!(ids(s.list_connections(&e, &by_id).await.unwrap()), vec!["c2"]);
}

fn write(conn: Uuid, external: &str, hash: &str) -> RecordWrite {
    RecordWrite {
        id: crate::records::record_id(conn, "m", external),
        external_id: external.to_owned(),
        data: format!("sm1.k.{external}.{hash}"),
        data_hash: hash.to_owned(),
    }
}

fn sync_state(conn: Uuid, name: &str) -> SyncStateRow {
    SyncStateRow {
        connection: conn,
        sync_name: name.to_owned(),
        status: SyncStatus::Success,
        frequency_ms: 60_000,
        paused: false,
        last_started_at: Some(t(1)),
        last_finished_at: Some(t(2)),
        last_success_at: Some(t(2)),
        last_error: None,
        last_counts: SyncCounts { added: 1, updated: 2, deleted: 3 },
    }
}

pub async fn connection_delete_cascades(s: Arc<dyn Store>) {
    let e = env();
    let row = s.upsert_connection(connection(&e, "gh", "c1")).await.unwrap().row;
    s.upsert_records(row.id, "m", vec![write(row.id, "1", "h")], t(1)).await.unwrap();
    s.put_sync_state(sync_state(row.id, "s")).await.unwrap();
    let deleted = s.delete_connection(&e, "gh", "c1").await.unwrap().unwrap();
    assert_eq!(deleted.id, row.id);
    assert_eq!(s.delete_connection(&e, "gh", "c1").await.unwrap(), None);
    assert_eq!(s.get_connection(&e, "gh", "c1").await.unwrap(), None);
    let q = RecordQuery { model: "m".to_owned(), limit: 10, ..RecordQuery::default() };
    assert!(s.list_records(row.id, &q).await.unwrap().is_empty());
    assert_eq!(s.get_sync_state(row.id, "s").await.unwrap(), None);
}

pub async fn integration_delete_cascades(s: Arc<dyn Store>) {
    let e = env();
    s.insert_integration(integration(&e, "gh")).await.unwrap();
    let row = s.upsert_connection(connection(&e, "gh", "c1")).await.unwrap().row;
    s.upsert_records(row.id, "m", vec![write(row.id, "1", "h")], t(1)).await.unwrap();
    s.upsert_connection(connection(&e, "other", "c1")).await.unwrap();
    assert!(s.delete_integration(&e, "gh").await.unwrap());
    assert_eq!(s.get_connection(&e, "gh", "c1").await.unwrap(), None);
    assert!(s.get_connection(&e, "other", "c1").await.unwrap().is_some());
    let q = RecordQuery { model: "m".to_owned(), limit: 10, ..RecordQuery::default() };
    assert!(s.list_records(row.id, &q).await.unwrap().is_empty());
}

pub async fn expiring_connections(s: Arc<dyn Store>) {
    let e = env();
    // Use far-future instants, so rows of other tests do not interfere.
    let base = Utc.timestamp_opt(4_000_000_000, 0).unwrap();
    let mut soon = connection(&e, "gh", "soon");
    soon.credentials_expires_at = Some(base + Duration::seconds(10));
    let mut sooner = connection(&e, "gh", "sooner");
    sooner.credentials_expires_at = Some(base + Duration::seconds(5));
    let mut late = connection(&e, "gh", "late");
    late.credentials_expires_at = Some(base + Duration::seconds(10_000));
    let mut never = connection(&e, "gh", "never");
    never.credentials_expires_at = None;
    let mut exhausted = connection(&e, "gh", "exhausted");
    exhausted.credentials_expires_at = Some(base + Duration::seconds(1));
    exhausted.refresh_exhausted = true;
    for r in [soon, sooner, late, never, exhausted] {
        s.upsert_connection(r).await.unwrap();
    }
    let got: Vec<String> = s
        .connections_expiring_before(base + Duration::seconds(60), 1_000)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.environment == e)
        .map(|r| r.connection_id)
        .collect();
    assert_eq!(got, vec!["sooner", "soon"]);
    let limited = s.connections_expiring_before(base + Duration::seconds(60), 1).await.unwrap();
    assert_eq!(limited.len(), 1);
}

pub async fn leases(s: Arc<dyn Store>) {
    let name = format!("lease-{}", Uuid::new_v4());
    assert!(s.try_lease(&name, "a", t(0), 10_000).await.unwrap());
    assert!(!s.try_lease(&name, "b", t(5), 10_000).await.unwrap());
    // The owner can take it again.
    assert!(s.try_lease(&name, "a", t(5), 10_000).await.unwrap());
    // Only the owner can give it back.
    s.release_lease(&name, "b").await.unwrap();
    assert!(!s.try_lease(&name, "b", t(6), 10_000).await.unwrap());
    s.release_lease(&name, "a").await.unwrap();
    assert!(s.try_lease(&name, "b", t(6), 10_000).await.unwrap());
    // An expired lease is free.
    assert!(s.try_lease(&name, "c", t(17), 10_000).await.unwrap());
}

fn connect_session(env: &str, hash: &str, expires: DateTime<Utc>) -> ConnectSessionRow {
    ConnectSessionRow {
        id: Uuid::new_v4(),
        environment: env.to_owned(),
        token_hash: hash.to_owned(),
        end_user: Some(EndUser {
            id: "u1".to_owned(),
            ..EndUser::default()
        }),
        organization: None,
        allowed_integrations: vec!["gh".to_owned()],
        integrations_config_defaults: BTreeMap::from([(
            "gh".to_owned(),
            IntegrationDefaults {
                user_scopes: Some("repo".to_owned()),
                ..IntegrationDefaults::default()
            },
        )]),
        tags: BTreeMap::new(),
        webhook_url_override: Some("https://hooks.example.com/x".to_owned()),
        reconnect: Some(ConnectionKey::new("gh", "c1")),
        created_at: t(0),
        expires_at: expires,
    }
}

pub async fn connect_sessions(s: Arc<dyn Store>) {
    let e = env();
    let hash = format!("h-{}", Uuid::new_v4());
    let row = connect_session(&e, &hash, t(1_800));
    s.insert_connect_session(row.clone()).await.unwrap();
    assert_eq!(s.get_connect_session(&hash).await.unwrap(), Some(row.clone()));
    assert_eq!(s.get_connect_session("nope").await.unwrap(), None);
    assert_eq!(s.get_connect_session_by_id(row.id).await.unwrap(), Some(row.clone()));
    assert_eq!(s.get_connect_session_by_id(Uuid::new_v4()).await.unwrap(), None);
    assert!(s.delete_connect_session(row.id).await.unwrap());
    assert!(!s.delete_connect_session(row.id).await.unwrap());
    assert_eq!(s.get_connect_session(&hash).await.unwrap(), None);
}

fn oauth_session(env: &str, hash: &str, expires: DateTime<Utc>) -> OAuthSessionRow {
    OAuthSessionRow {
        state_hash: hash.to_owned(),
        environment: env.to_owned(),
        provider_config_key: "gh".to_owned(),
        connection_id: "c1".to_owned(),
        connect_session_id: Some(Uuid::new_v4()),
        code_verifier: Some("sm1.k.verifier".to_owned()),
        connection_config: json!({"a": 1}),
        redirect_uri: "https://sm.example.com/oauth/callback".to_owned(),
        created_at: t(0),
        expires_at: expires,
    }
}

pub async fn oauth_sessions_are_single_use(s: Arc<dyn Store>) {
    let e = env();
    let hash = format!("s-{}", Uuid::new_v4());
    let row = oauth_session(&e, &hash, t(900));
    s.insert_oauth_session(row.clone()).await.unwrap();
    assert_eq!(s.take_oauth_session(&hash).await.unwrap(), Some(row));
    assert_eq!(s.take_oauth_session(&hash).await.unwrap(), None);
}

pub async fn purge_expired(s: Arc<dyn Store>) {
    let e = env();
    let old_c = format!("oc-{}", Uuid::new_v4());
    let new_c = format!("nc-{}", Uuid::new_v4());
    let old_o = format!("oo-{}", Uuid::new_v4());
    let new_o = format!("no-{}", Uuid::new_v4());
    s.insert_connect_session(connect_session(&e, &old_c, t(10))).await.unwrap();
    s.insert_connect_session(connect_session(&e, &new_c, t(1_000_000))).await.unwrap();
    s.insert_oauth_session(oauth_session(&e, &old_o, t(10))).await.unwrap();
    s.insert_oauth_session(oauth_session(&e, &new_o, t(1_000_000))).await.unwrap();
    let n = s.purge_expired(t(500)).await.unwrap();
    assert!(n >= 2, "{n}");
    assert_eq!(s.get_connect_session(&old_c).await.unwrap(), None);
    assert!(s.get_connect_session(&new_c).await.unwrap().is_some());
    assert_eq!(s.take_oauth_session(&old_o).await.unwrap(), None);
    assert!(s.take_oauth_session(&new_o).await.unwrap().is_some());
}

pub async fn record_upsert_classes(s: Arc<dyn Store>) {
    let conn = Uuid::new_v4();
    let got = s.upsert_records(conn, "m", vec![write(conn, "1", "a"), write(conn, "2", "b")], t(1)).await.unwrap();
    assert_eq!(got, vec![("1".to_owned(), Change::Added), ("2".to_owned(), Change::Added)]);
    let got = s.upsert_records(conn, "m", vec![write(conn, "1", "a"), write(conn, "2", "c"), write(conn, "3", "d")], t(2)).await.unwrap();
    assert_eq!(
        got,
        vec![("1".to_owned(), Change::Unchanged), ("2".to_owned(), Change::Updated), ("3".to_owned(), Change::Added)]
    );
    s.delete_records(conn, "m", vec![(crate::records::record_id(conn, "m", "1"), "1".to_owned())], t(3)).await.unwrap();
    // A record that comes back after a delete is added again.
    let got = s.upsert_records(conn, "m", vec![write(conn, "1", "a")], t(4)).await.unwrap();
    assert_eq!(got, vec![("1".to_owned(), Change::Added)]);
    let q = RecordQuery { model: "m".to_owned(), limit: 100, ..RecordQuery::default() };
    let rows = s.list_records(conn, &q).await.unwrap();
    let one = rows.iter().find(|r| r.external_id == "1").unwrap();
    assert_eq!(one.deleted_at, None);
    assert_eq!(one.created_at, t(4));
    assert_eq!(one.updated_at, t(4));
    let two = rows.iter().find(|r| r.external_id == "2").unwrap();
    assert_eq!(two.created_at, t(1));
    assert_eq!(two.updated_at, t(2));
    assert_eq!(two.data, "sm1.k.2.c");
    // Another model with the same external id is a different record.
    let got = s.upsert_records(conn, "other", vec![write(conn, "1", "a")], t(5)).await.unwrap();
    assert_eq!(got, vec![("1".to_owned(), Change::Added)]);
}

pub async fn record_deletes(s: Arc<dyn Store>) {
    let conn = Uuid::new_v4();
    s.upsert_records(conn, "m", vec![write(conn, "1", "a")], t(1)).await.unwrap();
    let id1 = crate::records::record_id(conn, "m", "1");
    let id9 = crate::records::record_id(conn, "m", "9");
    let got = s.delete_records(conn, "m", vec![(id1, "1".to_owned()), (id9, "9".to_owned())], t(2)).await.unwrap();
    assert_eq!(got, vec![("1".to_owned(), Change::Deleted), ("9".to_owned(), Change::Ignored)]);
    let got = s.delete_records(conn, "m", vec![(id1, "1".to_owned())], t(3)).await.unwrap();
    assert_eq!(got, vec![("1".to_owned(), Change::Ignored)]);
    let q = RecordQuery { model: "m".to_owned(), limit: 10, ..RecordQuery::default() };
    let rows = s.list_records(conn, &q).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].deleted_at, Some(t(2)));
    assert_eq!(rows[0].updated_at, t(2));
}

pub async fn record_listing(s: Arc<dyn Store>) {
    let conn = Uuid::new_v4();
    for (i, ext) in ["a", "b", "c", "d", "e"].iter().enumerate() {
        s.upsert_records(conn, "m", vec![write(conn, ext, "h")], t(i64::try_from(i).unwrap() / 2)).await.unwrap();
    }
    // a, b at t(0); c, d at t(1); e at t(2). Then b changes at t(3), d is deleted at t(4).
    s.upsert_records(conn, "m", vec![write(conn, "b", "h2")], t(3)).await.unwrap();
    s.delete_records(conn, "m", vec![(crate::records::record_id(conn, "m", "d"), "d".to_owned())], t(4)).await.unwrap();

    let base = RecordQuery { model: "m".to_owned(), limit: 100, ..RecordQuery::default() };
    let all = s.list_records(conn, &base).await.unwrap();
    assert_eq!(all.len(), 5);
    for w in all.windows(2) {
        assert!((w[0].updated_at, w[0].id) < (w[1].updated_at, w[1].id));
    }
    assert_eq!(all[3].external_id, "b");
    assert_eq!(all[4].external_id, "d");

    // `limit + 1` rows at most.
    let page = s.list_records(conn, &RecordQuery { limit: 2, ..base.clone() }).await.unwrap();
    assert_eq!(page.len(), 3);
    // The cursor gives the rows after it, with a strict compare.
    let cursor = Cursor { updated_at_us: page[1].updated_at.timestamp_micros(), id: page[1].id };
    let rest = s.list_records(conn, &RecordQuery { cursor: Some(cursor), ..base.clone() }).await.unwrap();
    assert_eq!(rest.len(), 3);
    assert_eq!(rest[0].id, page[2].id);

    let modified = s.list_records(conn, &RecordQuery { modified_after_us: Some(t(2).timestamp_micros()), ..base.clone() }).await.unwrap();
    let ext: Vec<&str> = modified.iter().map(|r| r.external_id.as_str()).collect();
    assert_eq!(ext, vec!["e", "b", "d"]);

    let deleted = s.list_records(conn, &RecordQuery { filter: vec![LastAction::Deleted], ..base.clone() }).await.unwrap();
    assert_eq!(deleted.iter().map(|r| r.external_id.as_str()).collect::<Vec<_>>(), vec!["d"]);
    let updated = s.list_records(conn, &RecordQuery { filter: vec![LastAction::Updated], ..base.clone() }).await.unwrap();
    assert_eq!(updated.iter().map(|r| r.external_id.as_str()).collect::<Vec<_>>(), vec!["b"]);
    let added = s.list_records(conn, &RecordQuery { filter: vec![LastAction::Added], ..base.clone() }).await.unwrap();
    assert_eq!(added.iter().map(|r| r.external_id.as_str()).collect::<Vec<_>>(), vec!["a", "c", "e"]);
    let by_id = s.list_records(conn, &RecordQuery { external_ids: vec!["c".to_owned(), "zz".to_owned()], ..base.clone() }).await.unwrap();
    assert_eq!(by_id.iter().map(|r| r.external_id.as_str()).collect::<Vec<_>>(), vec!["c"]);
    let other_model = s.list_records(conn, &RecordQuery { model: "x".to_owned(), ..base }).await.unwrap();
    assert!(other_model.is_empty());
}

pub async fn sync_states(s: Arc<dyn Store>) {
    let conn = Uuid::new_v4();
    assert_eq!(s.get_sync_state(conn, "b").await.unwrap(), None);
    s.put_sync_state(sync_state(conn, "b")).await.unwrap();
    s.put_sync_state(sync_state(conn, "a")).await.unwrap();
    let mut changed = sync_state(conn, "b");
    changed.status = SyncStatus::Error;
    changed.last_error = Some("boom".to_owned());
    changed.paused = true;
    s.put_sync_state(changed.clone()).await.unwrap();
    assert_eq!(s.get_sync_state(conn, "b").await.unwrap(), Some(changed));
    let names: Vec<String> = s.list_sync_states(conn).await.unwrap().into_iter().map(|r| r.sync_name).collect();
    assert_eq!(names, vec!["a", "b"]);
}

pub async fn environments_are_isolated(s: Arc<dyn Store>) {
    let (e1, e2) = (env(), env());
    s.insert_integration(integration(&e1, "gh")).await.unwrap();
    s.insert_integration(integration(&e2, "gh")).await.unwrap();
    s.upsert_connection(connection(&e1, "gh", "c1")).await.unwrap();
    assert!(s.get_connection(&e2, "gh", "c1").await.unwrap().is_none());
    assert_eq!(s.list_connections(&e2, &ConnectionFilter::default()).await.unwrap().len(), 0);
    assert!(s.delete_integration(&e2, "gh").await.unwrap());
    assert!(s.get_connection(&e1, "gh", "c1").await.unwrap().is_some());
    assert_eq!(s.list_integrations(&e1).await.unwrap().len(), 1);
}
