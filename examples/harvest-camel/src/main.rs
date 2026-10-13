//! A harvest workflow calls a provider through camel routes and the
//! `san-mateo:` component.

// camel 0.57 `CamelError` is 168 bytes. The camel API returns it by value.
#![allow(clippy::result_large_err)]

use std::time::Duration;

use san_mateo::MemoryTransport;
use san_mateo::transport::HttpResponse;
use serde_json::{Value, json};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The longest time for the scenario. CI must not hang.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

const CONTACTS_URL: &str = "https://api.acme-crm.example.com/v3/contacts";
const DUPLICATE_URL: &str = "https://api.acme-crm.example.com/v3/contacts/c-2";

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

/// Runs the scenario against the Postgres database at `url`, stops the
/// worker and camel, and checks the result.
async fn run(url: &str, fake: MemoryTransport) -> Result<Value, BoxError> {
    let _ = (url, fake, RUN_TIMEOUT);
    Err("not yet".into())
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
        let summary = run(&url, fake_provider()).await.expect("the example passes");
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
