//! A harvest workflow calls a provider through camel routes and the
//! `san-mateo:` component.
//!
//! ```text
//! workflow sync_contact (harvest worker, Postgres)
//!   1. activity create_contact ─► direct:crm-create ─► san-mateo:acme-crm  POST /v3/contacts      503, then 201
//!   2. activity find_duplicate ─► direct:crm-find   ─► san-mateo:acme-crm  GET  /v3/contacts/c-2  404
//! ```
//!
//! harvest-camel calls the routes from activities. It changes a camel error
//! into a harvest failure. A 503 is retryable, so harvest retries the
//! create. The create route sends the harvest idempotency key to the
//! provider, so the retry is safe. A 404 is not retryable, so harvest does
//! not retry the find. The workflow reads the 404 as "no duplicate". The
//! example checks the result. It returns an error when the result is wrong.
//!
//! The harvest-camel example `order_flow` shows the other direction: a camel
//! route starts a harvest workflow.
//!
//! `MemoryTransport` is the fake provider. For production: remove
//! `.transport(..)`, use `PgStore` (all workers must share one store), and
//! read the keys from a secret store. Each run uses its own queue, so you can
//! run it many times on one database.
//!
//! ```sh
//! DATABASE_URL=postgres://user:password@localhost/db \
//!   cargo run --locked --manifest-path examples/harvest-camel/Cargo.toml
//! ```

// camel 0.57 `CamelError` is 168 bytes. The camel API returns it by value.
#![allow(clippy::result_large_err)]

use std::sync::Arc;
use std::time::Duration;

use autumn_harvest::StartWorkflowParams;
use autumn_harvest::failure::ActivityFailure;
use autumn_harvest::migrate;
use autumn_harvest::pool::{DbRole, EngineDbTimeouts, engine_pool};
use autumn_harvest::prelude::*;
use autumn_harvest::worker::{DbPool, Worker};
use camel_api::{Body, CamelError, Exchange};
use camel_builder::{RouteBuilder, StepAccumulator};
use camel_component_direct::DirectComponent;
use camel_core::{CamelContext, RouteDefinition};
use harvest_camel::{CamelHandle, HEADER_IDEMPOTENCY_KEY, call_route_from_activity};
use san_mateo::clock::{Entropy, OsEntropy};
use san_mateo::credentials::OAuth2Credentials;
use san_mateo::transport::HttpResponse;
use san_mateo::{
    Catalog, Credentials, EnvironmentConfig, ImportConnection, KeyRing, MemoryTransport,
    NewIntegration, SanMateo, Secret,
};
use san_mateo_camel::SanMateoComponent;
use serde_json::{Map, Value, json};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The longest time for the scenario. CI must not hang.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

const ENV: &str = "dev";
const INTEGRATION: &str = "acme-crm";
const CREATE: &str = "direct:crm-create";
const FIND: &str = "direct:crm-find";
const CONTACTS_URL: &str = "https://api.acme-crm.example.com/v3/contacts";
const DUPLICATE_URL: &str = "https://api.acme-crm.example.com/v3/contacts/c-2";

/// The provider, in the Nango `providers.yaml` format.
const CATALOG: &str = r"
acme-crm:
    display_name: Acme CRM
    auth_mode: OAUTH2
    authorization_url: https://auth.acme-crm.example.com/authorize
    token_url: https://auth.acme-crm.example.com/token
    proxy:
        base_url: https://api.acme-crm.example.com
    docs: https://acme-crm.example.com/docs
";

/// Your workflow. Workflow code must be deterministic, so only the
/// activities call camel.
#[workflow]
async fn sync_contact(ctx: &WorkflowContext, contact: Value) -> HarvestResult<Value> {
    // Keep the activities on the queue of this run. With no queue, harvest
    // sends an activity to "default".
    let queue = ctx.queue_name().to_owned();
    let created: Value = ctx
        .execute_activity_with_opts(&create_contact_info(), &contact, Some(&queue), None, None)
        .await?;
    let found = ctx
        .execute_activity_with_opts::<_, Value>(
            &find_duplicate_info(),
            Value::Null,
            Some(&queue),
            None,
            None,
        )
        .await;
    let duplicate = match found {
        Ok(_) => true,
        // harvest-camel puts the HTTP status in the failure details.
        Err(HarvestError::ActivityFailed {
            details: Some(d), ..
        }) if d["status_code"] == 404 => false,
        Err(e) => return Err(e),
    };
    Ok(json!({"created": created["id"], "duplicate": duplicate}))
}

/// Sends the contact to the create route. harvest retries a retryable
/// failure, for example a 503.
#[activity(start_to_close = "30s", retry = RetryPolicy::exponential(3, Duration::from_millis(200)))]
async fn create_contact(ctx: &ActivityContext, contact: Value) -> Result<Value, ActivityFailure> {
    call_route_from_activity(ctx, CREATE, contact).await
}

/// Asks the find route for a duplicate. A 404 is not retryable.
#[activity(start_to_close = "30s", retry = RetryPolicy::exponential(3, Duration::from_millis(200)))]
async fn find_duplicate(ctx: &ActivityContext, input: Value) -> Result<Value, ActivityFailure> {
    call_route_from_activity(ctx, FIND, input).await
}

/// The camel routes. Do not add camel redelivery: harvest retries the
/// activity.
fn routes() -> Result<Vec<RouteDefinition>, CamelError> {
    let create = RouteBuilder::from(CREATE)
        .route_id("crm-create")
        // harvest-camel sets `HarvestIdempotencyKey`. It is the same for each
        // retry. The provider gets it, so it can find a duplicate.
        .set_header_fn("SanMateoProxy-Idempotency-Key", |ex: &Exchange| {
            ex.input
                .header(HEADER_IDEMPOTENCY_KEY)
                .cloned()
                .unwrap_or(Value::Null)
        })
        // The URI fixes the tenant. `allowHeaderOverrides` stays `false`, so
        // a header cannot change the connection.
        .to("san-mateo:acme-crm?connectionId=acme&method=POST&path=/v3/contacts")
        .build()?;
    let find = RouteBuilder::from(FIND)
        .route_id("crm-find")
        // A GET sends no body.
        .set_body(Body::Empty)
        .to("san-mateo:acme-crm?connectionId=acme&path=/v3/contacts/c-2")
        .build()?;
    Ok(vec![create, find])
}

/// The fake provider. The first `POST` fails with 503. The duplicate does
/// not exist.
fn fake_provider() -> MemoryTransport {
    let fake = MemoryTransport::new();
    fake.reply("POST", CONTACTS_URL, HttpResponse::new(503, "busy"));
    fake.reply(
        "POST",
        CONTACTS_URL,
        HttpResponse::json(201, &json!({"id": "c-1", "name": "Ada"})),
    );
    fake.reply(
        "GET",
        DUPLICATE_URL,
        HttpResponse::json(404, &json!({"error": "not_found"})),
    );
    fake
}

/// Builds the service with one integration and one imported connection.
async fn san_mateo(fake: &MemoryTransport) -> Result<SanMateo, BoxError> {
    let mut key = [0_u8; 32];
    OsEntropy.fill(&mut key);
    let secret_key = format!("sk_example_{}", OsEntropy.hex(16));
    let san = SanMateo::builder()
        .environment(EnvironmentConfig::new(ENV, secret_key))
        .key_ring(KeyRing::new("k1", key)?)
        .catalog(Catalog::from_yaml_str(CATALOG)?)
        .transport(Arc::new(fake.clone()))
        .build()?;
    san.create_integration(
        ENV,
        NewIntegration {
            unique_key: INTEGRATION.to_owned(),
            provider: INTEGRATION.to_owned(),
            client_id: Some("acme-client".to_owned()),
            client_secret: Some(Secret::new(OsEntropy.hex(16))),
            ..NewIntegration::default()
        },
    )
    .await?;
    // A token with no expiry: San Mateo never refreshes it.
    let credentials = Credentials::OAuth2(OAuth2Credentials {
        access_token: Secret::new("at-1"),
        refresh_token: None,
        expires_at: None,
        raw: Value::Null,
    });
    san.import_connection(
        ENV,
        ImportConnection {
            connection_id: Some("acme".to_owned()),
            provider_config_key: INTEGRATION.to_owned(),
            credentials,
            connection_config: Map::new(),
            metadata: Map::new(),
            tags: san_mateo::Tags::new(),
            end_user: None,
            webhook_url_override: None,
        },
    )
    .await?;
    Ok(san)
}

/// Runs the scenario against the Postgres database at `url`, stops the
/// worker and camel, and checks the result.
async fn run(url: &str, fake: MemoryTransport) -> Result<Value, BoxError> {
    // In production, apply the harvest migrations before you start.
    migrate::apply(url, &migrate::embedded()).await?;
    let pool = engine_pool(url, 8, DbRole::Hot, &EngineDbTimeouts::default())?;

    // camel: the San Mateo component and the routes.
    let mut camel = CamelContext::builder().build().await?;
    camel.register_component(DirectComponent::new());
    camel.register_component(SanMateoComponent::new(san_mateo(&fake).await?).environment(ENV));
    for route in routes()? {
        camel.add_route_definition(route).await?;
    }
    camel.start().await?;

    let output = with_worker(&camel, &pool, url).await;

    // Stop camel in all cases.
    camel.stop().await?;
    let summary = summarize(&output?, &fake);
    check(&summary)?;
    Ok(summary)
}

/// Starts a harvest worker, runs one workflow, and stops the worker.
async fn with_worker(camel: &CamelContext, pool: &DbPool, url: &str) -> Result<Value, BoxError> {
    // Activities get this handle. Resolve the routes before they run.
    let handle = CamelHandle::new();
    handle.resolve(camel, CREATE)?;
    handle.resolve(camel, FIND)?;

    // A queue for this run only. Two runs on one database then do not take
    // the tasks of each other.
    let run_id = OsEntropy.hex(8);
    let queue = format!("san-mateo-example-{run_id}");
    let mut worker_config = WorkerConfig::default()
        .with_queues([queue.as_str()])
        .with_notification_database_url(url);
    // A short drain, so that a failed run stops fast.
    worker_config.shutdown_timeout = Duration::from_secs(5);
    let (registry, _dags, _schedules, worker_config) = HarvestBuilder::new()
        .workflows(vec![sync_contact_info()])
        .activities(vec![create_contact_info(), find_duplicate_info()])
        .state(handle)
        .worker(worker_config)
        .try_build()?
        .into_worker_parts();
    let worker = Arc::new(Worker::new(worker_config.into(), Arc::new(registry))?);
    let running = tokio::spawn({
        let (worker, pool) = (Arc::clone(&worker), pool.clone());
        async move { worker.run(&pool).await }
    });

    let result =
        tokio::time::timeout(RUN_TIMEOUT, start_and_wait(pool, url, &queue, &run_id)).await;

    // Stop in all cases. `run` returns after the drain.
    worker.shutdown();
    running.await?;
    result.map_err(|_| format!("the workflow did not end in {RUN_TIMEOUT:?}"))?
}

/// Starts one workflow and waits for its result.
async fn start_and_wait(
    pool: &DbPool,
    url: &str,
    queue: &str,
    run_id: &str,
) -> Result<Value, BoxError> {
    let client = WorkflowHandleClient::single(pool.clone(), url.to_owned());
    let workflow_id = format!("sync-contact-{run_id}");
    let shard = client.pick_shard_for_new_workflow("sync_contact", &workflow_id);
    let params = StartWorkflowParams::new(
        "sync_contact",
        &workflow_id,
        ExecutionId::new_for_shard(shard),
        json!({"name": "Ada"}),
        queue,
    );
    let started = {
        let mut conn = pool.get().await?;
        client.start_or_load(&mut conn, params).await?
    };
    let output = started
        .handle
        .result_raw_with_timeout(RUN_TIMEOUT)
        .await
        .map_err(|e| format!("workflow {workflow_id}: {e}"))?;
    Ok(output)
}

/// Builds the summary from the workflow output and the fake log.
fn summarize(output: &Value, fake: &MemoryTransport) -> Value {
    let posts = fake.requests_to(CONTACTS_URL);
    let gets = fake.requests_to(DUPLICATE_URL);
    let keys: Vec<Option<&str>> = posts
        .iter()
        .map(|r| r.header_value("idempotency-key"))
        .collect();
    let keys_equal = keys
        .first()
        .is_some_and(|k| k.is_some_and(|k| !k.is_empty()))
        && keys.windows(2).all(|w| w[0] == w[1]);
    let auth = posts
        .iter()
        .chain(&gets)
        .all(|r| r.header_value("authorization") == Some("Bearer at-1"));
    json!({
        "workflow": output,
        "post_calls": posts.len(),
        "post_idempotency_keys_are_equal": keys_equal,
        "auth_added_by_san_mateo": auth,
        "get_calls": gets.len(),
    })
}

fn check(summary: &Value) -> Result<(), String> {
    let expected = json!({
        "workflow": {"created": "c-1", "duplicate": false},
        "post_calls": 2,
        "post_idempotency_keys_are_equal": true,
        "auth_added_by_san_mateo": true,
        "get_calls": 1,
    });
    if *summary == expected {
        Ok(())
    } else {
        Err(format!("the summary {summary} is not {expected}"))
    }
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .ok_or("set DATABASE_URL to a Postgres URL")?;
    let summary = run(&url, fake_provider()).await?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use san_mateo::Error;

    use super::*;

    const DB_ENV: &str = "SAN_MATEO_TEST_DATABASE_URL";

    /// The test database URL. With no URL, the test skips, but not in CI.
    fn database_url() -> Option<String> {
        match std::env::var(DB_ENV) {
            Ok(url) if !url.is_empty() => Some(url),
            _ => {
                assert!(
                    std::env::var_os("CI").is_none(),
                    "CI must set {DB_ENV} to a Postgres URL"
                );
                eprintln!("skipping: set {DB_ENV} to a Postgres URL to run the example");
                None
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_create_retry_sends_one_key_and_a_404_is_not_retried() {
        let Some(url) = database_url() else { return };
        let summary = run(&url, fake_provider())
            .await
            .expect("the example passes");
        check(&summary).expect("run checks the summary");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_client_error_fails_the_run() {
        let Some(url) = database_url() else { return };
        // A 400 is not retryable, so the workflow fails at once.
        let fake = MemoryTransport::new();
        fake.reply("POST", CONTACTS_URL, HttpResponse::new(400, "bad contact"));
        let err = run(&url, fake).await.expect_err("the workflow fails");
        assert!(err.to_string().contains("workflow"), "{err}");
    }

    /// `san-mateo-camel` keeps a copy of the harvest-camel retry rule. Here,
    /// the real harvest-camel rule must give the same answer as San Mateo.
    #[test]
    fn harvest_camel_retries_what_san_mateo_retries() {
        let s = || "x".to_owned();
        let provider = |status| Error::Provider {
            code: "refresh_failed",
            status,
            message: "the provider gave an error".to_owned(),
        };
        let errors = [
            Error::Invalid(s()),
            Error::MissingField(s()),
            Error::Template(s()),
            Error::Conflict(s()),
            Error::UpstreamTooLarge,
            Error::UnknownProvider(s()),
            Error::UnknownIntegration(s()),
            Error::UnknownConnection(s()),
            Error::UnknownFunction(s()),
            Error::UnsupportedAuthMode(s()),
            Error::Crypto,
            Error::Internal(s()),
            Error::Function(s()),
            Error::ForbiddenHost(s()),
            Error::IntegrationNotAllowed(s()),
            Error::Unauthorized(s()),
            Error::InvalidConnectSession,
            Error::InvalidOAuthState,
            Error::RefreshExhausted,
            Error::RefreshCooldown,
            Error::Upstream(s()),
            Error::Store(s()),
            provider(None),
            provider(Some(302)),
            provider(Some(400)),
            provider(Some(401)),
            provider(Some(404)),
            provider(Some(408)),
            provider(Some(425)),
            provider(Some(429)),
            provider(Some(500)),
            provider(Some(503)),
            provider(Some(600)),
        ];
        for err in errors {
            let camel = san_mateo_camel::to_camel_error(&err, "POST", "san-mateo:crm/v3/contacts");
            assert_eq!(
                harvest_camel::is_retryable(&camel),
                err.is_retryable(),
                "{err:?} -> {camel:?}"
            );
        }
    }
}
