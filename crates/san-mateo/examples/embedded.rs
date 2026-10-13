//! Embed San Mateo in your own axum app.
//!
//! ```text
//! backend ─► /san-mateo/*            San Mateo REST API (nested router)
//! backend ─► /contacts/{connection}  your handler ─► san.proxy(..) ─► provider
//! ```
//!
//! The example makes an integration and connects an OAuth2 account over the
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
//! cargo run -p san-mateo --example embedded
//! ```

use std::time::Duration;

use san_mateo::MemoryTransport;
use san_mateo::transport::HttpResponse;
use serde_json::{Value, json};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The longest time for the scenario. CI must not hang.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

const TOKEN_URL: &str = "https://auth.acme-crm.example.com/token";
const CONTACTS_URL: &str = "https://api.acme-crm.example.com/v3/contacts";
const HOOK_URL: &str = "https://backend.example.com/san-mateo-hooks";

/// Fake tokens. The checks make sure that no output shows them.
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
        HttpResponse::json(200, &json!({"results": [{"id": "c-1", "name": "Ada"}]})),
    );
    fake.reply("POST", HOOK_URL, HttpResponse::new(200, ""));
    fake
}

/// Runs the scenario on `fake`, stops all parts, and checks the result.
async fn run(fake: MemoryTransport) -> Result<Value, BoxError> {
    let _ = fake;
    Err("not yet".into())
}

/// Compares the summary with the expected values.
fn check(summary: &Value) -> Result<(), String> {
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
        "webhook": {"type": "auth", "operation": "creation", "success": true, "signature_valid": true},
        "stopped_cleanly": true,
    });
    if *summary != expected {
        return Err(format!("the summary {summary} is not {expected}"));
    }
    let text = summary.to_string();
    if let Some(secret) = FAKE_SECRETS.iter().find(|s| text.contains(*s)) {
        return Err(format!("the summary shows the secret {secret}"));
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
    async fn a_wrong_reply_fails_the_check() {
        // With no refresh reply, the fake repeats the first token reply. So
        // the upstream call gets the old token, and the check must fail.
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
            HttpResponse::json(200, &json!({"results": []})),
        );
        fake.reply("POST", HOOK_URL, HttpResponse::new(200, ""));
        let err = run(fake).await.expect_err("the check finds the old token");
        assert!(err.to_string().contains("is not"), "{err}");
    }
}
