//! Install with one call, the mounted API, and the declared routes (AC20).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use autumn_plugin_san_mateo::san_mateo::api::ROUTES;
use autumn_plugin_san_mateo::san_mateo::transport::HttpResponse;
use autumn_plugin_san_mateo::san_mateo::{EnvironmentConfig, KeyRing, MemoryTransport, SanMateo};
use autumn_plugin_san_mateo::{
    DEFAULT_PREFIX, PLUGIN_NAME, SanMateoPlugin, SanMateoService, autumn_error,
};
use autumn_web::plugin::Plugin;
use autumn_web::reexports::axum::Json;
use autumn_web::reexports::axum::routing::get;
use autumn_web::route_listing::{RouteClassification, RouteSource};
use autumn_web::test::TestApp;
use autumn_web::{AppState, AutumnResult};
use common::*;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn plugin_installs_with_one_call() {
    let http = network();
    let client = app(plugin(&http));
    // The core API answers under the default prefix.
    let r = client.get("/san-mateo/health").send().await;
    r.assert_status(200);
    assert_eq!(json(&r)["status"], "ok");
    // The API needs the secret key.
    let r = client
        .get("/san-mateo/providers")
        .header("accept", "application/json")
        .send()
        .await;
    r.assert_status(401);
    assert_eq!(json(&r)["error"]["code"], "unauthorized");
    let r = api(&client, "GET", "/san-mateo/providers", None).await;
    r.assert_status(200);
    let names: Vec<String> = json(&r)["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["keyed", "open"]);
    // The prod key selects the prod environment.
    let r = client
        .get("/san-mateo/integrations")
        .header("authorization", &bearer(PROD_SECRET))
        .send()
        .await;
    r.assert_status(200);
    // The service is in the app state.
    let san = SanMateoService::from_state(client.state()).unwrap();
    assert_eq!(san.config().server_url, "https://app.example.com/san-mateo");
    assert_eq!(san.config().environments.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_and_proxy_through_the_autumn_stack() {
    let http = network();
    let client = app(plugin(&http));
    integrations(&client, "/san-mateo").await;
    let token = session(&client, "/san-mateo").await;
    // The connect link uses the public URL and the prefix.
    let r = client
        .post(&format!(
            "/san-mateo/api-auth/api-key/keyed?connect_session_token={token}"
        ))
        .json(&json!({"apiKey": "user-api-key"}))
        .send()
        .await;
    r.assert_status(200);
    let connection_id = json(&r)["connectionId"].as_str().unwrap().to_owned();
    http.reply(
        "POST",
        "https://api.keyed.example.com/v1/notes",
        HttpResponse::new(201, r#"{"id":"n1"}"#).header("content-type", "application/json"),
    );
    let r = client
        .post("/san-mateo/proxy/v1/notes?draft=true")
        .header("authorization", &bearer(DEV_SECRET))
        .header("connection-id", &connection_id)
        .header("provider-config-key", "keyed")
        .json(&json!({"text": "hi"}))
        .send()
        .await;
    r.assert_status(201);
    assert_eq!(json(&r)["id"], "n1");
    let sent = &http.requests_to("https://api.keyed.example.com/v1/notes")[0];
    assert_eq!(sent.url, "https://api.keyed.example.com/v1/notes?draft=true");
    assert_eq!(sent.header_value("x-api-key"), Some("user-api-key"));
    assert_eq!(sent.body, br#"{"text":"hi"}"#);
    // A path parameter reaches the core once.
    let r = api(
        &client,
        "GET",
        &format!("/san-mateo/connections/{connection_id}?provider_config_key=keyed"),
        None,
    )
    .await;
    r.assert_status(200);
    assert_eq!(json(&r)["credentials"]["apiKey"], "user-api-key");
    let r = api(
        &client,
        "DELETE",
        &format!("/san-mateo/connections/{connection_id}?provider_config_key=keyed"),
        None,
    )
    .await;
    r.assert_status(204);
    // The `auth` webhooks are signed with the secret from the named env var.
    let san = SanMateoService::require(client.state()).unwrap();
    san.flush().await;
    let hooks = http.requests_to(HOOK);
    assert_eq!(hooks.len(), 2, "creation and deletion");
    assert!(hooks[0].header_value("x-san-mateo-hmac-sha256").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_session_links_use_the_prefix() {
    let http = network();
    let client = app(plugin(&http).prefix("/integrations-api/"));
    let r = api(
        &client,
        "POST",
        "/integrations-api/connect/sessions",
        Some(json!({"end_user": {"id": "u1"}})),
    )
    .await;
    r.assert_status(201);
    let link = json(&r)["data"]["connect_link"].as_str().unwrap().to_owned();
    assert!(
        link.starts_with("https://app.example.com/integrations-api/connect?session_token=smcs_"),
        "{link}"
    );
    // The default prefix is not mounted.
    client.get("/san-mateo/health").send().await.assert_status(404);
}

#[tokio::test(flavor = "multi_thread")]
async fn root_prefix_mounts_at_the_root() {
    let http = network();
    let client = app(plugin(&http).prefix("/"));
    let r = api(&client, "GET", "/providers", None).await;
    r.assert_status(200);
    // autumn owns `/health` at the root, so the plugin does not declare it.
    client.get("/health").send().await.assert_status(200);
    let declared = autumn_web::app()
        .plugin(SanMateoPlugin::new().prefix("/"))
        .plugin_route_infos()
        .unwrap();
    assert!(declared.iter().any(|r| r.path == "/providers"));
    assert!(!declared.iter().any(|r| r.path == "/health"));
}

#[test]
fn plugin_name_contract_and_config_section() {
    let p = SanMateoPlugin::new();
    assert_eq!(p.name(), PLUGIN_NAME);
    assert_eq!(p.route_prefix(), DEFAULT_PREFIX);
    let contract = p.contract().unwrap();
    assert!(format!("{contract:?}").contains("0.8"), "{contract:?}");
    let app = autumn_web::app().plugin(SanMateoPlugin::default());
    assert!(app.has_plugin(PLUGIN_NAME));
    assert!(app.has_config_section("san_mateo"));
}

#[test]
fn plugin_declares_all_routes_under_the_prefix() {
    let app = autumn_web::app().plugin(SanMateoPlugin::new().prefix("/api/sm"));
    let routes = app.plugin_route_infos().unwrap();
    let ours: Vec<_> = routes
        .iter()
        .filter(|r| r.source == RouteSource::Plugin(PLUGIN_NAME.to_owned()))
        .collect();
    assert_eq!(ours.len(), ROUTES.len());
    for spec in ROUTES {
        let path = format!("/api/sm{}", spec.path);
        let r = ours
            .iter()
            .find(|r| r.path == path && r.method == spec.method)
            .unwrap_or_else(|| panic!("no route {} {path}", spec.method));
        let expected = if spec.secret_key {
            RouteClassification::Gated
        } else {
            RouteClassification::Public
        };
        assert_eq!(r.classification, expected, "{path}");
    }
}

/// A user handler gets the service as an extractor.
async fn environments(san: SanMateoService) -> AutumnResult<Json<Value>> {
    let env = san.environment("prod").map_err(autumn_error)?;
    Ok(Json(json!({"id": env.id, "count": san.config().environments.len()})))
}

async fn missing(san: SanMateoService) -> AutumnResult<Json<Value>> {
    san.environment("nope").map_err(autumn_error)?;
    Ok(Json(json!({})))
}

#[tokio::test(flavor = "multi_thread")]
async fn handlers_and_jobs_get_the_service() {
    let http = network();
    let seen = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&seen);
    let client = TestApp::new()
        .plugin(plugin(&http))
        // autumn runs this after the plugin and before job workers start.
        .state_initializer(move |state: &AppState| {
            flag.store(SanMateoService::from_state(state).is_some(), Ordering::SeqCst);
        })
        .merge(
            autumn_web::reexports::axum::Router::new()
                .route("/app/environments", get(environments))
                .route("/app/missing", get(missing)),
        )
        .build();
    assert!(seen.load(Ordering::SeqCst), "no service before job workers");
    let r = client
        .get("/app/environments")
        .header("accept", "application/json")
        .send()
        .await;
    r.assert_status(200);
    assert_eq!(json(&r), json!({"id": "prod", "count": 2}));
    let r = client
        .get("/app/missing")
        .header("accept", "application/json")
        .send()
        .await;
    r.assert_status(400);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prebuilt_service_is_used_as_is() {
    let http = MemoryTransport::new();
    let san = SanMateo::builder()
        .server_url("https://own.example.com/sm")
        .environment(EnvironmentConfig::new("own", "sk_test_own_0123456789"))
        .key_ring(KeyRing::new("k9", [9; 32]).unwrap())
        .transport(Arc::new(http))
        .build()
        .unwrap();
    // No secrets are set: the plugin reads none.
    let client = app(SanMateoPlugin::with_config(config()).service(san));
    let r = client
        .get("/san-mateo/integrations")
        .header("authorization", &bearer("sk_test_own_0123456789"))
        .send()
        .await;
    r.assert_status(200);
    let s = SanMateoService::require(client.state()).unwrap();
    assert_eq!(s.config().server_url, "https://own.example.com/sm");
}
