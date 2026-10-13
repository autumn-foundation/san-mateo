//! The REST API over HTTP (AC1, AC3, AC4, AC8, AC10, AC14, AC15, AC17, AC23).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use san_mateo::transport::HttpResponse;
use san_mateo::{
    ActionContext, ActionFunction, ActionSpec, Registry, Result, SyncContext, SyncFunction,
    SyncSpec,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&self.body)))
    }
    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

async fn call(
    h: &Harness,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Reply {
    let app = san_mateo::api::router(h.san.clone());
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(b) => req
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

const AUTH: (&str, &str) = ("authorization", "Bearer sk_test_dev_0123456789");

#[tokio::test]
async fn health_needs_no_auth_and_other_routes_do() {
    let h = harness();
    let r = call(&h, "GET", "/health", &[], None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["status"], "ok");
    for (m, p) in [
        ("GET", "/providers"),
        ("GET", "/integrations"),
        ("GET", "/connections"),
        ("GET", "/proxy/x"),
        ("POST", "/connect/sessions"),
        ("GET", "/records?model=M"),
    ] {
        let r = call(&h, m, p, &[], None).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{m} {p}");
        assert_eq!(r.json()["error"]["code"], "unauthorized");
        let r = call(&h, m, p, &[("authorization", "Bearer wrong")], None).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{m} {p}");
    }
}

#[tokio::test]
async fn providers() {
    let h = harness();
    let r = call(&h, "GET", "/providers", &[AUTH], None).await;
    assert_eq!(r.status, StatusCode::OK);
    let list = r.json()["data"].as_array().unwrap().clone();
    assert_eq!(list.len(), 7);
    let crm = list.iter().find(|p| p["name"] == "crm").unwrap();
    assert_eq!(crm["auth_mode"], "OAUTH2");
    assert_eq!(crm["display_name"], "Test CRM");
    let r = call(&h, "GET", "/providers/crm", &[AUTH], None).await;
    assert_eq!(
        r.json()["data"]["proxy"]["base_url"],
        "https://api.crm.example.com"
    );
    let r = call(&h, "GET", "/providers/nope", &[AUTH], None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.json()["error"]["code"], "unknown_provider");
}

#[tokio::test]
async fn integrations_over_http() {
    let h = harness();
    let body = json!({"unique_key": "gh", "provider": "crm", "display_name": "CRM", "credentials": {"type": "OAUTH2", "client_id": "cid", "client_secret": "csecret", "scopes": "read,write"}});
    let r = call(&h, "POST", "/integrations", &[AUTH], Some(body.clone())).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let d = r.json()["data"].clone();
    assert_eq!(d["unique_key"], "gh");
    assert_eq!(d["provider"], "crm");
    assert_eq!(d["display_name"], "CRM");
    assert!(d["created_at"].is_string());
    assert!(!r.text().contains("csecret"));
    assert_eq!(
        call(&h, "POST", "/integrations", &[AUTH], Some(body))
            .await
            .status,
        StatusCode::CONFLICT
    );

    let r = call(
        &h,
        "GET",
        "/integrations/gh?include=credentials",
        &[AUTH],
        None,
    )
    .await;
    let d = r.json()["data"].clone();
    assert_eq!(d["credentials"]["client_id"], "cid");
    assert_eq!(d["credentials"]["scopes"], "read,write");
    assert!(d["credentials"].get("client_secret").is_none());
    let r = call(&h, "PATCH", "/integrations/gh", &[AUTH], Some(json!({"display_name": "New", "credentials": {"type": "OAUTH2", "client_id": "cid2", "client_secret": "s2", "scopes": "a"}}))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["data"]["display_name"], "New");
    let r = call(&h, "GET", "/integrations", &[AUTH], None).await;
    assert_eq!(r.json()["data"].as_array().unwrap().len(), 1);
    let r = call(&h, "DELETE", "/integrations/gh", &[AUTH], None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["success"], true);
    assert_eq!(
        call(&h, "GET", "/integrations/gh", &[AUTH], None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let r = call(
        &h,
        "POST",
        "/integrations",
        &[AUTH],
        Some(json!({"unique_key": 5})),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"]["code"], "invalid_body");
}

async fn session_token(h: &Harness, body: Value) -> String {
    let r = call(h, "POST", "/connect/sessions", &[AUTH], Some(body)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let d = r.json()["data"].clone();
    assert!(
        d["connect_link"]
            .as_str()
            .unwrap()
            .starts_with("https://sm.example.com/connect?session_token=smcs_")
    );
    assert!(d["expires_at"].is_string());
    d["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn connect_sessions_over_http() {
    let h = ready().await;
    let token = session_token(
        &h,
        json!({
            "end_user": {"id": "u1", "email": "u1@example.com", "display_name": "U One", "tags": {"a": "b"}},
            "organization": {"id": "o1", "display_name": "Org"},
            "allowed_integrations": ["crm", "keyed"],
            "integrations_config_defaults": {"crm": {"user_scopes": "read", "authorization_params": {"prompt": "none"}, "connection_config": {"x": "y"}}},
            "tags": {"plan": "pro"}
        }),
    )
    .await;
    let bearer = format!("Bearer {token}");
    let r = call(
        &h,
        "GET",
        "/connect/session",
        &[("authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let d = r.json()["data"].clone();
    assert_eq!(d["allowed_integrations"], json!(["crm", "keyed"]));
    assert_eq!(d["endUser"]["id"], "u1");
    assert_eq!(d["endUser"]["organization"]["id"], "o1");
    assert_eq!(
        d["integrations_config_defaults"]["crm"]["user_scopes"],
        "read"
    );
    assert_eq!(d["isReconnecting"], false);
    // The secret key is not a session token.
    assert_eq!(
        call(&h, "GET", "/connect/session", &[AUTH], None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &h,
            "DELETE",
            "/connect/session",
            &[("authorization", &bearer)],
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    let r = call(
        &h,
        "GET",
        "/connect/session",
        &[("authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json()["error"]["code"], "invalid_connect_session");
}

#[tokio::test]
async fn oauth_over_http() {
    let h = ready().await;
    let token = session_token(&h, json!({"end_user": {"id": "u1"}})).await;
    let r = call(&h, "GET", &format!("/oauth/connect/crm?connect_session_token={token}&user_scope=read&authorization_params[prompt]=login"), &[], None).await;
    assert_eq!(r.status, StatusCode::FOUND, "{}", r.text());
    let location = r.header("location").unwrap().to_owned();
    let q = query_of(&location);
    // As in Nango: with a connect session, the end user cannot change scopes.
    assert_eq!(q["scope"], "read,write");
    assert!(!q.contains_key("user_scope"));
    assert_eq!(q["prompt"], "login");
    h.http.reply(
        "POST",
        "https://auth.crm.example.com/token",
        token_reply("at-1", Some("rt-1"), Some(3_600)),
    );
    let r = call(
        &h,
        "GET",
        &format!("/oauth/callback?code=c1&state={}", q["state"]),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    let html = r.text();
    assert!(
        html.contains("postMessage") && html.contains("connectionId"),
        "{html}"
    );
    assert!(r.header("content-security-policy").is_some());
    let r = call(
        &h,
        "GET",
        &format!("/oauth/callback?code=c1&state={}", q["state"]),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("invalid_oauth_state"));
    let r = call(
        &h,
        "GET",
        "/oauth/connect/crm?connect_session_token=smcs_bad",
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // Connection config from `params[...]`.
    let r = call(
        &h,
        "GET",
        &format!("/oauth/connect/zen?connect_session_token={token}&params[subdomain]=acme"),
        &[],
        None,
    )
    .await;
    assert!(
        r.header("location")
            .unwrap()
            .starts_with("https://acme.zen.example.com/")
    );
}

#[tokio::test]
async fn other_auth_modes_over_http() {
    let h = ready().await;
    let token = session_token(&h, json!({"end_user": {"id": "u1"}})).await;
    h.http.reply(
        "GET",
        "https://api.keyed.example.com/v1/me",
        HttpResponse::new(200, "{}"),
    );
    let r = call(
        &h,
        "POST",
        &format!("/api-auth/api-key/keyed?connect_session_token={token}"),
        &[],
        Some(json!({"apiKey": "k1"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["providerConfigKey"], "keyed");
    assert!(r.json()["connectionId"].is_string());
    let bearer = format!("Bearer {token}");
    let r = call(
        &h,
        "POST",
        "/api-auth/basic/basic",
        &[("authorization", &bearer)],
        Some(json!({"username": "u", "password": "p"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    h.http.reply(
        "POST",
        "https://auth.machine.example.com/token",
        HttpResponse::json(200, &json!({"access_token": "cc", "expires_in": 60})),
    );
    let r = call(
        &h,
        "POST",
        &format!("/oauth2/auth/machine?connect_session_token={token}"),
        &[],
        Some(json!({"client_id": "i", "client_secret": "s"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let r = call(
        &h,
        "POST",
        &format!("/auth/unauthenticated/open?connect_session_token={token}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let r = call(
        &h,
        "POST",
        &format!("/api-auth/api-key/keyed?connect_session_token={token}"),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = call(&h, "GET", "/connections", &[AUTH], None).await;
    assert_eq!(r.json()["connections"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn connections_over_http() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    let id = conn.connection_id.clone();
    let r = call(
        &h,
        "GET",
        "/connections?integrationId=crm&endUserId=user-1",
        &[AUTH],
        None,
    )
    .await;
    let list = r.json()["connections"].as_array().unwrap().clone();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["connection_id"], id);
    assert_eq!(list[0]["provider_config_key"], "crm");
    assert_eq!(list[0]["provider"], "crm");
    assert!(list[0]["created"].is_string());
    assert_eq!(list[0]["end_user"]["id"], "user-1");
    assert!(list[0].get("credentials").is_none());

    let r = call(
        &h,
        "GET",
        &format!("/connections/{id}?provider_config_key=crm"),
        &[AUTH],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let full = r.json();
    assert_eq!(full["credentials"]["type"], "OAUTH2");
    assert_eq!(full["credentials"]["access_token"], "at-1");
    assert_eq!(full["connection_config"]["instance_id"], "inst-7");
    assert_eq!(full["errors"], json!([]));
    assert!(full["last_fetched_at"].is_string());
    let r = call(
        &h,
        "GET",
        &format!("/connection/{id}?provider_config_key=crm"),
        &[AUTH],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "the legacy path works");
    h.http.reply(
        "POST",
        "https://auth.crm.example.com/token",
        token_reply("at-2", None, Some(3_600)),
    );
    let r = call(
        &h,
        "GET",
        &format!("/connections/{id}?provider_config_key=crm&force_refresh=true"),
        &[AUTH],
        None,
    )
    .await;
    assert_eq!(r.json()["credentials"]["access_token"], "at-2");
    assert_eq!(
        call(&h, "GET", &format!("/connections/{id}"), &[AUTH], None)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &h,
            "GET",
            "/connections/nope?provider_config_key=crm",
            &[AUTH],
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    let r = call(&h, "POST", "/connections/metadata", &[AUTH], Some(json!({"connection_id": id, "provider_config_key": "crm", "metadata": {"a": 1, "b": 2}}))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let r = call(&h, "PATCH", "/connections/metadata", &[AUTH], Some(json!({"connection_id": [id.clone()], "provider_config_key": "crm", "metadata": {"b": 3}}))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let r = call(
        &h,
        "GET",
        &format!("/connections/{id}?provider_config_key=crm"),
        &[AUTH],
        None,
    )
    .await;
    assert_eq!(r.json()["metadata"], json!({"a": 1, "b": 3}));
    let r = call(
        &h,
        "PATCH",
        &format!("/connections/{id}?provider_config_key=crm"),
        &[AUTH],
        Some(json!({"tags": {"x": "y"}})),
    )
    .await;
    assert_eq!(r.json()["success"], true);

    let r = call(&h, "POST", "/connections", &[AUTH], Some(json!({"connection_id": "imp", "provider_config_key": "keyed", "credentials": {"type": "API_KEY", "apiKey": "zz"}}))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["connection_id"], "imp");

    let r = call(
        &h,
        "DELETE",
        &format!("/connections/{id}?provider_config_key=crm"),
        &[AUTH],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json(), json!({"success": true}));
    assert_eq!(
        call(
            &h,
            "DELETE",
            &format!("/connections/{id}?provider_config_key=crm"),
            &[AUTH],
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn get_connection_hides_refresh_tokens() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    let id = conn.connection_id.clone();
    for path in ["connections", "connection"] {
        let r = call(
            &h,
            "GET",
            &format!("/{path}/{id}?provider_config_key=crm"),
            &[AUTH],
            None,
        )
        .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        let v = r.json();
        assert_eq!(v["credentials"]["access_token"], "at-1");
        assert_eq!(v["credentials"]["raw"]["access_token"], "at-1");
        assert!(v["credentials"].get("refresh_token").is_none(), "{v}");
        assert!(
            v["credentials"]["raw"].get("refresh_token").is_none(),
            "{v}"
        );
        assert!(!r.text().contains("rt-1"), "{path}");
    }
    let r = call(
        &h,
        "GET",
        &format!("/connections/{id}?provider_config_key=crm&refresh_token=true"),
        &[AUTH],
        None,
    )
    .await;
    let v = r.json();
    assert_eq!(v["credentials"]["refresh_token"], "rt-1");
    assert_eq!(v["credentials"]["raw"]["refresh_token"], "rt-1");

    // Keys like `refreshToken` go too, at any depth, also in the config.
    let body = json!({
        "connection_id": "imp",
        "provider_config_key": "crm",
        "credentials": {
            "type": "OAUTH2",
            "access_token": "at-9",
            "refresh_token": "rt-9",
            "raw": {"data": [{"refreshToken": "rt-deep", "keep": 1}]}
        },
        "connection_config": {"Refresh-Token": "rt-cfg", "nested": {"refresh_token": "rt-n", "keep": 2}}
    });
    let r = call(&h, "POST", "/connections", &[AUTH], Some(body)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let r = call(
        &h,
        "GET",
        "/connections/imp?provider_config_key=crm",
        &[AUTH],
        None,
    )
    .await;
    let v = r.json();
    assert!(!r.text().contains("rt-"), "{v}");
    assert_eq!(v["credentials"]["raw"]["data"][0]["keep"], 1);
    assert_eq!(v["connection_config"]["nested"]["keep"], 2);
    let r = call(
        &h,
        "GET",
        "/connections/imp?provider_config_key=crm&refresh_token=true",
        &[AUTH],
        None,
    )
    .await;
    let v = r.json();
    assert_eq!(
        v["credentials"]["raw"]["data"][0]["refreshToken"],
        "rt-deep"
    );
    assert_eq!(v["connection_config"]["Refresh-Token"], "rt-cfg");
}

#[tokio::test]
async fn proxy_over_http() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    h.http.reply(
        "POST",
        "https://api.crm.example.com/v3/notes",
        HttpResponse::new(201, r#"{"id":"n1"}"#)
            .header("content-type", "application/json")
            .header("x-upstream", "1"),
    );
    let headers = [
        AUTH,
        ("connection-id", conn.connection_id.as_str()),
        ("provider-config-key", "crm"),
        ("nango-proxy-x-trace", "t1"),
        ("retries", "1"),
    ];
    let r = call(
        &h,
        "POST",
        "/proxy/v3/notes?draft=true",
        &headers,
        Some(json!({"text": "hi"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_eq!(r.json()["id"], "n1");
    assert_eq!(r.header("x-upstream"), Some("1"));
    let sent = &h.http.requests_to("https://api.crm.example.com/v3/notes")[0];
    assert_eq!(sent.url, "https://api.crm.example.com/v3/notes?draft=true");
    assert_eq!(sent.header_value("x-trace"), Some("t1"));
    assert_eq!(sent.header_value("authorization"), Some("Bearer at-1"));
    assert_eq!(sent.header_value("content-type"), Some("application/json"));
    assert_eq!(sent.body, br#"{"text":"hi"}"#);
    assert!(
        sent.header_value("connection-id").is_none(),
        "control headers stay here"
    );

    let r = call(
        &h,
        "GET",
        "/proxy/v3/x",
        &[AUTH, ("provider-config-key", "crm")],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"]["code"], "missing_field");
    let r = call(
        &h,
        "GET",
        "/proxy/v3/x",
        &[
            AUTH,
            ("connection-id", &conn.connection_id),
            ("provider-config-key", "crm"),
            ("base-url-override", "https://evil.example.org"),
        ],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.json()["error"]["code"], "forbidden_host");
    let r = call(
        &h,
        "GET",
        "/proxy/v3/x",
        &[
            AUTH,
            ("connection-id", &conn.connection_id),
            ("provider-config-key", "crm"),
            ("retries", "lots"),
        ],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn records_over_http() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", None, None).await;
    h.san
        .save_records(
            "dev",
            &conn.key(),
            "Contact",
            (0..3).map(|i| json!({"id": i.to_string()})).collect(),
        )
        .await
        .unwrap();
    let headers = [
        AUTH,
        ("connection-id", conn.connection_id.as_str()),
        ("provider-config-key", "crm"),
    ];
    let r = call(&h, "GET", "/records?model=Contact&limit=2", &headers, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let body = r.json();
    assert_eq!(body["records"].as_array().unwrap().len(), 2);
    let cursor = body["next_cursor"].as_str().unwrap().to_owned();
    let r = call(
        &h,
        "GET",
        &format!("/records?model=Contact&cursor={cursor}"),
        &headers,
        None,
    )
    .await;
    assert_eq!(r.json()["records"].as_array().unwrap().len(), 1);
    assert_eq!(r.json()["next_cursor"], Value::Null);
    let r = call(
        &h,
        "GET",
        "/records?model=Contact&filter=added&ids=0&ids=1",
        &headers,
        None,
    )
    .await;
    assert_eq!(r.json()["records"].as_array().unwrap().len(), 2);
    // Each `ids` value is one external id, commas and all.
    let r = call(
        &h,
        "GET",
        "/records?model=Contact&ids=0,1&ids[]=2",
        &headers,
        None,
    )
    .await;
    let records = r.json()["records"].as_array().unwrap().clone();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["id"], "2");
    assert_eq!(
        call(&h, "GET", "/records?model=Contact&limit=0", &headers, None)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&h, "GET", "/records", &headers, None).await.status,
        StatusCode::BAD_REQUEST
    );
}

struct Tiny(Arc<AtomicUsize>);

#[async_trait]
impl SyncFunction for Tiny {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "tiny".into(),
            integration: "crm".into(),
            models: vec!["T".into()],
            frequency: Duration::from_secs(60),
            auto_start: false,
        }
    }
    async fn run(&self, ctx: &SyncContext) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        ctx.batch_save("T", vec![json!({"id": "1"})]).await?;
        Ok(())
    }
}

struct Echo;

#[async_trait]
impl ActionFunction for Echo {
    fn spec(&self) -> ActionSpec {
        ActionSpec {
            name: "echo".into(),
            integration: "crm".into(),
        }
    }
    async fn run(&self, _ctx: &ActionContext, input: Value) -> Result<Value> {
        Ok(json!({"echo": input}))
    }
}

#[tokio::test]
async fn syncs_and_actions_over_http() {
    let runs = Arc::new(AtomicUsize::new(0));
    let h = ready_with(Registry::new().sync(Tiny(runs.clone())).action(Echo)).await;
    let conn = connect_crm(&h, "at-1", None, None).await;
    let r = call(&h, "POST", "/sync/trigger", &[AUTH], Some(json!({"syncs": ["tiny"], "provider_config_key": "crm", "connection_id": conn.connection_id}))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["success"], true);
    h.san.flush().await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let r = call(
        &h,
        "POST",
        "/sync/trigger",
        &[AUTH],
        Some(json!({"syncs": [{"name": "tiny"}], "provider_config_key": "crm"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    h.san.flush().await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    let r = call(
        &h,
        "GET",
        &format!(
            "/sync/status?provider_config_key=crm&syncs=*&connection_id={}",
            conn.connection_id
        ),
        &[AUTH],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let s = r.json()["syncs"][0].clone();
    assert_eq!(s["name"], "tiny");
    assert_eq!(s["status"], "SUCCESS");
    assert_eq!(s["connection_id"], conn.connection_id);
    assert_eq!(s["latestResult"]["added"], 0);
    assert!(s["finishedAt"].is_string());
    let r = call(
        &h,
        "POST",
        "/sync/pause",
        &[AUTH],
        Some(json!({"syncs": ["tiny"], "provider_config_key": "crm"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let r = call(
        &h,
        "POST",
        "/sync/start",
        &[AUTH],
        Some(json!({"syncs": ["tiny"], "provider_config_key": "crm"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());

    let headers = [
        AUTH,
        ("connection-id", conn.connection_id.as_str()),
        ("provider-config-key", "crm"),
    ];
    let r = call(
        &h,
        "POST",
        "/action/trigger",
        &headers,
        Some(json!({"action_name": "echo", "input": {"a": 1}})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json(), json!({"echo": {"a": 1}}));
    let r = call(
        &h,
        "POST",
        "/action/trigger",
        &headers,
        Some(json!({"action_name": "nope"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.json()["error"]["code"], "unknown_function");
}

#[tokio::test]
async fn the_connect_page_lists_integrations_and_escapes_text() {
    let h = ready().await;
    let token = session_token(&h, json!({"end_user": {"id": "u1"}, "allowed_integrations": ["crm", "keyed", "basic", "open"]})).await;
    let r = call(
        &h,
        "GET",
        &format!("/connect?session_token={token}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    assert!(r.header("content-security-policy").is_some());
    let html = r.text();
    assert!(html.contains("Test CRM"));
    assert!(html.contains(&format!("/oauth/connect/crm?connect_session_token={token}")));
    assert!(html.contains("/api-auth/api-key/keyed"));
    assert!(html.contains("/api-auth/basic/basic"));
    assert!(html.contains("&lt;b&gt;Open&lt;/b&gt; &amp; Free"));
    assert!(!html.contains("<b>Open</b>"));
    assert!(!html.contains("Zen Desk"), "only allowed integrations");
    let r = call(&h, "GET", "/connect?session_token=smcs_bad", &[], None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_route_list_matches_the_router() {
    let h = ready().await;
    assert!(san_mateo::api::ROUTES.len() >= 30);
    for spec in san_mateo::api::ROUTES {
        let path = spec
            .path
            .replace("{key}", "crm")
            .replace("{id}", "c1")
            .replace("{name}", "crm")
            .replace("{*path}", "x");
        let method = if spec.method == "ANY" {
            "GET"
        } else {
            spec.method
        };
        let r = call(&h, method, &path, &[], None).await;
        assert_ne!(
            r.status,
            StatusCode::NOT_FOUND,
            "{} {}",
            spec.method,
            spec.path
        );
        assert_ne!(
            r.status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{} {}",
            spec.method,
            spec.path
        );
        if spec.secret_key {
            assert_eq!(
                r.status,
                StatusCode::UNAUTHORIZED,
                "{} {}",
                spec.method,
                spec.path
            );
        }
    }
}
