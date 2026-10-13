//! Regression tests for the review findings on connect flows (AC4, AC5,
//! AC6, AC12, AC16, AC23).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use san_mateo::clock::{ManualClock, SeqEntropy};
use san_mateo::transport::HttpResponse;
use san_mateo::{
    Catalog, ConnectOptions, ConnectSessionInput, EnvironmentConfig, Error, IntegrationDefaults,
    MemoryStore, MemoryTransport, NewIntegration, ProxySettings, SanMateo, Secret,
};
use serde_json::{Map, Value, json};
use tower::ServiceExt as _;

const EXTRA: &str = r"
selfhost:
    display_name: Self Host
    auth_mode: OAUTH2
    authorization_url: https://${connectionConfig.domain}/oauth/authorize
    token_url: https://${connectionConfig.domain}/oauth/token
    connection_config:
        domain:
            type: string
            title: Domain
    proxy:
        base_url: https://${connectionConfig.domain}/api
pinned:
    display_name: Pinned Host
    auth_mode: OAUTH2
    authorization_url: https://${connectionConfig.hostname}/authorize
    token_url: https://${connectionConfig.hostname}/token
    connection_config:
        hostname:
            type: string
            title: Host
            pattern: '^[a-z0-9-]+\.pinned\.example\.com$'
instance:
    display_name: Instance
    auth_mode: OAUTH2
    authorization_url: https://login.instance.example.com/authorize
    token_url: https://login.instance.example.com/token
    token_response_metadata: [instance_url]
    proxy:
        base_url: ${connectionConfig.instance_url}
prefix:
    display_name: Prefix Host
    auth_mode: OAUTH2
    authorization_url: https://login.prefix.example.com/authorize
    token_url: https://${connectionConfig.domain_prefix}.prefix.example.com/token
    redirect_uri_metadata: [domain_prefix]
    connection_config:
        domain_prefix:
            type: string
            title: Prefix
            automated: true
            pattern: '^[a-z0-9-]+$'
openhost:
    display_name: Open Host
    auth_mode: OAUTH2
    authorization_url: https://login.openhost.example.com/authorize
    token_url: https://${connectionConfig.host}/token
    redirect_uri_metadata: [host]
    connection_config:
        host:
            type: string
            title: Host
            automated: true
ms:
    display_name: Micro
    auth_mode: OAUTH2
    authorization_url: https://login.ms.example.com/authorize
    token_url: https://login.ms.example.com/token
    default_scopes: [offline_access]
slack:
    display_name: Chat
    auth_mode: OAUTH2
    authorization_url: https://chat.example.com/oauth/v2/authorize
    token_url: https://chat.example.com/api/oauth.v2.access
    scope_separator: ','
    disable_pkce: true
algo:
    display_name: Algo
    auth_mode: API_KEY
    proxy:
        base_url: https://api.algo.example.com
        headers:
            x-key: ${apiKey}
        verification:
            method: POST
            endpoints: ['/1/keys/${apiKey}']
            data:
                query: me
notoken:
    display_name: No Token
    auth_mode: OAUTH2
    authorization_url: https://login.notoken.example.com/authorize
";

struct H {
    san: SanMateo,
    http: MemoryTransport,
    clock: Arc<ManualClock>,
}

fn build(allowed_hosts: Vec<String>) -> H {
    let http = MemoryTransport::new();
    http.reply("POST", HOOK, HttpResponse::new(200, ""));
    let clock = Arc::new(ManualClock::new(start()));
    let mut dev = EnvironmentConfig::new("dev", SECRET);
    dev.webhooks = webhooks();
    let san = SanMateo::builder()
        .server_url(SERVER)
        .environment(dev)
        .catalog(Catalog::from_yaml_str(&format!("{CATALOG}{EXTRA}")).unwrap())
        .store(Arc::new(MemoryStore::new()))
        .key_ring(keys())
        .transport(Arc::new(http.clone()))
        .clock(clock.clone())
        .entropy(Arc::new(SeqEntropy::default()))
        .proxy_settings(ProxySettings {
            initial_backoff_ms: 0,
            allowed_hosts,
            ..ProxySettings::default()
        })
        .build()
        .unwrap();
    H { san, http, clock }
}

async fn ready_h(allowed_hosts: Vec<String>) -> H {
    let h = build(allowed_hosts);
    for (key, scopes) in [
        ("crm", vec!["read", "write"]),
        ("selfhost", vec![]),
        ("pinned", vec![]),
        ("instance", vec![]),
        ("prefix", vec![]),
        ("openhost", vec![]),
        ("ms", vec!["User.Read"]),
        ("slack", vec!["chat:write"]),
        ("algo", vec![]),
        ("notoken", vec![]),
    ] {
        h.san
            .create_integration(
                "dev",
                NewIntegration {
                    unique_key: key.to_owned(),
                    provider: key.to_owned(),
                    client_id: Some(format!("{key}-client")),
                    client_secret: Some(Secret::new(format!("{key}-secret"))),
                    scopes: scopes.into_iter().map(str::to_owned).collect(),
                    ..NewIntegration::default()
                },
            )
            .await
            .unwrap();
    }
    h
}

async fn token(h: &H, input: ConnectSessionInput) -> String {
    h.san
        .create_connect_session("dev", input)
        .await
        .unwrap()
        .token
        .expose()
        .to_owned()
}

fn cfg(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
}

fn opts(pairs: &[(&str, &str)]) -> ConnectOptions {
    ConnectOptions {
        connection_config: cfg(pairs),
        ..ConnectOptions::default()
    }
}

fn callback(url: &str, extra: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut cb = BTreeMap::from([
        ("code".to_owned(), "code-1".to_owned()),
        ("state".to_owned(), query_of(url)["state"].clone()),
    ]);
    for (k, v) in extra {
        cb.insert((*k).to_owned(), (*v).to_owned());
    }
    cb
}

fn defaults(key: &str, d: IntegrationDefaults) -> ConnectSessionInput {
    ConnectSessionInput {
        integrations_config_defaults: BTreeMap::from([(key.to_owned(), d)]),
        ..ConnectSessionInput::default()
    }
}

#[tokio::test]
async fn an_end_user_cannot_pick_the_host_of_a_provider_url() {
    let h = ready_h(vec![]).await;
    let t = token(&h, ConnectSessionInput::default()).await;
    let err = h
        .san
        .oauth_start(&t, "selfhost", opts(&[("domain", "attacker.example")]))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    // An undeclared key that sets the proxy host is also refused.
    let err = h
        .san
        .oauth_start(
            &t,
            "instance",
            opts(&[("instance_url", "https://attacker.example")]),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    // A pattern that pins the domain makes the value safe.
    h.san
        .oauth_start(
            &t,
            "pinned",
            opts(&[("hostname", "acme.pinned.example.com")]),
        )
        .await
        .unwrap();
    // A value set by the developer in the session defaults is trusted.
    let dev = token(
        &h,
        defaults(
            "selfhost",
            IntegrationDefaults {
                connection_config: cfg(&[("domain", "git.corp.example.com")]),
                ..IntegrationDefaults::default()
            },
        ),
    )
    .await;
    let url = h
        .san
        .oauth_start(&dev, "selfhost", ConnectOptions::default())
        .await
        .unwrap();
    assert!(
        url.starts_with("https://git.corp.example.com/oauth/authorize?"),
        "{url}"
    );
}

#[tokio::test]
async fn an_allowed_host_can_come_from_the_end_user() {
    let h = ready_h(vec!["git.corp.example.com".to_owned()]).await;
    let t = token(&h, ConnectSessionInput::default()).await;
    h.san
        .oauth_start(&t, "selfhost", opts(&[("domain", "git.corp.example.com")]))
        .await
        .unwrap();
    assert!(
        h.san
            .oauth_start(&t, "selfhost", opts(&[("domain", "attacker.example")]))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn callback_metadata_goes_into_the_token_url_after_a_check() {
    let h = ready_h(vec![]).await;
    h.http.reply(
        "POST",
        "https://acme.prefix.example.com/token",
        token_reply("at", None, None),
    );
    let t = token(&h, ConnectSessionInput::default()).await;
    let url = h
        .san
        .oauth_start(&t, "prefix", ConnectOptions::default())
        .await
        .unwrap();
    let conn = h
        .san
        .oauth_callback(&callback(&url, &[("domain_prefix", "acme")]))
        .await
        .unwrap();
    assert_eq!(conn.connection_config["domain_prefix"], "acme");

    // A value that breaks the pattern is refused.
    let url = h
        .san
        .oauth_start(&t, "prefix", ConnectOptions::default())
        .await
        .unwrap();
    let err = h
        .san
        .oauth_callback(&callback(&url, &[("domain_prefix", "acme_x")]))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");

    // A callback value that sets the whole token host is refused.
    let url = h
        .san
        .oauth_start(&t, "openhost", ConnectOptions::default())
        .await
        .unwrap();
    let err = h
        .san
        .oauth_callback(&callback(&url, &[("host", "attacker.example")]))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert!(
        h.http
            .requests_to("https://attacker.example/token")
            .is_empty()
    );
}

#[tokio::test]
async fn the_oauth_state_ends_with_the_connect_session() {
    let h = ready_h(vec![]).await;
    h.http.reply(
        "POST",
        "https://auth.crm.example.com/token",
        token_reply("at", None, None),
    );
    let t = token(&h, ConnectSessionInput::default()).await;
    // The session lives 30 minutes. Start the flow at minute 25.
    h.clock.advance(chrono::Duration::minutes(25));
    let url = h
        .san
        .oauth_start(&t, "crm", ConnectOptions::default())
        .await
        .unwrap();
    h.clock.advance(chrono::Duration::minutes(6));
    assert!(h.san.oauth_callback(&callback(&url, &[])).await.is_err());

    // A deleted session also ends its OAuth flows.
    let t = token(&h, ConnectSessionInput::default()).await;
    let url = h
        .san
        .oauth_start(&t, "crm", ConnectOptions::default())
        .await
        .unwrap();
    h.san.delete_connect_session(&t).await.unwrap();
    let err = h
        .san
        .oauth_callback(&callback(&url, &[]))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidConnectSession), "{err:?}");
}

#[tokio::test]
async fn end_users_cannot_set_reserved_oauth_parameters() {
    let h = ready_h(vec![]).await;
    let t = token(&h, ConnectSessionInput::default()).await;
    for key in [
        "client_id",
        "redirect_uri",
        "state",
        "scope",
        "response_type",
        "code_challenge",
        "code_challenge_method",
    ] {
        let o = ConnectOptions {
            authorization_params: BTreeMap::from([(key.to_owned(), "x".to_owned())]),
            ..ConnectOptions::default()
        };
        let err = h.san.oauth_start(&t, "crm", o).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{key}: {err:?}");
    }
    let o = ConnectOptions {
        authorization_params: BTreeMap::from([("login_hint".to_owned(), "a@b.c".to_owned())]),
        ..ConnectOptions::default()
    };
    let url = h.san.oauth_start(&t, "crm", o).await.unwrap();
    assert_eq!(query_of(&url)["login_hint"], "a@b.c");
}

#[tokio::test]
async fn webhook_overrides_cannot_point_to_internal_hosts() {
    let h = ready_h(vec![]).await;
    for bad in [
        "https://10.0.0.5/admin",
        "https://169.254.169.254/latest",
        "https://localhost/x",
        "https://[::1]/x",
    ] {
        let r = h
            .san
            .create_connect_session(
                "dev",
                ConnectSessionInput {
                    webhook_url_override: Some(bad.to_owned()),
                    ..ConnectSessionInput::default()
                },
            )
            .await;
        assert!(r.is_err(), "{bad}");
    }
}

#[tokio::test]
async fn scopes_follow_nango_rules() {
    let h = ready_h(vec![]).await;
    // Default scopes apply only when the integration has no scopes.
    let t = token(&h, ConnectSessionInput::default()).await;
    let url = h
        .san
        .oauth_start(&t, "ms", ConnectOptions::default())
        .await
        .unwrap();
    assert_eq!(query_of(&url)["scope"], "User.Read");

    // Slack user scopes go in `user_scope`. `scope` stays the integration scopes.
    let t = token(
        &h,
        defaults(
            "slack",
            IntegrationDefaults {
                user_scopes: Some("search:read".to_owned()),
                ..IntegrationDefaults::default()
            },
        ),
    )
    .await;
    let url = h
        .san
        .oauth_start(&t, "slack", ConnectOptions::default())
        .await
        .unwrap();
    let q = query_of(&url);
    assert_eq!(q["scope"], "chat:write");
    assert_eq!(q["user_scope"], "search:read");

    // The end user cannot change the scopes.
    let t = token(&h, ConnectSessionInput::default()).await;
    let o = ConnectOptions {
        user_scopes: Some("admin".to_owned()),
        ..ConnectOptions::default()
    };
    let q = query_of(&h.san.oauth_start(&t, "slack", o).await.unwrap());
    assert_eq!(q["scope"], "chat:write");
    assert!(!q.contains_key("user_scope"));
}

#[tokio::test]
async fn verification_endpoints_are_templates_and_send_data() {
    let h = ready_h(vec![]).await;
    h.http.reply(
        "POST",
        "https://api.algo.example.com/1/keys/k-123",
        HttpResponse::json(200, &json!({})),
    );
    let t = token(&h, ConnectSessionInput::default()).await;
    h.san
        .connect_api_key(&t, "algo", Secret::new("k-123"), ConnectOptions::default())
        .await
        .unwrap();
    let req = &h
        .http
        .requests_to("https://api.algo.example.com/1/keys/k-123")[0];
    assert_eq!(
        serde_json::from_slice::<Value>(&req.body).unwrap(),
        json!({"query": "me"})
    );

    h.http.reply(
        "POST",
        "https://api.algo.example.com/1/keys/bad",
        HttpResponse::new(403, ""),
    );
    let err = h
        .san
        .connect_api_key(&t, "algo", Secret::new("bad"), ConnectOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "connection_test_failed");
    assert_eq!(err.status(), 400);
}

#[tokio::test]
async fn the_error_page_hides_internal_details() {
    let h = ready_h(vec![]).await;
    let t = token(&h, ConnectSessionInput::default()).await;
    let url = h
        .san
        .oauth_start(&t, "notoken", ConnectOptions::default())
        .await
        .unwrap();
    let state = query_of(&url)["state"].clone();
    let res = san_mateo::api::router(h.san.clone())
        .oneshot(
            Request::get(format!("/oauth/callback?code=c&state={state}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(!html.contains("token URL"), "{html}");
    assert!(html.contains("an internal error occurred"), "{html}");
}

#[tokio::test]
async fn the_connect_page_never_puts_secrets_in_a_url() {
    let h = ready_h(vec![]).await;
    let t = token(&h, ConnectSessionInput::default()).await;
    let res = san_mateo::api::router(h.san.clone())
        .oneshot(
            Request::get(format!("/connect?session_token={t}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    // Forms with secrets post, so a browser with no script never puts a
    // secret in a URL. OAuth forms have no secrets. The script opens them as
    // a link, so the CSP `form-action` does not block the provider redirect.
    let forms: Vec<&str> = html.split("<form").skip(1).collect();
    assert!(forms.iter().any(|f| f.starts_with(" class=\"sm-json\"")));
    assert!(forms.iter().any(|f| f.starts_with(" class=\"sm-oauth\"")));
    for form in forms {
        if form.starts_with(" class=\"sm-json\"") {
            assert!(form.contains("method=\"post\""), "{form}");
        } else {
            assert!(form.starts_with(" class=\"sm-oauth\""), "{form}");
        }
    }
    assert!(
        html.contains("form.sm-oauth"),
        "the script handles OAuth forms"
    );
}
