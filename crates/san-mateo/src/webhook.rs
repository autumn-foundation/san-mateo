//! Outbound webhooks to the product backend.
//!
//! The body is JSON with sorted keys. The headers `X-San-Mateo-Hmac-Sha256`
//! and `X-Nango-Hmac-Sha256` hold the hex HMAC-SHA256 of the body bytes.
//! Delivery is at least once. A 3xx or 4xx reply is not retried.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::model::{EndUser, Tags};
use crate::secret::Secret;
use crate::store::SyncCounts;
use crate::transport::HttpTransport;

/// The San Mateo signature header.
pub const SIGNATURE_HEADER: &str = "x-san-mateo-hmac-sha256";
/// The Nango-compatible signature header.
pub const NANGO_SIGNATURE_HEADER: &str = "x-nango-hmac-sha256";

/// Webhook settings for one environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookSettings {
    /// The primary URL.
    pub primary_url: Option<String>,
    /// A second URL.
    pub secondary_url: Option<String>,
    /// The signing secret. With no secret, there is no signature header.
    pub signing_secret: Option<Secret>,
    /// Retries after the first attempt.
    pub retries: u32,
    /// First backoff.
    pub initial_backoff_ms: u64,
    /// Send `auth` events.
    pub send_auth: bool,
    /// Send `sync` events.
    pub send_sync: bool,
}

impl Default for WebhookSettings {
    fn default() -> Self {
        Self {
            primary_url: None,
            secondary_url: None,
            signing_secret: None,
            retries: 2,
            initial_backoff_ms: 3_000,
            send_auth: true,
            send_sync: true,
        }
    }
}

/// The operation of an `auth` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthOperation {
    /// A new connection.
    Creation,
    /// New credentials for a connection that exists.
    Override,
    /// A token refresh.
    Refresh,
    /// A delete.
    Deletion,
}

/// An error in an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorPayload {
    /// The error code.
    #[serde(rename = "type")]
    pub kind: String,
    /// A short description.
    pub description: String,
}

/// An `auth` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEvent {
    /// The connection id.
    pub connection_id: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The provider name.
    pub provider: String,
    /// The auth mode name.
    pub auth_mode: String,
    /// The environment id.
    pub environment: String,
    /// The operation.
    pub operation: AuthOperation,
    /// `true` when the operation succeeded.
    pub success: bool,
    /// The error, when it failed.
    pub error: Option<ErrorPayload>,
    /// Connection tags.
    pub tags: Tags,
    /// The end user.
    pub end_user: Option<EndUser>,
}

/// A `sync` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncEvent {
    /// The connection id.
    pub connection_id: String,
    /// The integration key.
    pub provider_config_key: String,
    /// The sync name.
    pub sync_name: String,
    /// The models of the sync.
    pub models: Vec<String>,
    /// `true` when the run succeeded.
    pub success: bool,
    /// Read records changed after this time to get the changes of the run.
    pub modified_after: DateTime<Utc>,
    /// Counts of the run.
    pub counts: SyncCounts,
    /// The error, when it failed.
    pub error: Option<ErrorPayload>,
}

/// An event for the product backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEvent {
    /// A connection event.
    Auth(AuthEvent),
    /// A sync event.
    Sync(SyncEvent),
}

impl WebhookEvent {
    /// Returns the event as JSON, with Nango-compatible camelCase keys.
    #[must_use]
    pub fn to_json(&self) -> Value {
        todo!()
    }

    /// Returns the body bytes: JSON with sorted keys.
    #[must_use]
    pub fn body(&self) -> Vec<u8> {
        todo!()
    }
}

/// Returns the hex HMAC-SHA256 of `body` with `secret`.
#[must_use]
pub fn sign(secret: &str, body: &[u8]) -> String {
    let _ = (secret, body);
    todo!()
}

/// Checks a signature. The compare takes constant time. An empty secret
/// never verifies.
#[must_use]
pub fn verify(secret: &str, body: &[u8], signature_hex: &str) -> bool {
    let _ = (secret, body, signature_hex);
    todo!()
}

/// The result of one delivery to one URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The URL.
    pub url: String,
    /// The last status, if any.
    pub status: Option<u16>,
    /// The number of attempts.
    pub attempts: u32,
    /// `true` for a 2xx reply.
    pub ok: bool,
}

/// Sends events.
#[derive(Clone)]
pub struct WebhookSender {
    transport: Arc<dyn HttpTransport>,
}

impl std::fmt::Debug for WebhookSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebhookSender")
    }
}

impl WebhookSender {
    /// Makes a sender.
    #[must_use]
    pub fn new(transport: Arc<dyn HttpTransport>) -> Self {
        Self { transport }
    }

    /// Sends `event` to each URL. `url_override` replaces both environment
    /// URLs. Returns one result for each URL.
    pub async fn deliver(
        &self,
        settings: &WebhookSettings,
        url_override: Option<&str>,
        event: &WebhookEvent,
    ) -> Vec<Delivery> {
        let _ = (settings, url_override, event, &self.transport);
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{HttpResponse, MemoryTransport, TransportError};
    use serde_json::json;

    fn auth() -> WebhookEvent {
        WebhookEvent::Auth(AuthEvent {
            connection_id: "c1".into(),
            provider_config_key: "gh".into(),
            provider: "github".into(),
            auth_mode: "OAUTH2".into(),
            environment: "dev".into(),
            operation: AuthOperation::Creation,
            success: true,
            error: None,
            tags: Tags::from([("team".to_owned(), "a".to_owned())]),
            end_user: Some(EndUser {
                id: "u1".into(),
                email: Some("u@example.com".into()),
                organization: Some(crate::model::Organization { id: "o1".into(), display_name: None }),
                ..EndUser::default()
            }),
        })
    }

    #[test]
    fn auth_json_shape() {
        assert_eq!(
            auth().to_json(),
            json!({
                "from": "san-mateo",
                "type": "auth",
                "connectionId": "c1",
                "providerConfigKey": "gh",
                "provider": "github",
                "authMode": "OAUTH2",
                "environment": "dev",
                "operation": "creation",
                "success": true,
                "tags": {"team": "a"},
                "endUser": {"endUserId": "u1", "endUserEmail": "u@example.com", "organizationId": "o1", "tags": {}}
            })
        );
        let body = String::from_utf8(auth().body()).unwrap();
        assert!(body.starts_with(r#"{"authMode":"OAUTH2","connectionId":"c1","#), "{body}");
    }

    #[test]
    fn sync_and_error_json_shape() {
        let e = WebhookEvent::Sync(SyncEvent {
            connection_id: "c1".into(),
            provider_config_key: "gh".into(),
            sync_name: "issues".into(),
            models: vec!["Issue".into()],
            success: false,
            modified_after: chrono::TimeZone::timestamp_opt(&Utc, 1_700_000_000, 0).unwrap(),
            counts: SyncCounts { added: 1, updated: 2, deleted: 3 },
            error: Some(ErrorPayload { kind: "function_failed".into(), description: "boom".into() }),
        });
        assert_eq!(
            e.to_json(),
            json!({
                "from": "san-mateo",
                "type": "sync",
                "connectionId": "c1",
                "providerConfigKey": "gh",
                "syncName": "issues",
                "model": "Issue",
                "models": ["Issue"],
                "syncType": "INCREMENTAL",
                "success": false,
                "modifiedAfter": "2023-11-14T22:13:20Z",
                "responseResults": {"added": 1, "updated": 2, "deleted": 3},
                "error": {"type": "function_failed", "description": "boom"}
            })
        );
    }

    #[test]
    fn signatures() {
        // HMAC-SHA256 vector from RFC 4231 test case 2.
        assert_eq!(
            sign("Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let sig = sign("s3cret", b"body");
        assert!(verify("s3cret", b"body", &sig));
        assert!(!verify("s3cret", b"body2", &sig));
        assert!(!verify("other", b"body", &sig));
        assert!(!verify("", b"body", &sign("", b"body")));
        assert!(!verify("s3cret", b"body", "zz"));
    }

    fn settings() -> WebhookSettings {
        WebhookSettings {
            primary_url: Some("https://hooks.example.com/a".into()),
            secondary_url: Some("https://hooks.example.com/b".into()),
            signing_secret: Some("whsec".into()),
            retries: 2,
            initial_backoff_ms: 0,
            ..WebhookSettings::default()
        }
    }

    #[tokio::test]
    async fn delivers_signed_to_both_urls() {
        let t = MemoryTransport::new();
        t.reply("POST", "https://hooks.example.com/a", HttpResponse::new(200, ""));
        t.reply("POST", "https://hooks.example.com/b", HttpResponse::new(204, ""));
        let s = WebhookSender::new(Arc::new(t.clone()));
        let d = s.deliver(&settings(), None, &auth()).await;
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.ok && x.attempts == 1));
        let reqs = t.requests();
        let body = auth().body();
        for r in &reqs {
            assert_eq!(r.body, body);
            assert_eq!(r.header_value("content-type"), Some("application/json"));
            assert_eq!(r.header_value(SIGNATURE_HEADER), Some(sign("whsec", &body).as_str()));
            assert_eq!(r.header_value(NANGO_SIGNATURE_HEADER), Some(sign("whsec", &body).as_str()));
        }
    }

    #[tokio::test]
    async fn override_replaces_both_urls() {
        let t = MemoryTransport::new();
        t.reply("POST", "https://override.example.com/x", HttpResponse::new(200, ""));
        let s = WebhookSender::new(Arc::new(t.clone()));
        let d = s.deliver(&settings(), Some("https://override.example.com/x"), &auth()).await;
        assert_eq!(d.len(), 1);
        assert_eq!(t.requests().len(), 1);
    }

    #[tokio::test]
    async fn retries_5xx_and_network_errors_but_not_4xx() {
        let t = MemoryTransport::new();
        t.reply("POST", "https://hooks.example.com/a", HttpResponse::new(503, ""));
        t.reply("POST", "https://hooks.example.com/a", HttpResponse::new(200, ""));
        t.fail("POST", "https://hooks.example.com/b", TransportError::Timeout);
        let s = WebhookSender::new(Arc::new(t.clone()));
        let d = s.deliver(&settings(), None, &auth()).await;
        assert_eq!(d[0], Delivery { url: "https://hooks.example.com/a".into(), status: Some(200), attempts: 2, ok: true });
        assert_eq!(d[1], Delivery { url: "https://hooks.example.com/b".into(), status: None, attempts: 3, ok: false });

        let t = MemoryTransport::new();
        t.reply("POST", "https://hooks.example.com/a", HttpResponse::new(410, ""));
        let s = WebhookSender::new(Arc::new(t.clone()));
        let one = WebhookSettings { secondary_url: None, ..settings() };
        let d = s.deliver(&one, None, &auth()).await;
        assert_eq!(d[0].attempts, 1);
        assert!(!d[0].ok);
    }

    #[tokio::test]
    async fn no_urls_or_turned_off_sends_nothing() {
        let t = MemoryTransport::new();
        let s = WebhookSender::new(Arc::new(t.clone()));
        assert!(s.deliver(&WebhookSettings::default(), None, &auth()).await.is_empty());
        let off = WebhookSettings { send_auth: false, ..settings() };
        assert!(s.deliver(&off, None, &auth()).await.is_empty());
        let unsigned = WebhookSettings { signing_secret: None, secondary_url: None, ..settings() };
        t.reply("POST", "https://hooks.example.com/a", HttpResponse::new(200, ""));
        s.deliver(&unsigned, None, &auth()).await;
        assert_eq!(t.requests()[0].header_value(SIGNATURE_HEADER), None);
    }
}
