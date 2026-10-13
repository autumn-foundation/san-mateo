//! No secret reaches the logs (AC7).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use san_mateo::transport::HttpResponse;
use san_mateo::{ConnectOptions, GetOptions, ProxyRequest, Secret};
use tracing_subscriber::fmt::MakeWriter;

/// A log sink in memory.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Sink {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn no_secret_reaches_the_logs() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    // The test runtime has one thread, so spawned tasks log here too.
    let _guard = tracing::subscriber::set_default(subscriber);

    let h = ready().await;
    // OAuth flow and a proxy call.
    let conn = connect_crm(&h, "at-secret-1", Some("rt-secret-1"), Some(60)).await;
    h.http.reply(
        "GET",
        "https://api.crm.example.com/v3/me",
        HttpResponse::json(
            200,
            &serde_json::json!({"email": "body-secret@example.com"}),
        ),
    );
    h.san
        .proxy("dev", &conn.key(), ProxyRequest::get("/v3/me"))
        .await
        .unwrap();
    // The sweep logs a failed refresh. The reply holds a token. Move past the
    // first half of the token life, so the sweep tries.
    h.clock.advance(chrono::Duration::seconds(50));
    h.http.reply(
        "POST",
        "https://auth.crm.example.com/token",
        HttpResponse::new(
            400,
            r#"{"error":"invalid_grant","access_token":"at-reply-secret"}"#,
        ),
    );
    let _ = h
        .san
        .refresh_due(std::time::Duration::from_secs(1_800), 10)
        .await;
    let _ = h
        .san
        .get_connection(
            "dev",
            &conn.key(),
            GetOptions {
                refresh_if_needed: true,
                force_refresh: true,
            },
        )
        .await;
    // An API key connection sends an `auth` webhook to a URL with a secret in
    // its query. The delivery fails and San Mateo logs it.
    let hook = "https://hooks.example.com/override?token=hook-secret-1";
    h.http.reply("POST", hook, HttpResponse::new(500, "down"));
    let token = session_with(
        &h,
        san_mateo::ConnectSessionInput {
            webhook_url_override: Some(hook.to_owned()),
            ..san_mateo::ConnectSessionInput::default()
        },
    )
    .await;
    h.http.reply(
        "GET",
        "https://api.keyed.example.com/v1/me",
        HttpResponse::new(200, "{}"),
    );
    let _ = h
        .san
        .connect_api_key(
            &token,
            "keyed",
            Secret::new("key-secret-1"),
            ConnectOptions::default(),
        )
        .await;
    h.san.flush().await;

    let logs = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    // The test is real only when San Mateo logs on these paths.
    assert!(logs.contains("token refresh failed"), "{logs}");
    assert!(logs.contains("webhook delivery failed"), "{logs}");
    for secret in [
        "at-secret-1",
        "rt-secret-1",
        "at-reply-secret",
        "crm-secret",
        "key-secret-1",
        "body-secret",
        SECRET,
        WHSEC,
        "hook-secret-1",
        &token,
    ] {
        assert!(!logs.contains(secret), "{secret} is in the logs:\n{logs}");
    }
}
