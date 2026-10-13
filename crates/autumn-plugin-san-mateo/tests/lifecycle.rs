//! Startup checks, health, background loops and shutdown.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use autumn_plugin_san_mateo::san_mateo::transport::{
    HttpRequest, HttpResponse, HttpTransport, TransportError,
};
use autumn_plugin_san_mateo::san_mateo::{MemoryTransport, Store};
use autumn_plugin_san_mateo::{INDICATOR_NAME, PluginError, SanMateoPlugin};
use autumn_web::actuator::{HealthIndicator, HealthStatus};
use autumn_web::reexports::axum::body::Body;
use autumn_web::reexports::http::Request;
use autumn_web::test::TestApp;
use autumn_web::{AppState, ProcessRole};
use common::*;
use serde_json::json;
use tower::ServiceExt as _;

fn start_err(p: SanMateoPlugin) -> String {
    p.start(&AppState::for_test()).unwrap_err().to_string()
}

#[test]
fn a_missing_secret_key_fails_start_with_the_env_var_name() {
    let p = SanMateoPlugin::with_config(config())
        .secret_env(DEV_SECRET_ENV, DEV_SECRET)
        .secret_env(KEY_ENV, KEY_B64)
        .secret_env(WHSEC_ENV, WHSEC);
    let e = start_err(p);
    assert!(e.contains(PROD_SECRET_ENV) && e.contains("prod"), "{e}");
}

#[test]
fn a_missing_or_bad_encryption_key_fails_start_without_the_value() {
    let p = SanMateoPlugin::with_config(config())
        .secret_env(DEV_SECRET_ENV, DEV_SECRET)
        .secret_env(PROD_SECRET_ENV, PROD_SECRET)
        .secret_env(WHSEC_ENV, WHSEC);
    let e = start_err(p);
    assert!(e.contains(KEY_ENV), "{e}");
    let p = plugin(&network()).secret_env(KEY_ENV, "c2hvcnQtc2VjcmV0LWtleQ==");
    let e = start_err(p);
    assert!(e.contains(KEY_ENV) && !e.contains("c2hvcnQtc2VjcmV0LWtleQ=="), "{e}");
}

#[test]
fn a_missing_webhook_secret_fails_start() {
    let p = SanMateoPlugin::with_config(config())
        .secret_env(DEV_SECRET_ENV, DEV_SECRET)
        .secret_env(PROD_SECRET_ENV, PROD_SECRET)
        .secret_env(KEY_ENV, KEY_B64);
    let e = start_err(p);
    assert!(e.contains(WHSEC_ENV), "{e}");
}

#[test]
fn invalid_config_and_service_errors_fail_start() {
    let mut c = config();
    c.background.tick_secs = 0;
    let e = plugin_with(&network(), c).start(&AppState::for_test()).unwrap_err();
    assert!(matches!(e, PluginError::Config(_)), "{e:?}");
    // Two environments with the same secret: the core rejects the build.
    let e = plugin(&network())
        .secret_env(PROD_SECRET_ENV, DEV_SECRET)
        .start(&AppState::for_test())
        .unwrap_err();
    assert!(matches!(e, PluginError::Service(_)), "{e:?}");
    assert!(!e.to_string().contains(DEV_SECRET), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "SM_PLUGIN_TEST_PROD_SK")]
async fn a_missing_secret_stops_the_app_boot() {
    let p = SanMateoPlugin::with_config(config())
        .secret_env(DEV_SECRET_ENV, DEV_SECRET)
        .secret_env(KEY_ENV, KEY_B64)
        .secret_env(WHSEC_ENV, WHSEC);
    let _client = TestApp::new().plugin(p).build();
}

#[tokio::test(flavor = "multi_thread")]
async fn actuator_health_shows_san_mateo_without_secrets() {
    let client = TestApp::new().plugin(plugin(&network())).build();
    let body = client.get("/actuator/health").send().await.text();
    assert!(body.contains(INDICATOR_NAME), "{body}");
    assert!(body.contains("UP"), "{body}");
    assert!(!body.contains(DEV_SECRET) && !body.contains(KEY_B64), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn health_and_routes_follow_the_runtime() {
    let rt = plugin(&network()).start(&AppState::for_test()).unwrap();
    assert!(format!("{rt:?}").contains("SanMateoRuntime"));
    let health = rt.health();
    let out = health.check().await;
    assert_eq!(out.status, HealthStatus::Up);
    assert_eq!(out.details["environments"], json!(2));
    assert_eq!(out.details["providers"], json!(2));
    let req = || Request::get("/health").body(Body::empty()).unwrap();
    let res = rt.router().oneshot(req()).await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    rt.shutdown().await;
    assert_eq!(health.check().await.status, HealthStatus::Down);
    let res = rt.router().oneshot(req()).await.unwrap();
    assert_eq!(res.status().as_u16(), 503);
}

#[tokio::test(flavor = "multi_thread")]
async fn background_loops_follow_the_config_the_builder_and_the_role() {
    let mut on = config();
    on.background.enabled = true;
    let state = AppState::for_test();
    let rt = plugin_with(&network(), on.clone())
        .start_with_role(&state, ProcessRole::Combined)
        .unwrap();
    assert!(rt.background_running());
    rt.shutdown().await;
    assert!(!rt.background_running());
    // A web process runs no workers, so no loops.
    let rt = plugin_with(&network(), on.clone())
        .start_with_role(&AppState::for_test(), ProcessRole::Web)
        .unwrap();
    assert!(!rt.background_running());
    // The builder setting wins over the config.
    let rt = plugin_with(&network(), on)
        .background(false)
        .start_with_role(&AppState::for_test(), ProcessRole::Worker)
        .unwrap();
    assert!(!rt.background_running());
    let rt = plugin(&network())
        .background(true)
        .start_with_role(&AppState::for_test(), ProcessRole::Worker)
        .unwrap();
    assert!(rt.background_running());
    rt.shutdown().await;
}

/// Replies after a delay, so a webhook is in flight at shutdown.
struct Slow(MemoryTransport);

#[async_trait]
impl HttpTransport for Slow {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.0.send(request).await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_waits_for_webhooks_in_flight() {
    let http = network();
    let store = Arc::new(autumn_plugin_san_mateo::san_mateo::MemoryStore::new());
    let rt = plugin(&http)
        .transport(Arc::new(Slow(http.clone())))
        .store(Arc::clone(&store) as Arc<dyn Store>)
        .start(&AppState::for_test())
        .unwrap();
    let san = rt.service();
    san.create_integration(
        "dev",
        autumn_plugin_san_mateo::san_mateo::NewIntegration {
            unique_key: "open".to_owned(),
            provider: "open".to_owned(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let token = san
        .create_connect_session("dev", Default::default())
        .await
        .unwrap()
        .token;
    san.connect_unauthenticated(token.expose(), "open", Default::default())
        .await
        .unwrap();
    assert!(http.requests_to(HOOK).is_empty(), "the webhook is in flight");
    rt.shutdown().await;
    assert_eq!(http.requests_to(HOOK).len(), 1);
}
