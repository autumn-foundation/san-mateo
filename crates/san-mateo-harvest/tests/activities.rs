//! The activities against a real San Mateo service with a fake network (AC21).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use autumn_harvest::HarvestBuilder;
use autumn_harvest::context::ActivityContext;
use autumn_harvest::failure::{ActivityFailure, parse_typed_payload};
use autumn_harvest::types::{ActivityExecId, IdempotencyKey};
use common::*;
use san_mateo::transport::HttpResponse;
use san_mateo::{ConnectionKey, ProxyRequest, RecordsRequest, SanMateo};
use san_mateo_harvest::{
    ConnectionInput, ProxyInput, RefreshDueInput, RunActionInput, RunSyncInput, SyncRunOutput,
    proxy_with,
};
use serde_json::{Value, json};

fn assert_failure_has_no_secrets(f: &ActivityFailure) {
    assert_no_secrets(&serde_json::to_string(f).unwrap());
}

// ------------------------------------------------------------------ proxy

#[tokio::test]
async fn proxy_get_returns_status_headers_and_json_body() {
    let h = ready().await;
    h.http.reply(
        "GET",
        &format!("{CRM}/v3/contacts"),
        HttpResponse::json(200, &json!([{"id": "1"}]))
            .header("x-request-id", "r-1")
            .header("set-cookie", "session=s-1"),
    );
    let ctx = ActivityContext::new_test();
    let out = ProxyInput::new(ENV, &crm(), "GET", "/v3/contacts")
        .query("limit", "10")
        .send(&h.san, &ctx)
        .await
        .unwrap();
    assert_eq!(out.status, 200);
    assert!(out.is_success());
    assert_eq!(out.body, json!([{"id": "1"}]));
    assert!(
        out.headers
            .contains(&("x-request-id".to_owned(), "r-1".to_owned()))
    );
    assert!(out.headers.iter().all(|(n, _)| n != "set-cookie"));
    assert_no_secrets(&serde_json::to_string(&out).unwrap());

    let sent = h.http.requests_to(&format!("{CRM}/v3/contacts"));
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].header_value("authorization"),
        Some(format!("Bearer {ACCESS_TOKEN}").as_str())
    );
    assert!(sent[0].url.contains("limit=10"), "{}", sent[0].url);
    assert_eq!(sent[0].header_value("idempotency-key"), None);
}

#[tokio::test]
async fn proxy_forwards_the_idempotency_key_when_asked() {
    let h = ready().await;
    h.http.reply(
        "POST",
        &format!("{CRM}/v3/contacts"),
        HttpResponse::json(201, &json!({"id": "9"})),
    );
    let ctx = ActivityContext::new_test();
    let expected = ctx.idempotency_key().unwrap().as_str().to_owned();
    let out = ProxyInput::new(ENV, &crm(), "POST", "/v3/contacts")
        .json(json!({"name": "Ada"}))
        .forward_idempotency_key(true)
        .send(&h.san, &ctx)
        .await
        .unwrap();
    assert_eq!(out.status, 201);
    let sent = h.http.requests_to(&format!("{CRM}/v3/contacts"));
    assert_eq!(sent[0].header_value("idempotency-key"), Some(expected.as_str()));
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(body, json!({"name": "Ada"}));

    // Each attempt of one call has the same key.
    let id = ActivityExecId::new();
    for attempt in 1..=2 {
        let ctx = ActivityContext::new_test()
            .with_idempotency_key(IdempotencyKey::from_activity_exec_id(id))
            .with_attempt(attempt);
        proxy_with(
            &h.san,
            &ctx,
            ENV,
            &crm(),
            ProxyRequest::new("POST", "/v3/contacts").header("Idempotency-Key", "caller-key"),
            true,
        )
        .await
        .unwrap();
    }
    let sent = h.http.requests_to(&format!("{CRM}/v3/contacts"));
    let keys: Vec<Vec<&str>> = sent[1..]
        .iter()
        .map(|r| {
            r.headers
                .iter()
                .filter(|(n, _)| n.eq_ignore_ascii_case("idempotency-key"))
                .map(|(_, v)| v.as_str())
                .collect()
        })
        .collect();
    let want = IdempotencyKey::from_activity_exec_id(id);
    assert_eq!(keys, vec![vec![want.as_str()], vec![want.as_str()]]);
}

#[tokio::test]
async fn proxy_without_an_idempotency_key_on_the_context_is_a_config_failure() {
    let h = ready().await;
    let ctx = ActivityContext::new_for_idempotency_test_no_key();
    let f = ProxyInput::new(ENV, &crm(), "POST", "/v3/contacts")
        .forward_idempotency_key(true)
        .send(&h.san, &ctx)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::CONFIG_ERROR);
    assert!(f.non_retryable);
    assert!(h.http.requests().is_empty());
}

#[tokio::test]
async fn proxy_retryable_statuses_are_retryable_failures() {
    let h = ready().await;
    let url = format!("{KEYED}/v1/items");
    for status in [500_u16, 502, 503, 408, 425, 429] {
        h.http.reply("GET", &url, HttpResponse::new(status, "busy"));
        let f = ProxyInput::new(ENV, &keyed(), "GET", "/v1/items")
            .send(&h.san, &ActivityContext::new_test())
            .await
            .unwrap_err();
        assert_eq!(f.error_type, format!("san_mateo.http_{status}"));
        assert!(!f.non_retryable, "{status}");
        assert_eq!(f.details.as_ref().unwrap()["status"], status);
        assert_failure_has_no_secrets(&f);
    }
}

#[tokio::test]
async fn proxy_retry_after_becomes_a_retry_hint() {
    let h = ready().await;
    h.http.reply(
        "GET",
        &format!("{KEYED}/v1/items"),
        HttpResponse::new(503, "busy").header("Retry-After", "7"),
    );
    let f = proxy_with(
        &h.san,
        &ActivityContext::new_test(),
        ENV,
        &keyed(),
        ProxyRequest::get("/v1/items"),
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.http_503");
    assert_eq!(f.retry_after, Some(Duration::from_secs(7)));

    // A client error is not retryable, so it gets no hint.
    h.http.reply(
        "GET",
        &format!("{KEYED}/v1/items"),
        HttpResponse::new(404, "gone").header("Retry-After", "7"),
    );
    let f = proxy_with(
        &h.san,
        &ActivityContext::new_test(),
        ENV,
        &keyed(),
        ProxyRequest::get("/v1/items"),
        false,
    )
    .await
    .unwrap_err();
    assert!(f.non_retryable);
    assert_eq!(f.retry_after, None);
}

#[tokio::test]
async fn proxy_client_errors_are_not_retryable_and_hold_a_body_preview() {
    let h = ready().await;
    h.http.reply(
        "GET",
        &format!("{KEYED}/v1/items/9"),
        HttpResponse::json(404, &json!({"error": "no item 9"})),
    );
    let f = ProxyInput::new(ENV, &keyed(), "GET", "/v1/items/9")
        .header("X-Trace", "t-1")
        .send(&h.san, &ActivityContext::new_test())
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.http_404");
    assert!(f.non_retryable);
    let details = f.details.clone().unwrap();
    assert_eq!(details["status"], 404);
    assert_eq!(details["body_preview"], r#"{"error":"no item 9"}"#);
    // No request header goes into the failure.
    let text = serde_json::to_string(&f).unwrap();
    assert!(!text.contains("t-1"), "{text}");
    assert!(!text.contains("x-api-key"), "{text}");
    assert_failure_has_no_secrets(&f);
    // San Mateo added the API key upstream.
    let sent = h.http.requests_to(&format!("{KEYED}/v1/items/9"));
    assert_eq!(sent[0].header_value("x-api-key"), Some(API_KEY));
}

#[tokio::test]
async fn proxy_text_and_empty_bodies() {
    let h = ready().await;
    h.http.reply(
        "GET",
        &format!("{KEYED}/v1/ping"),
        HttpResponse::new(200, "pong").header("content-type", "text/plain"),
    );
    h.http.reply(
        "DELETE",
        &format!("{KEYED}/v1/items/1"),
        HttpResponse::new(204, ""),
    );
    let ctx = ActivityContext::new_test();
    let out = ProxyInput::new(ENV, &keyed(), "GET", "/v1/ping")
        .send(&h.san, &ctx)
        .await
        .unwrap();
    assert_eq!(out.body, json!("pong"));
    let out = ProxyInput::new(ENV, &keyed(), "DELETE", "/v1/items/1")
        .send(&h.san, &ctx)
        .await
        .unwrap();
    assert_eq!(out.status, 204);
    assert_eq!(out.body, Value::Null);
}

#[tokio::test]
async fn proxy_san_mateo_errors_map_to_codes() {
    let h = ready().await;
    let ctx = ActivityContext::new_test();
    let f = ProxyInput::new(ENV, &ConnectionKey::new("crm", "missing"), "GET", "/x")
        .send(&h.san, &ctx)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.unknown_connection");
    assert!(f.non_retryable);

    h.http.fail(
        "GET",
        &format!("{KEYED}/v1/flaky"),
        san_mateo::transport::TransportError::Timeout,
    );
    let f = ProxyInput::new(ENV, &keyed(), "GET", "/v1/flaky")
        .send(&h.san, &ctx)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.upstream_error");
    assert!(!f.non_retryable);
    assert_failure_has_no_secrets(&f);
}

#[tokio::test]
async fn proxy_input_with_a_secret_header_sends_nothing() {
    let h = ready().await;
    let f = ProxyInput::new(ENV, &crm(), "GET", "/v3/contacts")
        .header("Authorization", "Bearer mine")
        .send(&h.san, &ActivityContext::new_test())
        .await
        .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::INVALID_INPUT_ERROR);
    assert!(f.non_retryable);
    assert!(h.http.requests().is_empty());
}

// ---------------------------------------------------------------- refresh

#[tokio::test]
async fn refresh_forces_a_new_token_and_returns_only_the_expiry() {
    let h = ready().await;
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply(NEW_ACCESS_TOKEN, NEW_REFRESH_TOKEN, 7_200),
    );
    let out = ConnectionInput::new(ENV, &crm())
        .refresh(&h.san)
        .await
        .unwrap();
    let expected = (start() + autumn_harvest::chrono::Duration::seconds(7_200)).to_rfc3339();
    let got = autumn_harvest::chrono::DateTime::parse_from_rfc3339(out.expires_at.as_deref().unwrap())
        .unwrap()
        .to_rfc3339();
    assert_eq!(
        got,
        autumn_harvest::chrono::DateTime::parse_from_rfc3339(&expected)
            .unwrap()
            .to_rfc3339()
    );
    assert_eq!(h.http.requests_to(TOKEN_URL).len(), 1);
    assert_no_secrets(&serde_json::to_string(&out).unwrap());

    // The proxy now sends the new token.
    h.http.reply(
        "GET",
        &format!("{CRM}/v3/me"),
        HttpResponse::json(200, &json!({})),
    );
    ProxyInput::new(ENV, &crm(), "GET", "/v3/me")
        .send(&h.san, &ActivityContext::new_test())
        .await
        .unwrap();
    let sent = h.http.requests_to(&format!("{CRM}/v3/me"));
    assert_eq!(
        sent[0].header_value("authorization"),
        Some(format!("Bearer {NEW_ACCESS_TOKEN}").as_str())
    );
}

#[tokio::test]
async fn refresh_of_an_api_key_has_no_expiry() {
    let h = ready().await;
    let out = ConnectionInput::new(ENV, &keyed())
        .refresh(&h.san)
        .await
        .unwrap();
    assert_eq!(out.expires_at, None);
    assert!(h.http.requests().is_empty());
}

#[tokio::test]
async fn refresh_failures_follow_the_provider_status() {
    let h = ready().await;
    h.http.reply(
        "POST",
        TOKEN_URL,
        HttpResponse::json(400, &json!({"error": "invalid_grant"})),
    );
    let f = ConnectionInput::new(ENV, &crm())
        .refresh(&h.san)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.refresh_failed");
    assert!(f.non_retryable);
    assert_failure_has_no_secrets(&f);

    h.http
        .reply("POST", TOKEN_URL, HttpResponse::new(503, "down"));
    let f = ConnectionInput::new(ENV, &crm())
        .refresh(&h.san)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.refresh_failed");
    assert!(!f.non_retryable);
    assert_failure_has_no_secrets(&f);
}

// ------------------------------------------------------------- run sync

#[tokio::test]
async fn run_sync_saves_records_and_returns_the_report() {
    let h = ready().await;
    h.http.reply(
        "GET",
        &format!("{CRM}/v3/contacts"),
        HttpResponse::json(200, &json!([{"id": "1", "name": "Ada"}, {"id": "2", "name": "Bob"}])),
    );
    let out = RunSyncInput::new(ENV, &crm(), "contacts")
        .run(&h.san)
        .await
        .unwrap();
    assert_eq!(
        out,
        SyncRunOutput {
            sync_name: "contacts".into(),
            provider_config_key: "crm".into(),
            connection_id: "c-1".into(),
            success: true,
            counts: san_mateo::store::SyncCounts {
                added: 2,
                updated: 0,
                deleted: 0,
            },
            error: None,
        }
    );
    assert_eq!(h.contacts_runs.load(Ordering::SeqCst), 1);
    let page = h
        .san
        .list_records(
            ENV,
            &crm(),
            RecordsRequest {
                model: "Contact".into(),
                ..RecordsRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.records.len(), 2);
    // A second run sees no change.
    let out = RunSyncInput::new(ENV, &crm(), "contacts")
        .run(&h.san)
        .await
        .unwrap();
    assert_eq!(out.counts.added, 0);
}

#[tokio::test]
async fn a_failed_sync_run_is_a_retryable_failure() {
    let h = ready().await;
    let f = RunSyncInput::new(ENV, &crm(), "broken")
        .run(&h.san)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::SYNC_FAILED_ERROR);
    assert!(!f.non_retryable);
    let details = f.details.clone().unwrap();
    assert_eq!(details["sync_name"], "broken");
    assert_eq!(details["connection_id"], "c-1");
    assert_eq!(details["provider_config_key"], "crm");
    assert!(
        details["error"]
            .as_str()
            .unwrap()
            .contains("upstream shape changed")
    );
    assert_failure_has_no_secrets(&f);
}

#[tokio::test]
async fn run_sync_for_an_unknown_sync_or_connection_is_not_retryable() {
    let h = ready().await;
    let f = RunSyncInput::new(ENV, &crm(), "nope")
        .run(&h.san)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.unknown_function");
    assert!(f.non_retryable);
    let f = RunSyncInput::new(ENV, &ConnectionKey::new("crm", "gone"), "contacts")
        .run(&h.san)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.unknown_connection");
    assert!(f.non_retryable);
}

// ----------------------------------------------------------- run action

#[tokio::test]
async fn run_action_returns_the_action_output() {
    let h = ready().await;
    h.http.reply(
        "POST",
        &format!("{CRM}/v3/contacts"),
        HttpResponse::json(201, &json!({"id": "c-77"})),
    );
    let out = RunActionInput::new(ENV, &crm(), "create_contact", json!({"name": "Ada"}))
        .run(&h.san)
        .await
        .unwrap();
    assert_eq!(out, json!({"id": "c-77"}));
    let sent = h.http.requests_to(&format!("{CRM}/v3/contacts"));
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(body, json!({"name": "Ada"}));
}

#[tokio::test]
async fn run_action_failures() {
    let h = ready().await;
    let f = RunActionInput::new(ENV, &crm(), "refuse", Value::Null)
        .run(&h.san)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.function_failed");
    assert!(f.non_retryable);
    let f = RunActionInput::new(ENV, &crm(), "missing", Value::Null)
        .run(&h.san)
        .await
        .unwrap_err();
    assert_eq!(f.error_type, "san_mateo.unknown_function");
}

// ---------------------------------------------------------- refresh due

#[tokio::test]
async fn refresh_due_returns_counts() {
    let h = ready().await;
    h.http.reply(
        "POST",
        TOKEN_URL,
        token_reply(NEW_ACCESS_TOKEN, NEW_REFRESH_TOKEN, 3_600),
    );
    // The token expires in one hour. A sweep for 10 minutes does nothing.
    let out = RefreshDueInput {
        within_secs: 600,
        limit: 10,
    }
    .run(&h.san)
    .await
    .unwrap();
    assert_eq!((out.refreshed, out.failed), (0, 0));
    // The default sweep window is 30 minutes. Move the clock by 45 minutes.
    h.clock.advance(autumn_harvest::chrono::Duration::minutes(45));
    let out = RefreshDueInput::default().run(&h.san).await.unwrap();
    assert_eq!(out.refreshed, 1);
    assert_eq!(out.failed, 0);
    assert_eq!(h.http.requests_to(TOKEN_URL).len(), 1);
    assert_no_secrets(&serde_json::to_string(&out).unwrap());
}

// ------------------------------------------------- the activity handlers

/// Calls an activity through its harvest handler, as a worker does.
async fn call(
    info: autumn_harvest::ActivityInfo,
    ctx: &ActivityContext,
    input: Value,
) -> Result<Value, ActivityFailure> {
    (info.handler)(ctx, input)
        .await
        .map_err(|payload| parse_typed_payload(&payload).expect("a typed failure"))
}

#[tokio::test]
async fn each_activity_without_state_fails_with_a_config_failure() {
    let ctx = ActivityContext::new_test();
    let connection = json!({
        "environment": ENV,
        "provider_config_key": "crm",
        "connection_id": "c-1"
    });
    let mut proxy = connection.clone();
    proxy["method"] = json!("GET");
    proxy["endpoint"] = json!("/x");
    let mut sync = connection.clone();
    sync["sync_name"] = json!("contacts");
    let mut action = connection.clone();
    action["action_name"] = json!("create_contact");
    let calls = [
        (san_mateo_harvest::san_mateo_proxy_info(), proxy),
        (san_mateo_harvest::san_mateo_refresh_info(), connection),
        (san_mateo_harvest::san_mateo_run_sync_info(), sync),
        (san_mateo_harvest::san_mateo_run_action_info(), action),
        (san_mateo_harvest::san_mateo_refresh_due_info(), json!({})),
    ];
    for (info, input) in calls {
        let name = info.name;
        let f = call(info, &ctx, input).await.unwrap_err();
        assert_eq!(f.error_type, san_mateo_harvest::CONFIG_ERROR, "{name}");
        assert!(f.non_retryable, "{name}");
    }
}

#[tokio::test]
async fn the_activity_functions_without_state_fail_with_a_config_failure() {
    let ctx = ActivityContext::new_test();
    let f = san_mateo_harvest::san_mateo_proxy(&ctx, ProxyInput::new(ENV, &crm(), "GET", "/x"))
        .await
        .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::CONFIG_ERROR);
    let f = san_mateo_harvest::san_mateo_refresh(&ctx, ConnectionInput::new(ENV, &crm()))
        .await
        .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::CONFIG_ERROR);
    let f = san_mateo_harvest::san_mateo_run_sync(&ctx, RunSyncInput::new(ENV, &crm(), "contacts"))
        .await
        .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::CONFIG_ERROR);
    let f = san_mateo_harvest::san_mateo_run_action(
        &ctx,
        RunActionInput::new(ENV, &crm(), "create_contact", Value::Null),
    )
    .await
    .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::CONFIG_ERROR);
    let f = san_mateo_harvest::san_mateo_refresh_due(&ctx, RefreshDueInput::default())
        .await
        .unwrap_err();
    assert_eq!(f.error_type, san_mateo_harvest::CONFIG_ERROR);
}

#[tokio::test]
async fn registration_in_one_call() {
    let h = ready().await;
    let built = HarvestBuilder::new()
        .workflows(san_mateo_harvest::workflows())
        .activities(san_mateo_harvest::activities())
        .state(h.san.clone())
        .try_build()
        .unwrap();
    assert!(built.state::<SanMateo>().is_some());
}
