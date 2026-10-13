//! AC22: real camel routes call provider APIs through the `san-mateo:`
//! producer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::sync::Arc;

use camel_api::{Body, CamelError, Message};
use camel_component_api::{Component, NoOpComponentContext, RuntimeObservability};
use common::*;
use san_mateo::transport::{HttpResponse, TransportError};
use san_mateo_camel::{
    HEADER_CONNECTION_ID, HEADER_METHOD, HEADER_PATH, HEADER_RESPONSE_CODE, SCHEME,
    SanMateoComponent,
};
use serde_json::json;

fn http_status(err: &CamelError) -> u16 {
    match err {
        CamelError::HttpOperationFailed { status_code, .. } => *status_code,
        other => panic!("expected HttpOperationFailed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_get_adds_the_auth_and_returns_json() {
    let h = harness().await;
    h.http.reply(
        "GET",
        &format!("{CRM_API}/v3/contacts"),
        json_reply(200, &json!({"results": [{"id": 1}]}))
            .header("x-request-id", "r1")
            .header("set-cookie", "sid=secret-cookie"),
    );
    let camel = start(&h, "san-mateo:crm?connectionId=c1&path=/v3/contacts%3Flimit%3D10").await;
    let ex = send(&camel, Message::default()).await.unwrap();
    stop(camel).await;

    let sent = h.http.requests_to(&format!("{CRM_API}/v3/contacts"));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "GET");
    assert_eq!(sent[0].url, format!("{CRM_API}/v3/contacts?limit=10"));
    assert_eq!(sent[0].header_value("authorization"), Some("Bearer at-1"));
    assert_eq!(sent[0].header_value("x-crm-version"), Some("2024-01"));
    assert!(sent[0].body.is_empty(), "an empty body sends no body");
    assert_eq!(sent[0].header_value("content-type"), None);

    let reply = ex.output.as_ref().unwrap_or(&ex.input);
    assert_eq!(reply.body, Body::Json(json!({"results": [{"id": 1}]})));
    assert_eq!(reply.header(HEADER_RESPONSE_CODE), Some(&json!(200)));
    assert_eq!(
        reply.header("SanMateoResponse-x-request-id"),
        Some(&json!("r1"))
    );
    assert_eq!(reply.header("SanMateoResponse-set-cookie"), None);
    assert!(
        !format!("{:?}", reply.headers).contains("secret-cookie"),
        "no secret reply header reaches the exchange"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headers_replace_the_options_and_proxy_headers_go_upstream() {
    let h = harness().await;
    let url = format!("{KEYED_API}/v1/items");
    h.http
        .reply("POST", &url, json_reply(201, &json!({"id": "i-1"})));
    let camel = start(&h, "san-mateo:keyed?connectionId=wrong&method=GET&path=/nope").await;
    let mut msg = Message::new(Body::Json(json!({"name": "a"})));
    msg.set_header(HEADER_CONNECTION_ID, "k1");
    msg.set_header(HEADER_METHOD, "post");
    msg.set_header(HEADER_PATH, "/v1/items?dry=true");
    msg.set_header("SanMateoProxy-X-Trace", "t-1");
    msg.set_header("SanMateoProxy-Idempotency-Key", "idem-1");
    msg.set_header("HarvestWorkflowId", "wf-1");
    let ex = send(&camel, msg).await.unwrap();
    stop(camel).await;

    assert!(h.http.requests_to(&format!("{KEYED_API}/nope")).is_empty());
    let sent = h.http.requests_to(&url);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, format!("{url}?dry=true"));
    assert_eq!(sent[0].header_value("x-api-key"), Some("key-1"));
    assert_eq!(sent[0].header_value("x-trace"), Some("t-1"));
    assert_eq!(sent[0].header_value("idempotency-key"), Some("idem-1"));
    assert_eq!(sent[0].header_value("harvestworkflowid"), None);
    assert_eq!(
        sent[0].header_value("content-type"),
        Some("application/json")
    );
    assert_eq!(sent[0].body, br#"{"name":"a"}"#);
    assert_eq!(ex.input.body, Body::Json(json!({"id": "i-1"})));
    assert_eq!(ex.input.header(HEADER_RESPONSE_CODE), Some(&json!(201)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_and_byte_bodies_go_upstream() {
    let h = harness().await;
    let url = format!("{KEYED_API}/v1/upload");
    h.http.reply(
        "PUT",
        &url,
        HttpResponse::new(200, "stored").header("content-type", "text/plain"),
    );
    let camel = start(&h, "san-mateo:keyed?connectionId=k1&method=PUT&path=/v1/upload").await;
    let mut msg = Message::new(Body::Text("héllo".to_owned()));
    msg.set_header("SanMateoProxy-Content-Type", "text/csv");
    let ex = send(&camel, msg).await.unwrap();
    assert_eq!(ex.input.body, Body::Text("stored".to_owned()));
    let ex = send(&camel, Message::new(Body::Bytes(vec![0, 159, 146, 150].into())))
        .await
        .unwrap();
    assert_eq!(ex.input.header(HEADER_RESPONSE_CODE), Some(&json!(200)));
    let mut msg = Message::new(Body::Json(json!([1, 2])));
    msg.set_header("SanMateoProxy-Content-Type", "application/vnd.api+json");
    send(&camel, msg).await.unwrap();
    stop(camel).await;

    let sent = h.http.requests_to(&url);
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].body, "héllo".as_bytes());
    assert_eq!(sent[0].header_value("content-type"), Some("text/csv"));
    assert_eq!(sent[1].body, vec![0, 159, 146, 150]);
    assert_eq!(sent[1].header_value("content-type"), None);
    assert_eq!(sent[2].body, b"[1,2]");
    assert_eq!(
        sent[2].header_value("content-type"),
        Some("application/vnd.api+json"),
        "a caller content type wins"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_over_the_limit_fails_before_the_call() {
    let h = harness().await;
    let camel = start(
        &h,
        "san-mateo:keyed?connectionId=k1&method=POST&path=/v1/items&maxBodySize=4",
    )
    .await;
    let err = send(&camel, Message::new(Body::Text("too long".to_owned())))
        .await
        .unwrap_err();
    stop(camel).await;
    assert!(matches!(err, CamelError::StreamLimitExceeded(4)), "{err:?}");
    assert!(!harvest_retryable(&err));
    assert!(h.http.requests().is_empty());

    // The component default applies when the URI has no `maxBodySize`.
    let camel = start_with(
        SanMateoComponent::new(h.san.clone())
            .environment("dev")
            .max_body_size(2),
        "san-mateo:keyed?connectionId=k1&method=POST&path=/v1/items",
    )
    .await
    .unwrap();
    let err = send(&camel, Message::new(Body::Json(json!({"a": 1}))))
        .await
        .unwrap_err();
    stop(camel).await;
    assert!(matches!(err, CamelError::StreamLimitExceeded(2)), "{err:?}");
    assert!(h.http.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_2xx_reply_fails_the_exchange_with_the_http_error() {
    let h = harness().await;
    for (path, status, retry) in [
        ("/v3/missing", 404, false),
        ("/v3/invalid", 422, false),
        ("/v3/slow", 408, true),
        ("/v3/busy", 429, true),
        ("/v3/down", 503, true),
    ] {
        h.http.reply(
            "GET",
            &format!("{CRM_API}{path}"),
            json_reply(status, &json!({"detail": "body-secret"})).header("retry-after", "0"),
        );
        let camel = start(&h, &format!("san-mateo:crm?connectionId=c1&path={path}")).await;
        let err = send(&camel, Message::default()).await.unwrap_err();
        stop(camel).await;
        assert_eq!(http_status(&err), status);
        assert_eq!(harvest_retryable(&err), retry, "{status}");
        let text = format!("{err} {err:?}");
        assert!(!text.contains("body-secret"), "{text}");
        assert!(!text.contains("at-1"), "{text}");
        assert!(text.contains(&format!("san-mateo:crm{path}")), "{text}");
        if let CamelError::HttpOperationFailed {
            method,
            response_body,
            ..
        } = &err
        {
            assert_eq!(method, "GET");
            assert_eq!(response_body, &None);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_throw_exception_on_failure_false_the_reply_is_set() {
    let h = harness().await;
    h.http.reply(
        "GET",
        &format!("{CRM_API}/v3/missing"),
        json_reply(404, &json!({"error": "not found"})),
    );
    let camel = start(
        &h,
        "san-mateo:crm?connectionId=c1&path=/v3/missing&throwExceptionOnFailure=false",
    )
    .await;
    let ex = send(&camel, Message::default()).await.unwrap();
    stop(camel).await;
    assert_eq!(ex.input.header(HEADER_RESPONSE_CODE), Some(&json!(404)));
    assert_eq!(ex.input.body, Body::Json(json!({"error": "not found"})));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_and_retry_on_reach_the_proxy() {
    let h = harness().await;
    let url = format!("{CRM_API}/v3/deals");
    h.http.reply("POST", &url, HttpResponse::new(503, ""));
    h.http.reply("POST", &url, HttpResponse::new(503, ""));
    h.http.reply("POST", &url, json_reply(201, &json!({"id": 9})));
    let camel = start(
        &h,
        "san-mateo:crm?connectionId=c1&method=POST&path=/v3/deals&retries=2&retryOn=500,503",
    )
    .await;
    let ex = send(&camel, Message::new(Body::Json(json!({"n": 1}))))
        .await
        .unwrap();
    stop(camel).await;
    assert_eq!(ex.input.body, Body::Json(json!({"id": 9})));
    assert_eq!(h.http.requests_to(&url).len(), 3);

    // With no `retryOn`, a POST does not retry a 503.
    let url = format!("{CRM_API}/v3/notes");
    h.http.reply("POST", &url, HttpResponse::new(503, ""));
    let camel = start(
        &h,
        "san-mateo:crm?connectionId=c1&method=POST&path=/v3/notes&retries=2",
    )
    .await;
    let err = send(&camel, Message::default()).await.unwrap_err();
    stop(camel).await;
    assert_eq!(http_status(&err), 503);
    assert_eq!(h.http.requests_to(&url).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_401_refreshes_the_token_and_sends_again() {
    let h = harness().await;
    let url = format!("{CRM_API}/v3/me");
    h.http.reply("GET", &url, HttpResponse::new(401, "expired"));
    h.http.reply("GET", &url, json_reply(200, &json!({"me": true})));
    h.http.reply(
        "POST",
        TOKEN_URL,
        json_reply(
            200,
            &json!({"access_token": "at-2", "refresh_token": "rt-2", "expires_in": 3600}),
        ),
    );
    let camel = start(&h, "san-mateo:crm?connectionId=c1&path=/v3/me").await;
    let ex = send(&camel, Message::default()).await.unwrap();
    stop(camel).await;
    assert_eq!(ex.input.body, Body::Json(json!({"me": true})));
    let sent = h.http.requests_to(&url);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].header_value("authorization"), Some("Bearer at-1"));
    assert_eq!(sent[1].header_value("authorization"), Some("Bearer at-2"));
    assert_eq!(h.http.requests_to(TOKEN_URL).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_environment_option_selects_the_environment() {
    let h = harness().await;
    let url = format!("{CRM_API}/v3/contacts");
    h.http.reply("GET", &url, json_reply(200, &json!([])));
    let camel = start(
        &h,
        "san-mateo:crm?connectionId=c1&path=/v3/contacts&environment=prod",
    )
    .await;
    send(&camel, Message::default()).await.unwrap();
    stop(camel).await;
    let sent = h.http.requests_to(&url);
    assert_eq!(sent[0].header_value("authorization"), Some("Bearer at-prod"));

    // A component with no default environment uses the URI option.
    let camel = start_with(
        SanMateoComponent::new(h.san.clone()),
        "san-mateo:crm?connectionId=c1&path=/v3/contacts&environment=dev",
    )
    .await
    .unwrap();
    send(&camel, Message::default()).await.unwrap();
    stop(camel).await;
    let sent = h.http.requests_to(&url);
    assert_eq!(sent[1].header_value("authorization"), Some("Bearer at-1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_connection_is_a_non_retryable_config_error() {
    let h = harness().await;
    let camel = start(&h, "san-mateo:crm?connectionId=nobody&path=/v3/contacts").await;
    let err = send(&camel, Message::default()).await.unwrap_err();
    stop(camel).await;
    assert!(matches!(err, CamelError::Config(_)), "{err:?}");
    assert!(err.to_string().contains("unknown_connection"), "{err}");
    assert!(!harvest_retryable(&err));
    assert!(h.http.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forbidden_base_url_override_is_non_retryable() {
    let h = harness().await;
    let camel = start(
        &h,
        "san-mateo:crm?connectionId=c1&path=/v3/contacts&baseUrlOverride=https://evil.example.net",
    )
    .await;
    let err = send(&camel, Message::default()).await.unwrap_err();
    stop(camel).await;
    assert!(matches!(err, CamelError::Unauthorized(_)), "{err:?}");
    assert!(err.to_string().contains("forbidden_host"), "{err}");
    assert!(!harvest_retryable(&err));
    assert!(h.http.requests().is_empty(), "no token goes to the bad host");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_network_failure_is_retryable() {
    let h = harness().await;
    let url = format!("{CRM_API}/v3/contacts");
    h.http
        .fail("GET", &url, TransportError::Connect("refused".to_owned()));
    let camel = start(&h, "san-mateo:crm?connectionId=c1&path=/v3/contacts").await;
    let err = send(&camel, Message::default()).await.unwrap_err();
    stop(camel).await;
    assert!(matches!(err, CamelError::Io(_)), "{err:?}");
    assert!(err.to_string().contains("upstream_error"), "{err}");
    assert!(harvest_retryable(&err));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_connection_id_or_path_fails_validation() {
    let h = harness().await;
    let camel = start(&h, "san-mateo:crm").await;
    let err = send(&camel, Message::default()).await.unwrap_err();
    assert!(matches!(err, CamelError::ValidationError(_)), "{err:?}");
    assert!(err.to_string().contains(HEADER_CONNECTION_ID), "{err}");
    let mut msg = Message::default();
    msg.set_header(HEADER_CONNECTION_ID, "c1");
    let err = send(&camel, msg).await.unwrap_err();
    assert!(matches!(err, CamelError::ValidationError(_)), "{err:?}");
    assert!(err.to_string().contains(HEADER_PATH), "{err}");
    let mut msg = Message::default();
    msg.set_header(HEADER_CONNECTION_ID, "c1");
    msg.set_header(HEADER_PATH, "/v3/contacts");
    msg.set_header("SanMateoProxy-X-Bad", "a\r\nb");
    let err = send(&camel, msg).await.unwrap_err();
    assert!(matches!(err, CamelError::ValidationError(_)), "{err:?}");
    stop(camel).await;
    assert!(!harvest_retryable(&err));
    assert!(h.http.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_uris_fail_when_the_route_is_added() {
    let h = harness().await;
    let fails = |component: SanMateoComponent, uri: &'static str, needle: &'static str| async move {
        let err = start_with(component, uri)
            .await
            .err()
            .unwrap_or_else(|| panic!("{uri} must fail"));
        let text = err.to_string();
        assert!(text.contains(needle), "{uri}: {text}");
        assert!(!harvest_retryable(&err), "{uri}: {err:?}");
    };
    let dev = || SanMateoComponent::new(h.san.clone()).environment("dev");
    fails(dev(), "san-mateo:crm?connectionId=c1&colour=red", "colour").await;
    fails(dev(), "san-mateo:?connectionId=c1", "integration").await;
    fails(dev(), "san-mateo:crm?method=TRACE", "method").await;
    fails(dev(), "san-mateo:crm?retries=many", "retries").await;
    fails(dev(), "san-mateo:crm?environment=staging", "staging").await;
    fails(
        SanMateoComponent::new(h.san.clone()),
        "san-mateo:crm?connectionId=c1",
        "environment",
    )
    .await;
    fails(
        SanMateoComponent::new(h.san.clone()).environment("qa"),
        "san-mateo:crm?connectionId=c1",
        "qa",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_endpoint_options_are_checked_without_a_route() {
    let h = harness().await;
    let component = SanMateoComponent::new(h.san.clone()).environment("dev");
    let o = component
        .endpoint_options("san-mateo:crm?connectionId=c1&retries=3")
        .unwrap();
    assert_eq!(o.integration, "crm");
    assert_eq!(o.environment, "dev");
    assert_eq!(o.retries, 3);
    assert!(matches!(
        component.endpoint_options("san-mateo:crm?environment=nope"),
        Err(CamelError::Config(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_metadata_lists_the_uri_options() {
    let h = harness().await;
    let camel = start(&h, "san-mateo:crm?connectionId=c1&path=/x").await;
    let meta = camel.component_metadata(SCHEME).unwrap();
    stop(camel).await;
    assert_eq!(meta.scheme, "san-mateo");
    assert_eq!(meta.version, env!("CARGO_PKG_VERSION"));
    assert!(!meta.description.is_empty());
    assert_eq!(meta.uri_syntax, "san-mateo:integration");
    assert!(meta.capabilities.supports_producer);
    assert!(!meta.capabilities.supports_consumer);
    assert!(!meta.capabilities.supports_polling_consumer);
    let names: Vec<&str> = meta.uri_options.iter().map(|o| o.name.as_str()).collect();
    for name in [
        "connectionId",
        "environment",
        "method",
        "path",
        "retries",
        "retryOn",
        "baseUrlOverride",
        "throwExceptionOnFailure",
        "maxBodySize",
    ] {
        assert!(names.contains(&name), "{name}");
    }
    assert!(meta.validate_scheme(SCHEME).is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_component_is_producer_only() {
    let h = harness().await;
    let component = SanMateoComponent::new(h.san.clone()).environment("dev");
    assert_eq!(component.scheme(), SCHEME);
    let endpoint = component
        .create_endpoint("san-mateo:crm?connectionId=c1", &NoOpComponentContext)
        .unwrap();
    assert_eq!(endpoint.uri(), "san-mateo:crm?connectionId=c1");
    let rt: Arc<dyn RuntimeObservability> = Arc::new(NoOpComponentContext);
    let err = endpoint.create_consumer(rt).err().unwrap();
    assert!(matches!(err, CamelError::EndpointCreationFailed(_)), "{err:?}");
    assert!(err.to_string().contains("producer"), "{err}");
    let debug = format!("{component:?}");
    assert!(!debug.contains("sk_test"), "{debug}");
}
