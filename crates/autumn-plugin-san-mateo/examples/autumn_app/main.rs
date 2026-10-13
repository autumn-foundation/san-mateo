//! An autumn-web app with the San Mateo plugin.
//!
//! ```text
//! backend ─► /san-mateo/*            the San Mateo REST API (the plugin)
//! browser ─► /contacts/{connection}  your route ─► SanMateoService ─► provider
//! ```
//!
//! One `.plugin(..)` call adds San Mateo. `autumn.toml` (next to this file)
//! holds the names of the env vars with the secrets, never the secrets. Your
//! route `/contacts/{connection}` calls the provider. San Mateo adds the API
//! key of the connection.
//!
//! Run it from the repository root. autumn reads `autumn.toml` from
//! `AUTUMN_MANIFEST_DIR`:
//!
//! ```sh
//! export AUTUMN_MANIFEST_DIR=crates/autumn-plugin-san-mateo/examples/autumn_app
//! export SAN_MATEO_ENCRYPTION_KEY="$(openssl rand -base64 32)"
//! export SAN_MATEO_DEV_SECRET_KEY="sk_example_$(openssl rand -hex 16)"
//! cargo run --locked -p autumn-plugin-san-mateo --example autumn_app
//! ```
//!
//! Then call the API on `http://127.0.0.1:3000/san-mateo` with
//! `Authorization: Bearer $SAN_MATEO_DEV_SECRET_KEY`. The prod profile of
//! autumn-web also needs `AUTUMN_SECURITY__SIGNING_SECRET` and
//! `AUTUMN_SECURITY__TRUSTED_HOSTS__HOSTS`.
//! `scripts/smoke-autumn-example.sh` boots this app in the dev and prod
//! profiles and checks it. The test below runs the same routes and the same
//! config file in `TestApp`, with a fake provider.

use autumn_plugin_san_mateo::san_mateo::{Catalog, ConnectionKey, ProxyRequest};
use autumn_plugin_san_mateo::{SanMateoPlugin, SanMateoService, autumn_error};
use autumn_web::{AutumnResult, Json, Path};
use serde_json::Value;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const ENV: &str = "dev";
const INTEGRATION: &str = "acme-crm";

/// The provider, in the Nango `providers.yaml` format.
const CATALOG: &str = r"
acme-crm:
    display_name: Acme CRM
    auth_mode: API_KEY
    proxy:
        base_url: https://api.acme-crm.example.com
        headers:
            x-api-key: ${apiKey}
    docs: https://acme-crm.example.com/docs
";

#[autumn_web::get("/")]
async fn index() -> &'static str {
    "autumn_app: the San Mateo API is at /san-mateo"
}

/// Your route. San Mateo adds the API key of the connection.
///
/// This demo takes the connection id from the URL. In production, find the
/// connection of the signed-in user. Do not trust an id from the client.
#[autumn_web::get("/contacts/{connection_id}")]
async fn contacts(
    san: SanMateoService,
    Path(connection_id): Path<String>,
) -> AutumnResult<Json<Value>> {
    let key = ConnectionKey::new(INTEGRATION, connection_id);
    let reply = san
        .proxy(ENV, &key, ProxyRequest::get("/v1/contacts"))
        .await
        .map_err(autumn_error)?;
    Ok(Json(reply.json().map_err(autumn_error)?))
}

/// The provider catalog. It replaces the built-in catalog.
fn catalog() -> autumn_plugin_san_mateo::san_mateo::Result<Catalog> {
    Catalog::from_yaml_str(CATALOG)
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // `run` reads `[san_mateo]` from autumn.toml. It stops on SIGTERM or
    // Ctrl-C, after the plugin stops its loops and sends its webhooks.
    autumn_web::app()
        .routes(autumn_web::routes![index, contacts])
        .plugin(SanMateoPlugin::new().catalog(catalog()?))
        .run()
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use autumn_plugin_san_mateo::SanMateoConfig;
    use autumn_plugin_san_mateo::san_mateo::MemoryTransport;
    use autumn_plugin_san_mateo::san_mateo::clock::{Entropy, OsEntropy};
    use autumn_plugin_san_mateo::san_mateo::transport::HttpResponse;
    use autumn_web::test::{TestApp, TestClient, TestResponse};
    use base64::Engine as _;
    use serde_json::{Value, json};

    use super::*;

    /// The longest time for the test scenario.
    const RUN_TIMEOUT: Duration = Duration::from_secs(60);

    const CONTACTS_URL: &str = "https://api.acme-crm.example.com/v1/contacts";

    fn fake_provider() -> MemoryTransport {
        let fake = MemoryTransport::new();
        fake.reply(
            "GET",
            CONTACTS_URL,
            HttpResponse::json(200, &json!({"contacts": [{"id": "c-1", "name": "Ada"}]})),
        );
        fake
    }

    /// The secrets of one run. The test gives them to the plugin, so it sets
    /// no env var.
    struct Secrets {
        encryption_key_b64: String,
        secret_key: String,
        api_key: String,
    }

    /// Runs the app in `TestApp` with the same routes and config file as
    /// `main`, drives it over HTTP and checks the result.
    async fn run(fake: MemoryTransport) -> Result<Value, BoxError> {
        let mut key = [0_u8; 32];
        OsEntropy.fill(&mut key);
        let secrets = Secrets {
            encryption_key_b64: base64::engine::general_purpose::STANDARD.encode(key),
            secret_key: format!("sk_example_{}", OsEntropy.hex(16)),
            api_key: format!("ak_example_{}", OsEntropy.hex(16)),
        };
        let config = SanMateoConfig::from_toml_str(include_str!("autumn.toml"))?;
        let plugin = SanMateoPlugin::with_config(config)
            .catalog(catalog()?)
            .secret_env(
                "SAN_MATEO_ENCRYPTION_KEY",
                secrets.encryption_key_b64.as_str(),
            )
            .secret_env("SAN_MATEO_DEV_SECRET_KEY", secrets.secret_key.as_str())
            .transport(Arc::new(fake.clone()))
            .background(false);
        let client = TestApp::new()
            .routes(autumn_web::routes![index, contacts])
            .plugin(plugin)
            .build();
        let summary = tokio::time::timeout(RUN_TIMEOUT, drive(&client, &secrets))
            .await
            .map_err(|_| format!("the scenario did not end in {RUN_TIMEOUT:?}"))?;
        // `TestApp` runs no shutdown hook, so wait for the background tasks here.
        if let Some(san) = SanMateoService::from_state(client.state()) {
            san.flush().await;
        }
        let mut summary = summary?;
        let upstream = fake.requests_to(CONTACTS_URL);
        summary["upstream_calls"] = json!(upstream.len());
        summary["upstream_api_key_is_the_user_key"] = json!(
            !upstream.is_empty()
                && upstream
                    .iter()
                    .all(|r| r.header_value("x-api-key") == Some(secrets.api_key.as_str()))
        );
        let text = summary.to_string();
        for secret in [
            &secrets.encryption_key_b64,
            &secrets.secret_key,
            &secrets.api_key,
        ] {
            if text.contains(secret.as_str()) {
                return Err("the summary shows a secret".into());
            }
        }
        check(&summary)?;
        Ok(summary)
    }

    /// The backend and the browser. They call the app over HTTP.
    async fn drive(client: &TestClient, secrets: &Secrets) -> Result<Value, BoxError> {
        let auth = format!("Bearer {}", secrets.secret_key);
        let r = client
            .post("/san-mateo/integrations")
            .header("authorization", &auth)
            .json(&json!({"unique_key": INTEGRATION, "provider": INTEGRATION}))
            .send()
            .await;
        want(&r, 200)?;
        let session = client
            .post("/san-mateo/connect/sessions")
            .header("authorization", &auth)
            .json(&json!({"end_user": {"id": "user-1"}}))
            .send()
            .await;
        let token = body(&session)?["data"]["token"]
            .as_str()
            .ok_or("no session token")?
            .to_owned();
        // The end user sends the API key on the connect page.
        let connect = client
            .post(&format!(
                "/san-mateo/api-auth/api-key/{INTEGRATION}?connect_session_token={token}"
            ))
            .json(&json!({"apiKey": secrets.api_key}))
            .send()
            .await;
        let connection_id = body(&connect)?["connectionId"]
            .as_str()
            .ok_or("no connection id")?
            .to_owned();
        let app = client
            .get(&format!("/contacts/{connection_id}"))
            .send()
            .await;
        let health = client.get("/actuator/health").send().await;
        let health_text = String::from_utf8_lossy(&health.body).into_owned();
        if [
            &secrets.encryption_key_b64,
            &secrets.secret_key,
            &secrets.api_key,
        ]
        .iter()
        .any(|s| health_text.contains(s.as_str()))
        {
            return Err("the health report shows a secret".into());
        }
        Ok(json!({
            "session_status": session.status.as_u16(),
            "connect_status": connect.status.as_u16(),
            "app_reply": {"status": app.status.as_u16(), "body": body(&app).unwrap_or(Value::Null)},
            "health": {"status": health.status.as_u16(), "has_san_mateo": health_text.contains("san_mateo")},
        }))
    }

    fn want(r: &TestResponse, status: u16) -> Result<(), BoxError> {
        if r.status.as_u16() == status {
            Ok(())
        } else {
            Err(format!(
                "want {status}, got {}: {}",
                r.status,
                String::from_utf8_lossy(&r.body)
            )
            .into())
        }
    }

    fn body(r: &TestResponse) -> Result<Value, BoxError> {
        Ok(serde_json::from_slice(&r.body)?)
    }

    fn check(summary: &Value) -> Result<(), String> {
        let expected = json!({
            "session_status": 201,
            "connect_status": 200,
            "app_reply": {"status": 200, "body": {"contacts": [{"id": "c-1", "name": "Ada"}]}},
            "upstream_calls": 1,
            "upstream_api_key_is_the_user_key": true,
            "health": {"status": 200, "has_san_mateo": true},
        });
        if *summary == expected {
            Ok(())
        } else {
            Err(format!("the summary {summary} is not {expected}"))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_app_route_proxies_with_the_plugin() {
        let summary = run(fake_provider()).await.expect("the example passes");
        check(&summary).expect("run checks the summary");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_wrong_reply_fails_the_check() {
        let fake = MemoryTransport::new();
        fake.reply("GET", CONTACTS_URL, HttpResponse::new(500, "down"));
        let err = run(fake).await.expect_err("the check finds the error");
        assert!(err.to_string().contains("is not"), "{err}");
    }
}
