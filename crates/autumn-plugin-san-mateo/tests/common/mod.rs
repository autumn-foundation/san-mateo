//! Shared test helpers.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use autumn_plugin_san_mateo::config::EnvironmentSettings;
use autumn_plugin_san_mateo::san_mateo::transport::HttpResponse;
use autumn_plugin_san_mateo::san_mateo::{Catalog, MemoryTransport};
use autumn_plugin_san_mateo::{SanMateoConfig, SanMateoPlugin};
use autumn_web::test::{TestApp, TestClient, TestResponse};
use serde_json::Value;

pub const DEV_SECRET: &str = "sk_test_plugin_dev_0123456789";
pub const PROD_SECRET: &str = "sk_test_plugin_prod_0123456789";
/// 32 bytes, base64.
pub const KEY_B64: &str = "ZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7fH1+f4CBgoM=";
pub const WHSEC: &str = "whsec_plugin_test";
pub const HOOK: &str = "https://hooks.example.com/sm";
pub const SERVER: &str = "https://app.example.com";

/// Env var names in the test config. The tests never set them in the process.
pub const DEV_SECRET_ENV: &str = "SM_PLUGIN_TEST_DEV_SK";
pub const PROD_SECRET_ENV: &str = "SM_PLUGIN_TEST_PROD_SK";
pub const KEY_ENV: &str = "SM_PLUGIN_TEST_KEY";
pub const WHSEC_ENV: &str = "SM_PLUGIN_TEST_WHSEC";

pub const CATALOG: &str = r"
keyed:
    display_name: Keyed API
    auth_mode: API_KEY
    proxy:
        base_url: https://api.keyed.example.com
        headers:
            x-api-key: ${apiKey}
    docs: https://keyed.example.com/docs
open:
    display_name: Open API
    auth_mode: NONE
    proxy:
        base_url: https://api.open.example.com
    docs: https://open.example.com/docs
";

/// The test config: `dev` with signed webhooks, and `prod`.
pub fn config() -> SanMateoConfig {
    let mut c = SanMateoConfig::default();
    SERVER.clone_into(&mut c.server_url);
    KEY_ENV.clone_into(&mut c.encryption_key_env);
    let mut dev = EnvironmentSettings::default();
    DEV_SECRET_ENV.clone_into(&mut dev.secret_key_env);
    dev.webhooks.urls = vec![HOOK.to_owned()];
    dev.webhooks.signing_secret_env = Some(WHSEC_ENV.to_owned());
    dev.webhooks.retries = 0;
    dev.webhooks.initial_backoff_ms = 0;
    let mut prod = EnvironmentSettings::default();
    PROD_SECRET_ENV.clone_into(&mut prod.secret_key_env);
    c.environments = BTreeMap::from([("dev".to_owned(), dev), ("prod".to_owned(), prod)]);
    c.background.enabled = false;
    c.drain_timeout_secs = 5;
    c
}

/// A fake network that accepts webhooks.
pub fn network() -> MemoryTransport {
    let http = MemoryTransport::new();
    http.reply("POST", HOOK, HttpResponse::new(200, ""));
    http
}

/// A plugin with the test config, test secrets, the test catalog and a fake
/// network.
pub fn plugin(http: &MemoryTransport) -> SanMateoPlugin {
    plugin_with(http, config())
}

pub fn plugin_with(http: &MemoryTransport, c: SanMateoConfig) -> SanMateoPlugin {
    SanMateoPlugin::with_config(c)
        .secret_env(DEV_SECRET_ENV, DEV_SECRET)
        .secret_env(PROD_SECRET_ENV, PROD_SECRET)
        .secret_env(KEY_ENV, KEY_B64)
        .secret_env(WHSEC_ENV, WHSEC)
        .catalog(Catalog::from_yaml_str(CATALOG).unwrap())
        .transport(Arc::new(http.clone()))
}

pub fn app(p: SanMateoPlugin) -> TestClient {
    TestApp::new().plugin(p).build()
}

pub fn bearer(secret: &str) -> String {
    format!("Bearer {secret}")
}

/// Sends a JSON request with the dev secret key.
pub async fn api(client: &TestClient, method: &str, path: &str, body: Option<Value>) -> TestResponse {
    let auth = bearer(DEV_SECRET);
    let req = match method {
        "GET" => client.get(path),
        "POST" => client.post(path),
        "PATCH" => client.patch(path),
        "DELETE" => client.delete(path),
        "PUT" => client.put(path),
        other => panic!("no helper for {other}"),
    }
    .header("authorization", &auth)
    .header("accept", "application/json");
    match body {
        Some(b) => req.json(&b).send().await,
        None => req.send().await,
    }
}

pub fn json(r: &TestResponse) -> Value {
    serde_json::from_slice(&r.body)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&r.body)))
}

/// Creates the `keyed` and `open` integrations in `dev`.
pub async fn integrations(client: &TestClient, prefix: &str) {
    for key in ["keyed", "open"] {
        let r = api(
            client,
            "POST",
            &format!("{prefix}/integrations"),
            Some(serde_json::json!({"unique_key": key, "provider": key})),
        )
        .await;
        r.assert_status(200);
    }
}

/// Makes a connect session in `dev` and returns its token.
pub async fn session(client: &TestClient, prefix: &str) -> String {
    let r = api(
        client,
        "POST",
        &format!("{prefix}/connect/sessions"),
        Some(serde_json::json!({"end_user": {"id": "u1"}})),
    )
    .await;
    r.assert_status(201);
    json(&r)["data"]["token"].as_str().unwrap().to_owned()
}

/// Waits until `f` is true, or fails after `limit`.
pub async fn wait_until(limit: Duration, f: impl Fn() -> bool) {
    let start = tokio::time::Instant::now();
    while !f() {
        assert!(start.elapsed() < limit, "condition not met in {limit:?}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
