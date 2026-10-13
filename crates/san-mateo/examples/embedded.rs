//! Embed San Mateo in your own axum app.
//!
//! ```text
//! backend ─► /san-mateo/*            San Mateo REST API (nested router)
//! backend ─► /contacts/{connection}  your handler ─► san.proxy(..) ─► provider
//! ```
//!
//! The example makes an integration and connects an `OAuth2` account over the
//! REST API, with no browser. Then your handler calls the provider. The first
//! token expires soon, so San Mateo refreshes it before the call. The example
//! checks the result. It returns an error when the result is wrong.
//!
//! `MemoryTransport` is the fake provider, so you need no account and no
//! network. For production: remove `.transport(..)` (the default is
//! `ReqwestTransport`), use `PgStore` from `san-mateo-postgres`, and read the
//! keys from a secret store.
//!
//! ```sh
//! cargo run --locked -p san-mateo --example embedded
//! ```

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use san_mateo::clock::{Entropy, OsEntropy};
use san_mateo::transport::HttpResponse;
use san_mateo::{
    BackgroundSettings, Catalog, ConnectionKey, EnvironmentConfig, KeyRing, MemoryTransport,
    ProxyRequest, SanMateo, Secret, WebhookSettings,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use url::Url;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The longest time for the scenario. CI must not hang.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

const ENV: &str = "dev";
const INTEGRATION: &str = "acme-crm";
const TOKEN_URL: &str = "https://auth.acme-crm.example.com/token";
const CONTACTS_URL: &str = "https://api.acme-crm.example.com/v3/contacts";
const HOOK_URL: &str = "https://backend.example.com/san-mateo-hooks";

/// The provider, in the Nango `providers.yaml` format.
const CATALOG: &str = r"
acme-crm:
    display_name: Acme CRM
    auth_mode: OAUTH2
    authorization_url: https://auth.acme-crm.example.com/authorize
    token_url: https://auth.acme-crm.example.com/token
    default_scopes: [contacts.read]
    proxy:
        base_url: https://api.acme-crm.example.com
    docs: https://acme-crm.example.com/docs
";

/// Fake tokens. The check makes sure that the summary does not show them.
const FAKE_SECRETS: [&str; 4] = ["at-1", "at-2", "rt-1", "rt-2"];

/// The fake provider and the fake backend that gets the webhooks.
fn fake_network() -> MemoryTransport {
    let fake = MemoryTransport::new();
    // The code exchange. The token expires in 60 s, which is inside the
    // refresh buffer (15 min).
    fake.reply(
        "POST",
        TOKEN_URL,
        HttpResponse::json(
            200,
            &json!({"access_token": "at-1", "refresh_token": "rt-1", "expires_in": 60}),
        ),
    );
    // The refresh.
    fake.reply(
        "POST",
        TOKEN_URL,
        HttpResponse::json(
            200,
            &json!({"access_token": "at-2", "refresh_token": "rt-2", "expires_in": 3600}),
        ),
    );
    fake.reply(
        "GET",
        CONTACTS_URL,
        HttpResponse::json(200, &contacts_json()),
    );
    fake.reply("POST", HOOK_URL, HttpResponse::new(200, ""));
    fake
}

/// The contacts of the fake provider.
fn contacts_json() -> Value {
    json!({"results": [{"id": "c-1", "name": "Ada"}]})
}

/// Your handler. San Mateo adds the auth and refreshes the token.
///
/// This demo takes the connection id from the URL. In production, find the
/// connection of the signed-in user. Do not trust an id from the client.
async fn contacts(
    State(san): State<SanMateo>,
    Path(connection_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let key = ConnectionKey::new(INTEGRATION, connection_id);
    let reply = san
        .proxy(ENV, &key, ProxyRequest::get("/v3/contacts"))
        .await
        .map_err(|e| api_error(&e))?;
    // San Mateo gives back each provider reply, also a 4xx or a 5xx.
    if !reply.is_success() {
        let error = json!({"error": {"code": "upstream_status", "status": reply.status}});
        return Err((StatusCode::BAD_GATEWAY, Json(error)));
    }
    reply.json().map(Json).map_err(|e| api_error(&e))
}

/// An error reply with the status and the code of a San Mateo error.
fn api_error(e: &san_mateo::Error) -> (StatusCode, Json<Value>) {
    let status = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_GATEWAY);
    (status, Json(json!({"error": {"code": e.code()}})))
}

/// The secrets of one run. In production, read them from a secret store.
struct Secrets {
    secret_key: String,
    client_secret: String,
    webhook_secret: String,
}

impl Secrets {
    /// The secrets of the run and the fake tokens.
    fn all(&self) -> Vec<String> {
        let mut all = vec![
            self.secret_key.clone(),
            self.client_secret.clone(),
            self.webhook_secret.clone(),
        ];
        all.extend(FAKE_SECRETS.iter().map(|s| (*s).to_owned()));
        all
    }

    fn random() -> Self {
        Self {
            secret_key: format!("sk_example_{}", OsEntropy.hex(16)),
            client_secret: OsEntropy.hex(16),
            webhook_secret: OsEntropy.hex(16),
        }
    }
}

/// Builds the service. `server_url` is the public URL of the San Mateo API.
fn san_mateo(
    server_url: &str,
    secrets: &Secrets,
    fake: &MemoryTransport,
) -> Result<SanMateo, BoxError> {
    let mut key = [0_u8; 32];
    OsEntropy.fill(&mut key);
    let mut env = EnvironmentConfig::new(ENV, secrets.secret_key.as_str());
    env.webhooks = WebhookSettings {
        primary_url: Some(HOOK_URL.to_owned()),
        signing_secret: Some(Secret::new(secrets.webhook_secret.as_str())),
        ..WebhookSettings::default()
    };
    Ok(SanMateo::builder()
        .server_url(server_url)
        .environment(env)
        .key_ring(KeyRing::new("k1", key)?)
        .catalog(Catalog::from_yaml_str(CATALOG)?)
        .transport(Arc::new(fake.clone()))
        .build()?)
}

/// What the backend saw.
struct Seen {
    /// The provider URL from the 302 of the OAuth start.
    authorize: Url,
    app_status: u16,
    app_body: Value,
}

/// Runs the scenario on `fake`, stops all parts, and checks the result.
async fn run(fake: MemoryTransport) -> Result<Value, BoxError> {
    // Bind first: the OAuth callback URL needs the port.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server_url = format!("{base}/san-mateo");
    let secrets = Secrets::random();
    let san = san_mateo(&server_url, &secrets, &fake)?;

    // The refresh sweep, the sync scheduler and the session purge.
    let background = san.start_background(BackgroundSettings::default());
    let app = Router::new()
        .route("/contacts/{connection_id}", get(contacts))
        .with_state(san.clone())
        .nest("/san-mateo", san_mateo::api::router(san.clone()));
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });

    let seen = tokio::time::timeout(RUN_TIMEOUT, backend(&base, &secrets)).await;

    // Stop in all cases: the server, the loops, then the webhooks.
    let _ = stop.send(());
    let served = serving.await;
    background.stop().await;
    let flushed = san.flush_within(Duration::from_secs(5)).await;

    let seen = seen.map_err(|_| format!("the scenario did not end in {RUN_TIMEOUT:?}"))??;
    served??;
    let summary = summarize(&seen, &fake, &server_url, &secrets, flushed);
    check(&summary)?;
    Ok(summary)
}

/// Your backend. It calls San Mateo over HTTP with the secret key.
async fn backend(base: &str, secrets: &Secrets) -> Result<Seen, BoxError> {
    // San Mateo never follows a redirect. The backend does not either.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let api = Api {
        client: &client,
        base,
        secret_key: &secrets.secret_key,
    };

    // 1. The integration. The client secret is sealed and never comes back.
    let integration = json!({
        "unique_key": INTEGRATION,
        "provider": INTEGRATION,
        "credentials": {"type": "OAUTH2", "client_id": "acme-client", "client_secret": secrets.client_secret},
    });
    api.post("/integrations", &integration, 200).await?;

    // 2. A connect session for one end user.
    let user = json!({"end_user": {"id": "user-1"}});
    let session = api.post("/connect/sessions", &user, 201).await?;
    let token = session["data"]["token"].as_str().ok_or("no token")?;

    // 3. The OAuth start. A browser follows the 302 to the provider.
    let start =
        format!("{base}/san-mateo/oauth/connect/{INTEGRATION}?connect_session_token={token}");
    let start = client.get(start).send().await?;
    if start.status() != reqwest::StatusCode::FOUND {
        return Err(format!("the OAuth start gave {}", start.status()).into());
    }
    let location = start.headers().get("location").ok_or("no Location")?;
    let authorize = Url::parse(location.to_str()?)?;

    // 4. The user agrees. The provider sends the browser to the callback.
    let state = query(&authorize, "state").unwrap_or_default();
    let callback = format!("{base}/san-mateo/oauth/callback?code=code-1&state={state}");
    let callback = client.get(callback).send().await?;
    if !callback.status().is_success() {
        return Err(format!("the OAuth callback gave {}", callback.status()).into());
    }

    // 5. The new connection.
    let list = api
        .get(&format!("/connections?integrationId={INTEGRATION}"))
        .await?;
    let connection_id = list["connections"][0]["connection_id"]
        .as_str()
        .ok_or("no connection")?;

    // 6. Your own route.
    let reply = client
        .get(format!("{base}/contacts/{connection_id}"))
        .send()
        .await?;
    let app_status = reply.status().as_u16();
    let app_body = serde_json::from_slice(&reply.bytes().await?)?;
    Ok(Seen {
        authorize,
        app_status,
        app_body,
    })
}

/// The San Mateo REST API, with the secret key of the environment.
struct Api<'a> {
    client: &'a reqwest::Client,
    base: &'a str,
    secret_key: &'a str,
}

impl Api<'_> {
    async fn get(&self, path: &str) -> Result<Value, BoxError> {
        self.send(self.client.get(self.url(path)), 200).await
    }

    async fn post(&self, path: &str, body: &Value, status: u16) -> Result<Value, BoxError> {
        let request = self
            .client
            .post(self.url(path))
            .header("content-type", "application/json")
            .body(body.to_string());
        self.send(request, status).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}/san-mateo{path}", self.base)
    }

    /// Sends the request with the secret key and wants `status`.
    async fn send(&self, request: reqwest::RequestBuilder, status: u16) -> Result<Value, BoxError> {
        let auth = format!("Bearer {}", self.secret_key);
        let reply = request.header("authorization", auth).send().await?;
        let got = reply.status().as_u16();
        let bytes = reply.bytes().await?;
        if got != status {
            let text = String::from_utf8_lossy(&bytes);
            return Err(format!("want {status}, got {got}: {text}").into());
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn query(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// Builds the summary from what the backend saw and from the fake log.
fn summarize(
    seen: &Seen,
    fake: &MemoryTransport,
    server_url: &str,
    secrets: &Secrets,
    flushed: bool,
) -> Value {
    let token_calls = fake.requests_to(TOKEN_URL);
    let form = |i: usize, name: &str| -> Option<String> {
        let body = &token_calls.get(i)?.body;
        url::form_urlencoded::parse(body)
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    let grants: Vec<Option<String>> = (0..token_calls.len())
        .map(|i| form(i, "grant_type"))
        .collect();
    let verifier = form(0, "code_verifier").unwrap_or_default();
    let challenge = query(&seen.authorize, "code_challenge").unwrap_or_default();
    let upstream = fake.requests_to(CONTACTS_URL);
    let new_token = upstream
        .iter()
        .all(|r| r.header_value("authorization") == Some("Bearer at-2"));
    let hooks = fake.requests_to(HOOK_URL);
    let webhook = hooks.first().map_or(Value::Null, |hook| {
        let body: Value = serde_json::from_slice(&hook.body).unwrap_or(Value::Null);
        let signature = hook.header_value("x-san-mateo-hmac-sha256").unwrap_or_default();
        let text = String::from_utf8_lossy(&hook.body);
        json!({
            "type": body["type"],
            "operation": body["operation"],
            "success": body["success"],
            "signature_valid": san_mateo::webhook::verify(&secrets.webhook_secret, &hook.body, signature),
            "shows_no_secret": !secrets.all().iter().any(|s| text.contains(s.as_str())),
        })
    });
    let redirect_uri = query(&seen.authorize, "redirect_uri").unwrap_or_default();
    json!({
        "authorize": {
            "host": seen.authorize.host_str(),
            "redirect_uri": redirect_uri.replace(server_url, "{server}"),
            "code_challenge_method": query(&seen.authorize, "code_challenge_method"),
            "pkce_verifier_matches": !verifier.is_empty() && san_mateo::oauth2::pkce_challenge(&verifier) == challenge,
            "state_is_set": query(&seen.authorize, "state").is_some_and(|s| !s.is_empty()),
        },
        "token_grants": grants,
        "upstream_calls": upstream.len(),
        "upstream_used_new_token": !upstream.is_empty() && new_token,
        "app_reply": {"status": seen.app_status, "body": seen.app_body},
        "webhook": webhook,
        "stopped_cleanly": flushed,
    })
}

/// Compares the summary with the expected values. The error starts with
/// `check failed:`.
fn check(summary: &Value) -> Result<(), String> {
    // Look for a secret first. Then the error message does not show it.
    let text = summary.to_string();
    if FAKE_SECRETS.iter().any(|s| text.contains(s)) {
        return Err("check failed: the summary shows a fake token".to_owned());
    }
    let expected = json!({
        "authorize": {
            "host": "auth.acme-crm.example.com",
            "redirect_uri": "{server}/oauth/callback",
            "code_challenge_method": "S256",
            "pkce_verifier_matches": true,
            "state_is_set": true,
        },
        "token_grants": ["authorization_code", "refresh_token"],
        "upstream_calls": 1,
        "upstream_used_new_token": true,
        "app_reply": {"status": 200, "body": {"results": [{"id": "c-1", "name": "Ada"}]}},
        "webhook": {
            "type": "auth",
            "operation": "creation",
            "success": true,
            "signature_valid": true,
            "shows_no_secret": true,
        },
        "stopped_cleanly": true,
    });
    if *summary != expected {
        return Err(format!(
            "check failed: the summary {summary} is not {expected}"
        ));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let summary = run(fake_network()).await?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_app_connects_refreshes_and_proxies() {
        let summary = run(fake_network()).await.expect("the example passes");
        check(&summary).expect("run checks the summary");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_missing_refresh_fails_the_check() {
        // The fake has no refresh reply, so it repeats the first token reply.
        // The upstream call then gets the old token.
        let fake = MemoryTransport::new();
        fake.reply(
            "POST",
            TOKEN_URL,
            HttpResponse::json(
                200,
                &json!({"access_token": "at-1", "refresh_token": "rt-1", "expires_in": 60}),
            ),
        );
        fake.reply(
            "GET",
            CONTACTS_URL,
            HttpResponse::json(200, &contacts_json()),
        );
        fake.reply("POST", HOOK_URL, HttpResponse::new(200, ""));
        let err = run(fake).await.expect_err("the check finds the old token");
        let err = err.to_string();
        assert!(err.starts_with("check failed:"), "{err}");
        assert!(err.contains(r#""upstream_used_new_token":false"#), "{err}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upstream_error_gives_a_502() {
        let fake = MemoryTransport::new();
        for (access, expires_in) in [("at-1", 60), ("at-2", 3600)] {
            let reply =
                json!({"access_token": access, "refresh_token": "rt-x", "expires_in": expires_in});
            fake.reply("POST", TOKEN_URL, HttpResponse::json(200, &reply));
        }
        fake.reply(
            "GET",
            CONTACTS_URL,
            HttpResponse::json(500, &json!({"error": "down"})),
        );
        fake.reply("POST", HOOK_URL, HttpResponse::new(200, ""));
        let err = run(fake).await.expect_err("the handler gives a 502");
        let err = err.to_string();
        assert!(err.starts_with("check failed:"), "{err}");
        assert!(err.contains(r#""status":502"#), "{err}");
    }
}
