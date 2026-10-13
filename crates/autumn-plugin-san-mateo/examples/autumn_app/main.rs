//! An autumn-web app with the San Mateo plugin.

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    Err("not yet".into())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use autumn_plugin_san_mateo::san_mateo::MemoryTransport;
    use autumn_plugin_san_mateo::san_mateo::transport::HttpResponse;
    use serde_json::{Value, json};

    use super::BoxError;

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

    /// Runs the app in `TestApp` with the same routes and config file as
    /// `main`, drives it over HTTP and checks the result.
    async fn run(fake: MemoryTransport) -> Result<Value, BoxError> {
        let _ = (fake, RUN_TIMEOUT);
        Err("not yet".into())
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
