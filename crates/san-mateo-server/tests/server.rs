//! The standalone server (AC19).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use san_mateo_server::{ServerConfig, app, build, serve};
use tower::ServiceExt as _;

const KEY: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    move |k| map.get(k).cloned()
}

fn base() -> Vec<(&'static str, &'static str)> {
    vec![("SAN_MATEO_ENCRYPTION_KEY", KEY), ("SAN_MATEO_DEV_SECRET_KEY", "sk_dev_0123456789abcdef")]
}

#[test]
fn defaults() {
    let c = ServerConfig::from_env(env(&base())).unwrap();
    assert_eq!(c.service.server_url, "http://localhost:3003");
    assert_eq!(c.bind.to_string(), "0.0.0.0:3003");
    assert_eq!(c.database_url, None);
    assert_eq!(c.encryption_key, ("k1".to_owned(), KEY.to_owned()));
    assert_eq!(c.service.environments.len(), 1);
    assert_eq!(c.service.environments[0].id, "dev");
    assert!(c.background);
    assert_eq!(c.tick_secs, 30);
    assert!(!c.service.proxy.allow_private_hosts);
}

#[test]
fn full_settings() {
    let mut e = base();
    e.extend([
        ("SAN_MATEO_SERVER_URL", "https://sm.example.com"),
        ("SAN_MATEO_BIND", "127.0.0.1:8080"),
        ("SAN_MATEO_DATABASE_URL", "postgres://u:p@db/sm"),
        ("SAN_MATEO_ENCRYPTION_KEY_ID", "k2"),
        ("SAN_MATEO_OLD_ENCRYPTION_KEYS", "k1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="),
        ("SAN_MATEO_LOOKUP_KEY_ID", "k1"),
        ("SAN_MATEO_ENVIRONMENTS", "dev, prod-eu"),
        ("SAN_MATEO_PROD_EU_SECRET_KEY", "sk_prod_0123456789abcdef"),
        ("SAN_MATEO_PROD_EU_CALLBACK_URL", "https://sm.example.com/eu/oauth/callback"),
        ("SAN_MATEO_PROD_EU_WEBHOOK_URL", "https://hooks.example.com/a"),
        ("SAN_MATEO_PROD_EU_WEBHOOK_URL_SECONDARY", "https://hooks.example.com/b"),
        ("SAN_MATEO_PROD_EU_WEBHOOK_SECRET", "whsec"),
        ("SAN_MATEO_PROVIDERS_FILE", "/etc/sm/providers.yaml"),
        ("SAN_MATEO_ALLOWED_HOSTS", "files.example.com, *.cdn.example.com"),
        ("SAN_MATEO_ALLOW_PRIVATE_HOSTS", "true"),
        ("SAN_MATEO_ALLOW_PLAIN_HTTP", "1"),
        ("SAN_MATEO_BACKGROUND", "false"),
        ("SAN_MATEO_TICK_SECS", "5"),
        ("SAN_MATEO_CONNECT_URL", "https://connect.example.com"),
    ]);
    let c = ServerConfig::from_env(env(&e)).unwrap();
    assert_eq!(c.service.server_url, "https://sm.example.com");
    assert_eq!(c.bind.to_string(), "127.0.0.1:8080");
    assert_eq!(c.database_url.as_deref(), Some("postgres://u:p@db/sm"));
    assert_eq!(c.encryption_key.0, "k2");
    assert_eq!(c.old_keys.len(), 1);
    assert_eq!(c.lookup_key_id.as_deref(), Some("k1"));
    let prod = &c.service.environments[1];
    assert_eq!(prod.id, "prod-eu");
    assert_eq!(prod.callback_url.as_deref(), Some("https://sm.example.com/eu/oauth/callback"));
    assert_eq!(prod.webhooks.primary_url.as_deref(), Some("https://hooks.example.com/a"));
    assert_eq!(prod.webhooks.secondary_url.as_deref(), Some("https://hooks.example.com/b"));
    assert_eq!(prod.webhooks.signing_secret.as_ref().unwrap().expose(), "whsec");
    assert_eq!(c.providers_file.as_deref(), Some("/etc/sm/providers.yaml"));
    assert_eq!(c.service.proxy.allowed_hosts, vec!["files.example.com", "*.cdn.example.com"]);
    assert!(c.service.proxy.allow_private_hosts && c.service.proxy.allow_plain_http);
    assert!(!c.background);
    assert_eq!(c.tick_secs, 5);
    assert_eq!(c.service.connect_url.as_deref(), Some("https://connect.example.com"));
}

#[test]
fn errors_name_the_variable() {
    let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
        (vec![("SAN_MATEO_DEV_SECRET_KEY", "sk_dev_0123456789abcdef")], "SAN_MATEO_ENCRYPTION_KEY"),
        (vec![("SAN_MATEO_ENCRYPTION_KEY", KEY)], "SAN_MATEO_DEV_SECRET_KEY"),
        (vec![("SAN_MATEO_ENCRYPTION_KEY", "c2hvcnQ="), ("SAN_MATEO_DEV_SECRET_KEY", "sk")], "SAN_MATEO_ENCRYPTION_KEY"),
        ([base(), vec![("SAN_MATEO_BIND", "nope")]].concat(), "SAN_MATEO_BIND"),
        ([base(), vec![("SAN_MATEO_TICK_SECS", "x")]].concat(), "SAN_MATEO_TICK_SECS"),
        ([base(), vec![("SAN_MATEO_BACKGROUND", "maybe")]].concat(), "SAN_MATEO_BACKGROUND"),
        ([base(), vec![("SAN_MATEO_OLD_ENCRYPTION_KEYS", "no-colon")]].concat(), "SAN_MATEO_OLD_ENCRYPTION_KEYS"),
        ([base(), vec![("SAN_MATEO_ENVIRONMENTS", " , ")]].concat(), "SAN_MATEO_ENVIRONMENTS"),
    ];
    for (vars, name) in cases {
        let err = ServerConfig::from_env(env(&vars)).unwrap_err().to_string();
        assert!(err.contains(name), "{name}: {err}");
        assert!(!err.contains(KEY), "no secret in errors: {err}");
    }
}

#[tokio::test]
async fn the_app_serves_the_api() {
    let c = ServerConfig::from_env(env(&base())).unwrap();
    let san = build(&c).await.unwrap();
    let app = app(san);
    let res = app.clone().oneshot(Request::get("/health").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .oneshot(Request::get("/providers").header("authorization", "Bearer sk_dev_0123456789abcdef").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v["data"].as_array().unwrap().len() >= 25, "the built-in catalog");
}

#[tokio::test]
async fn a_providers_file_adds_to_the_catalog() {
    let dir = std::env::temp_dir().join(format!("sm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("providers.yaml");
    std::fs::write(&file, "acme-api:\n  display_name: Acme\n  auth_mode: API_KEY\n  proxy:\n    base_url: https://api.acme.example.com\n").unwrap();
    let mut e = base();
    let path = file.to_string_lossy().into_owned();
    e.push(("SAN_MATEO_PROVIDERS_FILE", &path));
    let san = build(&ServerConfig::from_env(env(&e)).unwrap()).await.unwrap();
    assert!(san.catalog().get("acme-api").is_some());
    assert!(san.catalog().get("github").is_some());
    let mut bad = base();
    bad.push(("SAN_MATEO_PROVIDERS_FILE", "/no/such/file.yaml"));
    assert!(build(&ServerConfig::from_env(env(&bad)).unwrap()).await.is_err());
}

#[tokio::test]
async fn build_uses_postgres_when_set() {
    let Ok(url) = std::env::var("SAN_MATEO_TEST_DATABASE_URL") else {
        eprintln!("SAN_MATEO_TEST_DATABASE_URL is not set: the Postgres build test does nothing");
        return;
    };
    let mut e = base();
    e.push(("SAN_MATEO_DATABASE_URL", &url));
    let san = build(&ServerConfig::from_env(env(&e)).unwrap()).await.unwrap();
    let list = san.list_integrations("dev").await.unwrap();
    let _ = list;
}

#[tokio::test]
async fn serve_stops_cleanly_on_shutdown() {
    let c = ServerConfig::from_env(env(&base())).unwrap();
    let san = build(&c).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, app(san), async move {
        let _ = rx.await;
    }));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    stream.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn run_starts_and_stops() {
    let mut e = base();
    e.push(("SAN_MATEO_BIND", "127.0.0.1:0"));
    e.push(("SAN_MATEO_TICK_SECS", "1"));
    let c = ServerConfig::from_env(env(&e)).unwrap();
    let done = tokio::time::timeout(Duration::from_secs(10), san_mateo_server::run(c, tokio::time::sleep(Duration::from_millis(300)))).await;
    done.unwrap().unwrap();
}
