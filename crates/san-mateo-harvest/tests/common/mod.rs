//! Test harness: a San Mateo service with fakes for the network, the clock and
//! the random source. It needs no database. The shape follows the harness of
//! the core crate.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use autumn_harvest::chrono::{self, DateTime, TimeZone, Utc};
use san_mateo::clock::{ManualClock, SeqEntropy};
use san_mateo::credentials::OAuth2Credentials;
use san_mateo::transport::HttpResponse;
use san_mateo::{
    ActionContext, ActionFunction, ActionSpec, Catalog, ConnectionKey, Credentials,
    EnvironmentConfig, Error, ImportConnection, KeyRing, MemoryStore, MemoryTransport,
    NewIntegration, ProxyRequest, ProxySettings, Registry, SanMateo, Secret, SyncContext,
    SyncFunction, SyncSpec,
};
use serde_json::{Map, Value, json};

pub const SECRET: &str = "sk_test_dev_0123456789";
pub const SERVER: &str = "https://sm.example.com";
pub const ENV: &str = "dev";

/// Secrets that must never leave San Mateo.
pub const ACCESS_TOKEN: &str = "at-secret-1";
pub const REFRESH_TOKEN: &str = "rt-secret-1";
pub const NEW_ACCESS_TOKEN: &str = "at-secret-2";
pub const NEW_REFRESH_TOKEN: &str = "rt-secret-2";
pub const API_KEY: &str = "key-secret-1";
pub const CLIENT_SECRET: &str = "crm-client-secret";

pub const TOKEN_URL: &str = "https://auth.crm.example.com/token";
pub const CRM: &str = "https://api.crm.example.com";
pub const KEYED: &str = "https://api.keyed.example.com";

pub const CATALOG: &str = r"
crm:
    display_name: Test CRM
    auth_mode: OAUTH2
    authorization_url: https://auth.crm.example.com/authorize
    token_url: https://auth.crm.example.com/token
    proxy:
        base_url: https://api.crm.example.com
    docs: https://crm.example.com/docs
keyed:
    display_name: Keyed API
    auth_mode: API_KEY
    proxy:
        base_url: https://api.keyed.example.com
        headers:
            x-api-key: ${apiKey}
    docs: https://keyed.example.com/docs
";

pub struct Harness {
    pub san: SanMateo,
    pub http: MemoryTransport,
    pub clock: Arc<ManualClock>,
    pub store: Arc<MemoryStore>,
    pub contacts_runs: Arc<AtomicUsize>,
}

pub fn start() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

pub fn crm() -> ConnectionKey {
    ConnectionKey::new("crm", "c-1")
}

pub fn keyed() -> ConnectionKey {
    ConnectionKey::new("keyed", "k-1")
}

type Fut<'a, T> = Pin<Box<dyn Future<Output = san_mateo::Result<T>> + Send + 'a>>;

/// A sync that saves the contacts of `GET /v3/contacts`.
pub struct Contacts {
    pub runs: Arc<AtomicUsize>,
}

impl SyncFunction for Contacts {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "contacts".into(),
            integration: "crm".into(),
            models: vec!["Contact".into()],
            frequency: Duration::from_secs(3_600),
            auto_start: false,
        }
    }

    fn run<'a, 'b, 'c>(&'a self, ctx: &'b SyncContext) -> Fut<'c, ()>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async move {
            self.runs.fetch_add(1, Ordering::SeqCst);
            let records = ctx.get("/v3/contacts").await?.json::<Vec<Value>>()?;
            ctx.batch_save("Contact", records).await?;
            Ok(())
        })
    }
}

/// A sync that always fails.
pub struct Broken;

impl SyncFunction for Broken {
    fn spec(&self) -> SyncSpec {
        SyncSpec {
            name: "broken".into(),
            integration: "crm".into(),
            models: vec!["X".into()],
            frequency: Duration::from_secs(60),
            auto_start: false,
        }
    }

    fn run<'a, 'b, 'c>(&'a self, _ctx: &'b SyncContext) -> Fut<'c, ()>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async move { Err(Error::Function("upstream shape changed".into())) })
    }
}

/// An action that creates a contact with `POST /v3/contacts`.
pub struct CreateContact;

impl ActionFunction for CreateContact {
    fn spec(&self) -> ActionSpec {
        ActionSpec {
            name: "create_contact".into(),
            integration: "crm".into(),
        }
    }

    fn run<'a, 'b, 'c>(&'a self, ctx: &'b ActionContext, input: Value) -> Fut<'c, Value>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async move {
            let reply = ctx
                .proxy(ProxyRequest::new("POST", "/v3/contacts").json(&input))
                .await?;
            reply.json::<Value>()
        })
    }
}

/// An action that always fails.
pub struct Refuse;

impl ActionFunction for Refuse {
    fn spec(&self) -> ActionSpec {
        ActionSpec {
            name: "refuse".into(),
            integration: "crm".into(),
        }
    }

    fn run<'a, 'b, 'c>(&'a self, _ctx: &'b ActionContext, _input: Value) -> Fut<'c, Value>
    where
        'a: 'c,
        'b: 'c,
        Self: 'c,
    {
        Box::pin(async move { Err(Error::Function("the action refuses".into())) })
    }
}

pub fn token_reply(access: &str, refresh: &str, expires_in: i64) -> HttpResponse {
    HttpResponse::json(
        200,
        &json!({
            "access_token": access,
            "refresh_token": refresh,
            "token_type": "bearer",
            "expires_in": expires_in
        }),
    )
}

/// A service with the `crm` and `keyed` integrations, an OAuth connection
/// `crm/c-1` that expires in one hour, and an API key connection `keyed/k-1`.
pub async fn ready() -> Harness {
    let http = MemoryTransport::new();
    let clock = Arc::new(ManualClock::new(start()));
    let store = Arc::new(MemoryStore::new());
    let contacts_runs = Arc::new(AtomicUsize::new(0));
    let registry = Registry::new()
        .sync(Contacts {
            runs: contacts_runs.clone(),
        })
        .sync(Broken)
        .action(CreateContact)
        .action(Refuse);
    let san = SanMateo::builder()
        .server_url(SERVER)
        .environment(EnvironmentConfig::new(ENV, SECRET))
        .catalog(Catalog::from_yaml_str(CATALOG).unwrap())
        .store(store.clone())
        .key_ring(KeyRing::new("k1", [42; 32]).unwrap())
        .transport(Arc::new(http.clone()))
        .clock(clock.clone())
        .entropy(Arc::new(SeqEntropy::default()))
        .proxy_settings(ProxySettings {
            initial_backoff_ms: 0,
            max_wait_ms: 5_000,
            ..ProxySettings::default()
        })
        .registry(registry)
        .build()
        .unwrap();
    for (key, client_id, client_secret) in [
        ("crm", Some("crm-client"), Some(CLIENT_SECRET)),
        ("keyed", None, None),
    ] {
        san.create_integration(
            ENV,
            NewIntegration {
                unique_key: key.to_owned(),
                provider: key.to_owned(),
                client_id: client_id.map(str::to_owned),
                client_secret: client_secret.map(Secret::new),
                ..NewIntegration::default()
            },
        )
        .await
        .unwrap();
    }
    import(
        &san,
        &crm(),
        Credentials::OAuth2(OAuth2Credentials {
            access_token: Secret::new(ACCESS_TOKEN),
            refresh_token: Some(Secret::new(REFRESH_TOKEN)),
            expires_at: Some(start() + chrono::Duration::hours(1)),
            raw: json!({}),
        }),
    )
    .await;
    import(
        &san,
        &keyed(),
        Credentials::ApiKey {
            api_key: Secret::new(API_KEY),
        },
    )
    .await;
    Harness {
        san,
        http,
        clock,
        store,
        contacts_runs,
    }
}

async fn import(san: &SanMateo, key: &ConnectionKey, credentials: Credentials) {
    san.import_connection(
        ENV,
        ImportConnection {
            connection_id: Some(key.connection_id.clone()),
            provider_config_key: key.provider_config_key.clone(),
            credentials,
            connection_config: Map::new(),
            metadata: Map::new(),
            tags: std::collections::BTreeMap::new(),
            end_user: None,
            webhook_url_override: None,
        },
    )
    .await
    .unwrap();
}

/// Asserts that a text holds no secret of the harness.
pub fn assert_no_secrets(text: &str) {
    for secret in [
        ACCESS_TOKEN,
        REFRESH_TOKEN,
        NEW_ACCESS_TOKEN,
        NEW_REFRESH_TOKEN,
        API_KEY,
        CLIENT_SECRET,
        SECRET,
    ] {
        assert!(!text.contains(secret), "a secret leaked: {text}");
    }
}
