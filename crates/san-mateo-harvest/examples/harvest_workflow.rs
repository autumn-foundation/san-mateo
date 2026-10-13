//! A durable harvest workflow calls a provider through San Mateo.
//!
//! ```text
//! workflow add_contact (harvest worker, Postgres)
//!   1. activity san_mateo_proxy: POST /v3/contacts       503, then 201: harvest retries
//!   2. activity san_mateo_proxy: GET  /v3/contacts/{id}
//! ```
//!
//! The workflow input holds ids only. San Mateo adds the auth, so no token
//! goes into the harvest history. The `POST` sends the harvest idempotency
//! key to the provider, so a retry after a 503 is safe. Without the key, San
//! Mateo makes a `POST` after a 503 non-retryable. The example checks the
//! result. It returns an error when the result is wrong.
//!
//! `MemoryTransport` is the fake provider. For production: remove
//! `.transport(..)`, use `PgStore` (all workers must share one store), and
//! read the keys from a secret store. Each run uses its own queue, so you can
//! run it many times on one database.
//!
//! ```sh
//! DATABASE_URL=postgres://user:password@localhost/db \
//!   cargo run --locked -p san-mateo-harvest --example harvest_workflow
//! ```

use std::sync::Arc;
use std::time::Duration;

use autumn_harvest::StartWorkflowParams;
use autumn_harvest::migrate;
use autumn_harvest::pool::{DbRole, EngineDbTimeouts, engine_pool};
use autumn_harvest::prelude::*;
use autumn_harvest::worker::{DbPool, Worker};
use san_mateo::clock::{Entropy, OsEntropy};
use san_mateo::credentials::OAuth2Credentials;
use san_mateo::transport::HttpResponse;
use san_mateo::{
    Catalog, ConnectionKey, Credentials, EnvironmentConfig, ImportConnection, KeyRing,
    MemoryTransport, NewIntegration, SanMateo, Secret,
};
use san_mateo_harvest::{IDEMPOTENCY_KEY_HEADER, ProxyInput, ProxyOutput, san_mateo_proxy_info};
use serde_json::{Map, Value, json};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The longest time for the scenario. CI must not hang.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

const ENV: &str = "dev";
const INTEGRATION: &str = "acme-crm";
const CONNECTION: &str = "acme";
const CONTACTS_URL: &str = "https://api.acme-crm.example.com/v3/contacts";
const CONTACT_URL: &str = "https://api.acme-crm.example.com/v3/contacts/c-1";

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
/// activities call the provider.
#[workflow]
async fn add_contact(ctx: &WorkflowContext, contact: Value) -> HarvestResult<Value> {
    // Keep the activities on the queue of this run. With no queue, harvest
    // sends an activity to "default".
    let queue = ctx.queue_name().to_owned();
    let key = ConnectionKey::new(INTEGRATION, CONNECTION);

    let mut create = ProxyInput::new(ENV, &key, "POST", "/v3/contacts").json(contact);
    // The provider gets the harvest idempotency key, so a retry is safe.
    create.forward_idempotency_key = true;
    let created: ProxyOutput = ctx
        .execute_activity_with_opts(&san_mateo_proxy_info(), &create, Some(&queue), None, None)
        .await?;
    let id = created.body["id"].as_str().unwrap_or_default().to_owned();

    let find = ProxyInput::new(ENV, &key, "GET", format!("/v3/contacts/{id}"));
    let found: ProxyOutput = ctx
        .execute_activity_with_opts(&san_mateo_proxy_info(), &find, Some(&queue), None, None)
        .await?;
    Ok(json!({"created": id, "post_status": created.status, "contact": found.body}))
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
            connection_id: Some(CONNECTION.to_owned()),
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

/// The fake provider. The first `POST` fails with 503.
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
        CONTACT_URL,
        HttpResponse::json(200, &json!({"id": "c-1", "name": "Ada"})),
    );
    fake
}

/// Runs the scenario against the Postgres database at `url`, stops the
/// worker, and checks the result.
async fn run(url: &str, fake: MemoryTransport) -> Result<Value, BoxError> {
    // In production, apply the harvest migrations before you start.
    migrate::apply(url, &migrate::embedded()).await?;
    let pool = engine_pool(url, 8, DbRole::Hot, &EngineDbTimeouts::default())?;
    let san = san_mateo(&fake).await?;

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
        .workflows(vec![add_contact_info()])
        .activities(san_mateo_harvest::activities())
        .state(san)
        .worker(worker_config)
        .try_build()?
        .into_worker_parts();
    let worker = Arc::new(Worker::new(worker_config.into(), Arc::new(registry))?);
    let running = tokio::spawn({
        let (worker, pool) = (Arc::clone(&worker), pool.clone());
        async move { worker.run(&pool).await }
    });

    let result =
        tokio::time::timeout(RUN_TIMEOUT, start_and_wait(&pool, url, &queue, &run_id)).await;

    // Stop in all cases. `run` returns after the drain.
    worker.shutdown();
    running.await?;

    let output = result.map_err(|_| format!("the workflow did not end in {RUN_TIMEOUT:?}"))??;
    let summary = summarize(&output, &fake);
    check(&summary)?;
    Ok(summary)
}

/// Starts one workflow and waits for its result.
async fn start_and_wait(
    pool: &DbPool,
    url: &str,
    queue: &str,
    run_id: &str,
) -> Result<Value, BoxError> {
    let client = WorkflowHandleClient::single(pool.clone(), url.to_owned());
    let workflow_id = format!("add-contact-{run_id}");
    let shard = client.pick_shard_for_new_workflow("add_contact", &workflow_id);
    let params = StartWorkflowParams::new(
        "add_contact",
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
    let gets = fake.requests_to(CONTACT_URL);
    let keys: Vec<Option<&str>> = posts
        .iter()
        .map(|r| r.header_value(IDEMPOTENCY_KEY_HEADER))
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
        "workflow": {
            "created": "c-1",
            "post_status": 201,
            "contact": {"id": "c-1", "name": "Ada"},
        },
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
    async fn a_post_retry_sends_one_idempotency_key() {
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
}
