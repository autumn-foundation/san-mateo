//! Test harness: a San Mateo service with a fake network, and a camel
//! context with a `direct:in` route to a `san-mateo:` endpoint.

#![allow(
    dead_code,
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::result_large_err
)]

use std::sync::Arc;

use camel_api::{BoxProcessor, CamelError, Exchange, Message};
use camel_builder::{RouteBuilder, StepAccumulator};
use camel_component_api::{NoOpComponentContext, RuntimeObservability};
use camel_component_direct::DirectComponent;
use camel_core::CamelContext;
use san_mateo::clock::{ManualClock, SeqEntropy, from_ms};
use san_mateo::credentials::OAuth2Credentials;
use san_mateo::transport::HttpResponse;
use san_mateo::{
    Catalog, Credentials, EnvironmentConfig, ImportConnection, KeyRing, MemoryStore,
    MemoryTransport, NewIntegration, ProxySettings, SanMateo, Secret,
};
use san_mateo_camel::SanMateoComponent;
use serde_json::{Map, json};
use tower::ServiceExt;

pub const CRM_API: &str = "https://api.crm.example.com";
pub const KEYED_API: &str = "https://api.keyed.example.com";
pub const TOKEN_URL: &str = "https://auth.crm.example.com/token";

pub const CATALOG: &str = r"
crm:
    display_name: Test CRM
    auth_mode: OAUTH2
    authorization_url: https://auth.crm.example.com/authorize
    token_url: https://auth.crm.example.com/token
    proxy:
        base_url: https://api.crm.example.com
        headers:
            x-crm-version: '2024-01'
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
}

fn oauth(access: &str, refresh: &str) -> Credentials {
    Credentials::OAuth2(OAuth2Credentials {
        access_token: Secret::new(access),
        refresh_token: Some(Secret::new(refresh)),
        expires_at: None,
        raw: json!({}),
    })
}

async fn import(san: &SanMateo, env: &str, key: &str, id: &str, credentials: Credentials) {
    san.import_connection(
        env,
        ImportConnection {
            connection_id: Some(id.to_owned()),
            provider_config_key: key.to_owned(),
            credentials,
            connection_config: Map::new(),
            metadata: Map::new(),
            tags: san_mateo::Tags::new(),
            end_user: None,
            webhook_url_override: None,
        },
    )
    .await
    .unwrap();
}

async fn integration(san: &SanMateo, env: &str, key: &str, oauth: bool) {
    san.create_integration(
        env,
        NewIntegration {
            unique_key: key.to_owned(),
            provider: key.to_owned(),
            client_id: oauth.then(|| "crm-client".to_owned()),
            client_secret: oauth.then(|| Secret::new("crm-secret")),
            ..NewIntegration::default()
        },
    )
    .await
    .unwrap();
}

/// A service with two environments.
///
/// - `dev`: integration `crm` (`OAuth2`) with connection `c1` (token `at-1`),
///   and integration `keyed` (API key) with connection `k1` (key `key-1`).
/// - `prod`: integration `crm` with connection `c1` (token `at-prod`).
pub async fn harness() -> Harness {
    let http = MemoryTransport::new();
    let san = SanMateo::builder()
        .server_url("https://sm.example.com")
        .environment(EnvironmentConfig::new("dev", "sk_test_dev_0123456789"))
        .environment(EnvironmentConfig::new("prod", "sk_test_prod_0123456789"))
        .catalog(Catalog::from_yaml_str(CATALOG).unwrap())
        .store(Arc::new(MemoryStore::new()))
        .key_ring(KeyRing::new("k1", [42; 32]).unwrap())
        .transport(Arc::new(http.clone()))
        .clock(Arc::new(ManualClock::new(from_ms(1_700_000_000_000))))
        .entropy(Arc::new(SeqEntropy::default()))
        .proxy_settings(ProxySettings {
            initial_backoff_ms: 0,
            max_wait_ms: 1_000,
            ..ProxySettings::default()
        })
        .build()
        .unwrap();
    integration(&san, "dev", "crm", true).await;
    integration(&san, "dev", "keyed", false).await;
    integration(&san, "prod", "crm", true).await;
    import(&san, "dev", "crm", "c1", oauth("at-1", "rt-1")).await;
    import(
        &san,
        "dev",
        "keyed",
        "k1",
        Credentials::ApiKey {
            api_key: Secret::new("key-1"),
        },
    )
    .await;
    import(&san, "prod", "crm", "c1", oauth("at-prod", "rt-prod")).await;
    Harness { san, http }
}

/// Starts a camel context with `direct:in` to `to`. The component default
/// environment is `dev`.
pub async fn start(h: &Harness, to: &str) -> CamelContext {
    start_with(SanMateoComponent::new(h.san.clone()).environment("dev"), to)
        .await
        .unwrap()
}

/// Starts a camel context with `direct:in` to `to`.
pub async fn start_with(
    component: SanMateoComponent,
    to: &str,
) -> Result<CamelContext, CamelError> {
    let mut camel = CamelContext::builder().build().await?;
    camel.register_component(DirectComponent::new());
    camel.register_component(component);
    let route = RouteBuilder::from("direct:in")
        .route_id("to-san-mateo")
        .to(to)
        .build()?;
    camel.add_route_definition(route).await?;
    camel.start().await?;
    Ok(camel)
}

/// Sends one message to `direct:in` and waits for the result.
pub async fn send(camel: &CamelContext, message: Message) -> Result<Exchange, CamelError> {
    let direct = camel.registry().get_or_err("direct")?;
    let endpoint = direct.create_endpoint("direct:in", camel)?;
    let rt: Arc<dyn RuntimeObservability> = Arc::new(NoOpComponentContext);
    let producer: BoxProcessor = endpoint.create_producer(rt, &camel.producer_context())?;
    let mut exchange = producer.oneshot(Exchange::new(message)).await?;
    let error = exchange.error.take();
    error.map_or(Ok(exchange), Err)
}

/// Stops a camel context.
pub async fn stop(mut camel: CamelContext) {
    camel.stop().await.unwrap();
}

/// A JSON reply.
pub fn json_reply(status: u16, value: &serde_json::Value) -> HttpResponse {
    HttpResponse::json(status, value)
}

/// A copy of `harvest_camel::is_retryable` (harvest-camel `src/route.rs`).
/// harvest-camel uses camel 0.55, so these tests cannot depend on it.
pub fn harvest_retryable(err: &CamelError) -> bool {
    match err {
        CamelError::ValidationError(_)
        | CamelError::TypeConversionFailed(_)
        | CamelError::InvalidUri(_)
        | CamelError::EndpointUri(_)
        | CamelError::Config(_)
        | CamelError::ConfigValidation(_)
        | CamelError::ComponentNotFound(_)
        | CamelError::EndpointCreationFailed(_)
        | CamelError::EndpointCreationFailedWithSource(..)
        | CamelError::Unauthenticated(_)
        | CamelError::Unauthorized(_)
        | CamelError::UnsupportedMediaType { .. }
        | CamelError::NotAcceptable { .. }
        | CamelError::StreamLimitExceeded(_)
        | CamelError::AlreadyConsumed => false,
        CamelError::HttpOperationFailed { status_code, .. } => {
            !(400..500).contains(status_code) || matches!(status_code, 408 | 425 | 429)
        }
        _ => true,
    }
}
