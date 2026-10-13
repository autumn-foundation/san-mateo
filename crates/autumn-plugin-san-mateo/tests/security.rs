//! autumn CSRF, CAPTCHA and CSP with the San Mateo routes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use autumn_plugin_san_mateo::PluginError;
use autumn_web::AppState;
use autumn_web::config::AutumnConfig;
use autumn_web::test::TestApp;
use common::*;
use serde_json::json;

fn state_with(cfg: AutumnConfig) -> AppState {
    let s = AppState::for_test();
    s.insert_extension(cfg);
    s
}

fn csrf_on() -> AutumnConfig {
    let mut cfg = AutumnConfig::default();
    cfg.security.csrf.enabled = true;
    cfg
}

#[test]
fn csrf_without_an_exemption_fails_start_with_the_fix() {
    let e = plugin(&network())
        .start(&state_with(csrf_on()))
        .unwrap_err();
    assert!(matches!(e, PluginError::Config(_)));
    let msg = e.to_string();
    assert!(msg.contains("[security.csrf] exempt_paths"), "{msg}");
    assert!(msg.contains("[\"/san-mateo\"]"), "{msg}");
    // A worker serves no San Mateo route, so it needs no exemption.
    plugin(&network())
        .start_with_role(&state_with(csrf_on()), autumn_web::ProcessRole::Worker)
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "exempt_paths")]
async fn csrf_without_an_exemption_stops_the_app_boot() {
    let _client = TestApp::new()
        .config(csrf_on())
        .plugin(plugin(&network()))
        .build();
}

#[tokio::test(flavor = "multi_thread")]
async fn csrf_with_the_exemption_lets_bearer_calls_through() {
    let mut cfg = csrf_on();
    cfg.security.csrf.exempt_paths = vec!["/san-mateo".to_owned()];
    let client = TestApp::new()
        .config(cfg)
        .plugin(plugin(&network()))
        .build();
    let r = api(
        &client,
        "POST",
        "/san-mateo/integrations",
        Some(json!({"unique_key": "keyed", "provider": "keyed"})),
    )
    .await;
    r.assert_status(200);
    let token = session(&client, "/san-mateo").await;
    let r = client
        .post(&format!(
            "/san-mateo/api-auth/api-key/keyed?connect_session_token={token}"
        ))
        .json(&json!({"apiKey": "k"}))
        .send()
        .await;
    r.assert_status(200);
}

#[test]
fn captcha_without_an_exemption_fails_start() {
    let mut cfg = AutumnConfig::default();
    cfg.bot_protection.enabled = true;
    let e = plugin(&network())
        .start(&state_with(cfg.clone()))
        .unwrap_err();
    assert!(e.to_string().contains("captcha_exempt_paths"), "{e}");
    cfg.security.captcha_exempt_paths = vec!["/san-mateo".to_owned()];
    plugin(&network()).start(&state_with(cfg)).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hosted_connect_page_runs_under_the_autumn_csp() {
    let mut cfg = AutumnConfig::default();
    cfg.security.headers.csp_nonce.enabled = true;
    let client = TestApp::new()
        .config(cfg)
        .plugin(plugin(&network()))
        .build();
    integrations(&client, "/san-mateo").await;
    let token = session(&client, "/san-mateo").await;
    let r = client
        .get(&format!("/san-mateo/connect?session_token={token}"))
        .send()
        .await;
    r.assert_status(200);
    let csp = r.header("content-security-policy").unwrap().to_owned();
    let nonce = csp
        .split("'nonce-")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .unwrap()
        .to_owned();
    let html = r.text();
    assert!(
        html.contains(&format!("<script nonce=\"{nonce}\">")),
        "{csp}\n{html}"
    );
    assert!(html.contains("/san-mateo/api-auth/api-key/keyed"), "{html}");
}
