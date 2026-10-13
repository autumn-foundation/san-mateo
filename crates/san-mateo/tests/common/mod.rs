//! Test harness: a service with fakes for the network, the clock and the
//! random source.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use san_mateo::clock::{Clock, ManualClock, SeqEntropy};
use san_mateo::credentials::OAuth2Credentials;
use san_mateo::transport::HttpResponse;
use san_mateo::webhook::WebhookSettings;
use san_mateo::{
    Catalog, ConnectOptions, ConnectSessionInput, Connection, ConnectionKey, Credentials, EndUser,
    EnvironmentConfig, ImportConnection, KeyRing, MemoryStore, MemoryTransport, NewIntegration,
    ProxySettings, Registry, SanMateo, Secret,
};
use serde_json::{Map, Value, json};

pub const SECRET: &str = "sk_test_dev_0123456789";
pub const PROD_SECRET: &str = "sk_test_prod_0123456789";
pub const SERVER: &str = "https://sm.example.com";
pub const HOOK: &str = "https://hooks.example.com/sm";
pub const WHSEC: &str = "whsec_test";

pub const CATALOG: &str = r"
crm:
    display_name: Test CRM
    auth_mode: OAUTH2
    categories: [crm]
    authorization_url: https://auth.crm.example.com/authorize
    token_url: https://auth.crm.example.com/token
    authorization_params:
        prompt: consent
    scope_separator: ','
    token_response_metadata: [instance_id]
    proxy:
        base_url: https://api.crm.example.com
        headers:
            x-crm-version: '2024-01'
        retry:
            at: [x-reset]
            remaining: x-remaining
            error_code: ['403']
        paginate:
            type: cursor
            cursor_path_in_response: next
            cursor_name_in_request: cursor
            limit_name_in_request: limit
            response_path: results
    docs: https://crm.example.com/docs
zen:
    display_name: Zen Desk
    auth_mode: OAUTH2
    authorization_url: https://${connectionConfig.subdomain}.zen.example.com/oauth/authorize
    token_url: https://${connectionConfig.subdomain}.zen.example.com/oauth/token
    disable_pkce: true
    token_request_auth_method: basic
    body_format: json
    token_params:
        expires_in: '1800'
    connection_config:
        subdomain:
            type: string
            title: Subdomain
            pattern: '^[a-z0-9-]+$'
    proxy:
        base_url: https://${connectionConfig.subdomain}.zen.example.com/api
    docs: https://zen.example.com/docs
keyed:
    display_name: Keyed API
    auth_mode: API_KEY
    proxy:
        base_url: https://api.keyed.example.com
        headers:
            x-api-key: ${apiKey}
        verification:
            method: GET
            endpoints: [/v1/me]
    docs: https://keyed.example.com/docs
basic:
    display_name: Basic API
    auth_mode: BASIC
    proxy:
        base_url: https://api.basic.example.com
    docs: https://basic.example.com/docs
machine:
    display_name: Machine API
    auth_mode: OAUTH2_CC
    token_url: https://auth.machine.example.com/token
    proxy:
        base_url: https://api.machine.example.com
    docs: https://machine.example.com/docs
open:
    display_name: <b>Open</b> & Free
    auth_mode: NONE
    proxy:
        base_url: https://api.open.example.com
    docs: https://open.example.com/docs
twostep:
    display_name: Two Step
    auth_mode: TWO_STEP
";

pub struct Harness {
    pub san: SanMateo,
    pub http: MemoryTransport,
    pub clock: Arc<ManualClock>,
    pub store: Arc<MemoryStore>,
    pub keys: KeyRing,
}

pub fn keys() -> KeyRing {
    KeyRing::new("k1", [42; 32]).unwrap()
}

pub fn webhooks() -> WebhookSettings {
    WebhookSettings {
        primary_url: Some(HOOK.to_owned()),
        secondary_url: None,
        signing_secret: Some(Secret::new(WHSEC)),
        retries: 0,
        initial_backoff_ms: 0,
        ..WebhookSettings::default()
    }
}

pub fn start() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

pub fn harness_with(registry: Registry) -> Harness {
    harness_with_catalog(registry, CATALOG)
}

/// A harness with the catalog `yaml`.
pub fn harness_with_catalog(registry: Registry, yaml: &str) -> Harness {
    let http = MemoryTransport::new();
    http.reply("POST", HOOK, HttpResponse::new(200, ""));
    let clock = Arc::new(ManualClock::new(start()));
    let store = Arc::new(MemoryStore::new());
    let mut dev = EnvironmentConfig::new("dev", SECRET);
    dev.webhooks = webhooks();
    let san = SanMateo::builder()
        .server_url(SERVER)
        .environment(dev)
        .environment(EnvironmentConfig::new("prod", PROD_SECRET))
        .catalog(Catalog::from_yaml_str(yaml).unwrap())
        .store(store.clone())
        .key_ring(keys())
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
    Harness {
        san,
        http,
        clock,
        store,
        keys: keys(),
    }
}

pub fn harness() -> Harness {
    harness_with(Registry::new())
}

/// A harness with one integration for each test provider.
pub async fn ready_with(registry: Registry) -> Harness {
    let h = harness_with(registry);
    for (key, provider, id, secret, scopes) in [
        (
            "crm",
            "crm",
            Some("crm-client"),
            Some("crm-secret"),
            vec!["read", "write"],
        ),
        ("zen", "zen", Some("zen-client"), Some("zen-secret"), vec![]),
        ("keyed", "keyed", None, None, vec![]),
        ("basic", "basic", None, None, vec![]),
        ("machine", "machine", None, None, vec!["jobs"]),
        ("open", "open", None, None, vec![]),
        ("twostep", "twostep", None, None, vec![]),
    ] {
        h.san
            .create_integration(
                "dev",
                NewIntegration {
                    unique_key: key.to_owned(),
                    provider: provider.to_owned(),
                    client_id: id.map(str::to_owned),
                    client_secret: secret.map(Secret::new),
                    scopes: scopes.into_iter().map(str::to_owned).collect(),
                    ..NewIntegration::default()
                },
            )
            .await
            .unwrap();
    }
    h
}

pub async fn ready() -> Harness {
    ready_with(Registry::new()).await
}

pub fn end_user() -> EndUser {
    EndUser {
        id: "user-1".to_owned(),
        email: Some("user-1@example.com".to_owned()),
        ..EndUser::default()
    }
}

pub async fn session(h: &Harness) -> String {
    session_with(
        h,
        ConnectSessionInput {
            end_user: Some(end_user()),
            tags: BTreeMap::from([("plan".to_owned(), "pro".to_owned())]),
            ..ConnectSessionInput::default()
        },
    )
    .await
}

pub async fn session_with(h: &Harness, input: ConnectSessionInput) -> String {
    h.san
        .create_connect_session("dev", input)
        .await
        .unwrap()
        .token
        .expose()
        .to_owned()
}

pub fn query_of(url: &str) -> BTreeMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

pub fn token_reply(access: &str, refresh: Option<&str>, expires_in: Option<i64>) -> HttpResponse {
    let mut v = json!({"access_token": access, "token_type": "bearer", "instance_id": "inst-7"});
    if let Some(r) = refresh {
        v["refresh_token"] = json!(r);
    }
    if let Some(e) = expires_in {
        v["expires_in"] = json!(e);
    }
    HttpResponse::json(200, &v)
}

/// Runs the OAuth flow for `crm` and returns the connection.
pub async fn connect_crm(
    h: &Harness,
    access: &str,
    refresh: Option<&str>,
    expires_in: Option<i64>,
) -> Connection {
    h.http.reply(
        "POST",
        "https://auth.crm.example.com/token",
        token_reply(access, refresh, expires_in),
    );
    let token = session(h).await;
    let url = h
        .san
        .oauth_start(&token, "crm", ConnectOptions::default())
        .await
        .unwrap();
    let q = query_of(&url);
    let mut cb = BTreeMap::new();
    cb.insert("code".to_owned(), "code-1".to_owned());
    cb.insert("state".to_owned(), q["state"].clone());
    h.san.oauth_callback(&cb).await.unwrap()
}

/// Returns the webhook bodies sent so far, as JSON.
pub fn webhooks_sent(h: &Harness) -> Vec<Value> {
    h.http
        .requests_to(HOOK)
        .into_iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

pub fn now(h: &Harness) -> chrono::DateTime<Utc> {
    h.clock.now()
}

/// A provider with `authorization_method: header` and optional template values.
pub const SHOP: &str = r"
shop:
    display_name: Shop
    auth_mode: OAUTH2
    authorization_url: https://auth.shop.example.com/authorize
    token_url: https://auth.shop.example.com/token
    authorization_method: header
    proxy:
        base_url: https://api.shop.example.com
        headers:
            x-api-key: ${clientId}:${clientSecret}
            x-login-customer-id: ${connectionConfig.login_customer_id}
        query:
            account: ${connectionConfig.account}
    docs: https://shop.example.com/docs
";

/// A harness with the `shop` catalog and a `shop` integration.
pub async fn shop_harness() -> Harness {
    let h = harness_with_catalog(Registry::new(), SHOP);
    h.san
        .create_integration(
            "dev",
            NewIntegration {
                unique_key: "shop".to_owned(),
                provider: "shop".to_owned(),
                client_id: Some("shop-client".to_owned()),
                client_secret: Some(Secret::new("shop-secret")),
                ..NewIntegration::default()
            },
        )
        .await
        .unwrap();
    h
}

/// Imports a `shop` connection with the access token `at-1`.
pub async fn shop_connection(
    h: &Harness,
    id: &str,
    connection_config: Value,
    refresh: Option<&str>,
) -> ConnectionKey {
    let Value::Object(connection_config) = connection_config else {
        panic!("the connection config must be an object");
    };
    h.san
        .import_connection(
            "dev",
            ImportConnection {
                connection_id: Some(id.to_owned()),
                provider_config_key: "shop".to_owned(),
                credentials: Credentials::OAuth2(OAuth2Credentials {
                    access_token: "at-1".into(),
                    refresh_token: refresh.map(Into::into),
                    expires_at: None,
                    raw: json!({}),
                }),
                connection_config,
                metadata: Map::new(),
                tags: BTreeMap::new(),
                end_user: None,
                webhook_url_override: None,
            },
        )
        .await
        .unwrap()
        .key()
}
