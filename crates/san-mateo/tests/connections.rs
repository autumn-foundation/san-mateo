//! Connections API and token refresh (AC8, AC9).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::*;
use san_mateo::model::ConnectionKey;
use san_mateo::store::{ConnectionFilter, Store};
use san_mateo::transport::{HttpResponse, TransportError};
use san_mateo::{ConnectionPatch, Error, GetOptions};
use serde_json::{Map, json};

const TOKEN_URL: &str = "https://auth.crm.example.com/token";

fn refresh_form(h: &Harness, n: usize) -> BTreeMap<String, String> {
    let reqs = h.http.requests_to(TOKEN_URL);
    url::form_urlencoded::parse(&reqs[n].body)
        .into_owned()
        .collect()
}

#[tokio::test]
async fn get_returns_fresh_credentials_without_a_refresh() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    let got = h
        .san
        .get_connection("dev", &conn.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-1"));
    assert_eq!(got.last_fetched_at, Some(now(&h)));
    assert_eq!(
        h.http.requests_to(TOKEN_URL).len(),
        1,
        "only the code exchange"
    );
    assert!(matches!(
        h.san
            .get_connection(
                "dev",
                &ConnectionKey::new("crm", "nope"),
                GetOptions::default()
            )
            .await,
        Err(Error::UnknownConnection(_))
    ));
    assert!(matches!(
        h.san
            .get_connection("prod", &conn.key(), GetOptions::default())
            .await,
        Err(Error::UnknownConnection(_))
    ));
}

#[tokio::test]
async fn refresh_inside_the_buffer_keeps_the_old_refresh_token() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    // 46 minutes later, 14 minutes remain: inside the 15 minute buffer.
    h.clock.advance(chrono::Duration::minutes(46));
    h.http
        .reply("POST", TOKEN_URL, token_reply("at-2", None, Some(3_600)));
    let got = h
        .san
        .get_connection("dev", &conn.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-2"));
    assert_eq!(got.credentials.refresh_token(), Some("rt-1"));
    assert_eq!(
        got.credentials.expires_at(),
        Some(now(&h) + chrono::Duration::seconds(3_600))
    );
    let form = refresh_form(&h, 1);
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["refresh_token"], "rt-1");
    assert_eq!(form["client_id"], "crm-client");
    let row = h
        .store
        .get_connection("dev", "crm", &conn.connection_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.last_refresh_success, Some(now(&h)));
    assert_eq!(row.credentials_expires_at, got.credentials.expires_at());
}

#[tokio::test]
async fn header_authorization_method_refreshes_with_basic_auth() {
    let h = shop_harness().await;
    let key = shop_connection(&h, "s1", json!({}), Some("rt-1")).await;
    let url = "https://auth.shop.example.com/token";
    h.http
        .reply("POST", url, token_reply("at-2", None, Some(3_600)));
    let got = h
        .san
        .get_connection(
            "dev",
            &key,
            GetOptions {
                force_refresh: true,
                ..GetOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-2"));
    let sent = &h.http.requests_to(url)[0];
    assert_eq!(
        sent.header_value("authorization"),
        Some("Basic c2hvcC1jbGllbnQ6c2hvcC1zZWNyZXQ=")
    );
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(&sent.body)
        .into_owned()
        .collect();
    assert_eq!(form["refresh_token"], "rt-1");
    assert!(!form.contains_key("client_id") && !form.contains_key("client_secret"));
}

#[tokio::test]
async fn force_refresh_and_no_refresh_token() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply("at-2", Some("rt-2"), Some(3_600)),
    );
    let got = h
        .san
        .get_connection(
            "dev",
            &conn.key(),
            GetOptions {
                force_refresh: true,
                ..GetOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-2"));
    assert_eq!(got.credentials.refresh_token(), Some("rt-2"));

    // With no refresh token, an expired token comes back as it is.
    let plain = connect_crm(&h, "at-x", None, Some(60)).await;
    h.clock.advance(chrono::Duration::hours(2));
    let before = h.http.requests_to(TOKEN_URL).len();
    let got = h
        .san
        .get_connection("dev", &plain.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-x"));
    assert_eq!(h.http.requests_to(TOKEN_URL).len(), before);
    // `refresh_if_needed = false` skips the refresh.
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
    assert_eq!(got.credentials.bearer_token(), Some("at-2"));
}

#[tokio::test]
async fn refresh_failures_cool_down_and_exhaust() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    let key = conn.key();
    h.http.reply(
        "POST",
        TOKEN_URL,
        HttpResponse::json(400, &json!({"error": "invalid_grant"})),
    );
    h.clock.advance(chrono::Duration::minutes(5));
    let err = h
        .san
        .get_connection("dev", &key, GetOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Provider {
                code: "refresh_failed",
                status: Some(400),
                ..
            }
        ),
        "{err:?}"
    );
    // A second call in the cooldown does not call the provider.
    let calls = h.http.requests_to(TOKEN_URL).len();
    assert_eq!(
        h.san
            .get_connection("dev", &key, GetOptions::default())
            .await
            .unwrap_err(),
        Error::RefreshCooldown
    );
    assert_eq!(h.http.requests_to(TOKEN_URL).len(), calls);
    // Failures on the same day count once.
    h.clock.advance(chrono::Duration::seconds(31));
    assert!(
        h.san
            .get_connection("dev", &key, GetOptions::default())
            .await
            .is_err()
    );
    let row = h
        .store
        .get_connection("dev", "crm", &conn.connection_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.refresh_attempts, 1);
    assert!(!row.refresh_exhausted);
    // Failures on four days exhaust the connection.
    for _ in 0..3 {
        h.clock.advance(chrono::Duration::days(1));
        assert!(
            h.san
                .get_connection("dev", &key, GetOptions::default())
                .await
                .is_err()
        );
    }
    let row = h
        .store
        .get_connection("dev", "crm", &conn.connection_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.refresh_attempts, 4);
    assert!(row.refresh_exhausted);
    h.clock.advance(chrono::Duration::days(1));
    assert_eq!(
        h.san
            .get_connection("dev", &key, GetOptions::default())
            .await
            .unwrap_err(),
        Error::RefreshExhausted
    );
    // A forced refresh tries again. Success resets the counters.
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply("at-9", Some("rt-9"), Some(3_600)),
    );
    let got = h
        .san
        .get_connection(
            "dev",
            &key,
            GetOptions {
                force_refresh: true,
                ..GetOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-9"));
    let row = h
        .store
        .get_connection("dev", "crm", &conn.connection_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            row.refresh_attempts,
            row.refresh_exhausted,
            row.last_refresh_failure
        ),
        (0, false, None)
    );

    h.san.flush().await;
    let refresh_hooks: Vec<_> = webhooks_sent(&h)
        .into_iter()
        .filter(|w| w["operation"] == "refresh")
        .collect();
    assert!(!refresh_hooks.is_empty());
    assert!(
        refresh_hooks
            .iter()
            .all(|w| w["success"] == false && w["error"]["type"] == "refresh_failed")
    );
}

#[tokio::test]
async fn network_errors_during_refresh_are_retryable() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    h.http.fail("POST", TOKEN_URL, TransportError::Timeout);
    let err = h
        .san
        .get_connection("dev", &conn.key(), GetOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::Provider {
                code: "refresh_failed",
                status: None,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.is_retryable());
}

#[tokio::test]
async fn concurrent_gets_share_one_refresh() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply("at-2", Some("rt-2"), Some(3_600)),
    );
    let key = conn.key();
    let calls = (0..8).map(|_| h.san.get_connection("dev", &key, GetOptions::default()));
    let results = futures::future::join_all(calls).await;
    assert!(
        results
            .iter()
            .all(|r| r.as_ref().unwrap().credentials.bearer_token() == Some("at-2"))
    );
    assert_eq!(
        h.http.requests_to(TOKEN_URL).len(),
        2,
        "one code exchange and one refresh"
    );
}

#[tokio::test(start_paused = true)]
async fn a_lease_of_another_process_is_respected() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    let lease = format!("refresh:dev:crm:{}", conn.connection_id);
    assert!(
        h.store
            .try_lease(&lease, "other-process", now(&h), 60_000)
            .await
            .unwrap()
    );
    // The other process writes fresh credentials after a short time.
    let store = h.store.clone();
    let keys = h.keys.clone();
    let id = conn.connection_id.clone();
    let fresh_at = now(&h) + chrono::Duration::hours(1);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut row = store
            .get_connection("dev", "crm", &id)
            .await
            .unwrap()
            .unwrap();
        let mut creds: san_mateo::Credentials = keys
            .open_json(
                &row.credentials,
                &san_mateo::crypto::aad(&["connection", "dev", "crm", &id]),
            )
            .unwrap();
        if let san_mateo::Credentials::OAuth2(c) = &mut creds {
            c.access_token = "from-other".into();
            c.expires_at = Some(fresh_at);
        }
        row.credentials = keys
            .seal_json(
                &creds,
                &san_mateo::crypto::aad(&["connection", "dev", "crm", &id]),
            )
            .unwrap();
        row.credentials_expires_at = Some(fresh_at);
        store.update_connection(&row).await.unwrap();
    });
    let got = h
        .san
        .get_connection("dev", &conn.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("from-other"));
    assert_eq!(
        h.http.requests_to(TOKEN_URL).len(),
        1,
        "no refresh in this process"
    );
}

#[tokio::test(start_paused = true)]
async fn a_held_lease_with_no_progress_gives_a_cooldown_error() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(60)).await;
    h.clock.advance(chrono::Duration::minutes(5));
    let lease = format!("refresh:dev:crm:{}", conn.connection_id);
    assert!(
        h.store
            .try_lease(&lease, "stuck-process", now(&h), 600_000)
            .await
            .unwrap()
    );
    let err = h
        .san
        .get_connection("dev", &conn.key(), GetOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err, Error::RefreshCooldown);
}

#[tokio::test]
async fn client_credentials_refresh_with_a_new_grant() {
    let h = ready().await;
    let token = session(&h).await;
    h.http.reply(
        "POST",
        "https://auth.machine.example.com/token",
        HttpResponse::json(200, &json!({"access_token": "cc-1", "expires_in": 600})),
    );
    let cc = h
        .san
        .connect_client_credentials(
            &token,
            "machine",
            "m-id".into(),
            "m-secret".into(),
            san_mateo::ConnectOptions::default(),
        )
        .await
        .unwrap();
    h.clock.advance(chrono::Duration::minutes(9));
    h.http.reply(
        "POST",
        "https://auth.machine.example.com/token",
        HttpResponse::json(200, &json!({"access_token": "cc-2", "expires_in": 600})),
    );
    let got = h
        .san
        .get_connection("dev", &cc.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("cc-2"));
    let reqs = h.http.requests_to("https://auth.machine.example.com/token");
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(&reqs[1].body)
        .into_owned()
        .collect();
    assert_eq!(form["grant_type"], "client_credentials");
    assert_eq!(form["client_id"], "m-id");
}

#[tokio::test]
async fn list_delete_metadata_and_patch() {
    let h = ready().await;
    let a = connect_crm(&h, "at-a", None, None).await;
    let b = connect_crm(&h, "at-b", None, None).await;
    let all = h
        .san
        .list_connections("dev", &ConnectionFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert!(
        !format!("{all:?}").contains("at-a"),
        "no credentials in a list"
    );
    let by_user = ConnectionFilter {
        end_user_id: Some("user-1".into()),
        ..ConnectionFilter::default()
    };
    assert_eq!(
        h.san.list_connections("dev", &by_user).await.unwrap().len(),
        2
    );

    let m = h
        .san
        .set_metadata(
            "dev",
            &a.key(),
            Map::from_iter([("x".to_owned(), json!(1)), ("y".to_owned(), json!(2))]),
            false,
        )
        .await
        .unwrap();
    assert_eq!(m.len(), 2);
    let m = h
        .san
        .set_metadata(
            "dev",
            &a.key(),
            Map::from_iter([("y".to_owned(), json!(3))]),
            true,
        )
        .await
        .unwrap();
    assert_eq!(json!(m), json!({"x": 1, "y": 3}));
    let m = h
        .san
        .set_metadata(
            "dev",
            &a.key(),
            Map::from_iter([("z".to_owned(), json!(0))]),
            false,
        )
        .await
        .unwrap();
    assert_eq!(json!(m), json!({"z": 0}));

    h.san
        .patch_connection(
            "dev",
            &a.key(),
            ConnectionPatch {
                tags: Some(BTreeMap::from([("t".to_owned(), "1".to_owned())])),
                webhook_url_override: Some(Some("https://override.example.com/h".to_owned())),
                end_user: None,
            },
        )
        .await
        .unwrap();
    let got = h
        .san
        .get_connection("dev", &a.key(), GetOptions::default())
        .await
        .unwrap();
    assert_eq!(got.tags["t"], "1");
    assert_eq!(
        got.webhook_url_override.as_deref(),
        Some("https://override.example.com/h")
    );
    assert!(matches!(
        h.san
            .patch_connection(
                "dev",
                &a.key(),
                ConnectionPatch {
                    webhook_url_override: Some(Some("ftp://x".into())),
                    ..ConnectionPatch::default()
                }
            )
            .await,
        Err(Error::Invalid(_))
    ));

    h.http.reply(
        "POST",
        "https://override.example.com/h",
        HttpResponse::new(200, ""),
    );
    h.san.delete_connection("dev", &a.key()).await.unwrap();
    assert!(matches!(
        h.san.delete_connection("dev", &a.key()).await,
        Err(Error::UnknownConnection(_))
    ));
    assert_eq!(
        h.san
            .list_connections("dev", &ConnectionFilter::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        h.san.set_metadata("dev", &a.key(), Map::new(), true).await,
        Err(Error::UnknownConnection(_))
    ));
    h.san.flush().await;
    let deletion = h.http.requests_to("https://override.example.com/h");
    assert_eq!(
        deletion.len(),
        1,
        "the override URL gets the deletion event"
    );
    let body: serde_json::Value = serde_json::from_slice(&deletion[0].body).unwrap();
    assert_eq!(body["operation"], "deletion");
    let _ = b;
}

#[tokio::test]
async fn the_sweep_refreshes_tokens_that_expire_soon() {
    let h = ready().await;
    let soon = connect_crm(&h, "at-soon", Some("rt-soon"), Some(600)).await;
    let late = connect_crm(&h, "at-late", Some("rt-late"), Some(86_400)).await;
    let broken = connect_crm(&h, "at-broken", Some("rt-broken"), Some(300)).await;
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply("at-new", Some("rt-new"), Some(3_600)),
    );
    let _ = broken;
    let report = h
        .san
        .refresh_due(Duration::from_secs(30 * 60), 100)
        .await
        .unwrap();
    assert_eq!(report.refreshed, 2);
    assert_eq!(report.failed, 0);
    let got = h
        .san
        .get_connection(
            "dev",
            &soon.key(),
            GetOptions {
                refresh_if_needed: false,
                force_refresh: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-new"));
    let got = h
        .san
        .get_connection(
            "dev",
            &late.key(),
            GetOptions {
                refresh_if_needed: false,
                force_refresh: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-late"));

    h.clock.advance(chrono::Duration::minutes(50));
    h.http.reply("POST", TOKEN_URL, HttpResponse::new(503, ""));
    let report = h
        .san
        .refresh_due(Duration::from_secs(30 * 60), 100)
        .await
        .unwrap();
    assert_eq!(report.failed, 2);
    let report = h
        .san
        .refresh_due(Duration::from_secs(30 * 60), 100)
        .await
        .unwrap();
    assert_eq!(report.skipped, 2, "cooldown");
}

#[tokio::test(start_paused = true)]
async fn background_loops_start_and_stop() {
    let h = ready().await;
    let soon = connect_crm(&h, "at-soon", Some("rt-soon"), Some(600)).await;
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply("at-bg", Some("rt-bg"), Some(3_600)),
    );
    let bg = h.san.start_background(san_mateo::BackgroundSettings {
        tick: Duration::from_secs(1),
        ..san_mateo::BackgroundSettings::default()
    });
    tokio::time::sleep(Duration::from_secs(3)).await;
    bg.stop().await;
    let got = h
        .san
        .get_connection(
            "dev",
            &soon.key(),
            GetOptions {
                refresh_if_needed: false,
                force_refresh: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-bg"));
}
