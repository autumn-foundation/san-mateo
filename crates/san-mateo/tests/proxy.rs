//! Proxy, retry, SSRF and pagination (AC10 to AC13).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use common::*;
use san_mateo::model::ConnectionKey;
use san_mateo::transport::{HttpResponse, TransportError};
use san_mateo::{ConnectOptions, Error, PaginateOptions, ProxyRequest, Secret};
use serde_json::{Map, Value, json};

const API: &str = "https://api.crm.example.com";

async fn crm(h: &Harness) -> ConnectionKey {
    connect_crm(h, "at-1", Some("rt-1"), Some(3_600))
        .await
        .key()
}

#[tokio::test]
async fn oauth_proxy_adds_auth_and_provider_headers() {
    let h = ready().await;
    let key = crm(&h).await;
    h.http.reply(
        "GET",
        &format!("{API}/v3/contacts"),
        HttpResponse::json(200, &json!({"ok": true}))
            .header("transfer-encoding", "chunked")
            .header("x-request-id", "r1"),
    );
    let req = ProxyRequest::get("/v3/contacts?limit=10")
        .header("accept", "application/json")
        .query("archived", "false");
    let res = h.san.proxy("dev", &key, req).await.unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.json::<Value>().unwrap()["ok"], true);
    assert_eq!(res.header_value("x-request-id"), Some("r1"));
    assert_eq!(
        res.header_value("transfer-encoding"),
        None,
        "hop-by-hop headers stay out"
    );
    let sent = &h.http.requests_to(&format!("{API}/v3/contacts"))[0];
    assert_eq!(
        sent.url,
        format!("{API}/v3/contacts?limit=10&archived=false")
    );
    assert_eq!(sent.header_value("authorization"), Some("Bearer at-1"));
    assert_eq!(sent.header_value("x-crm-version"), Some("2024-01"));
    assert_eq!(sent.header_value("accept"), Some("application/json"));
}

#[tokio::test]
async fn api_key_and_basic_proxies() {
    let h = ready().await;
    let token = session(&h).await;
    h.http.reply(
        "GET",
        "https://api.keyed.example.com/v1/me",
        HttpResponse::new(200, "{}"),
    );
    let k = h
        .san
        .connect_api_key(
            &token,
            "keyed",
            Secret::new("key-1"),
            ConnectOptions::default(),
        )
        .await
        .unwrap();
    h.http.reply(
        "POST",
        "https://api.keyed.example.com/v1/items",
        HttpResponse::new(201, "{}"),
    );
    let res = h
        .san
        .proxy(
            "dev",
            &k.key(),
            ProxyRequest::new("post", "/v1/items").json(&json!({"a": 1})),
        )
        .await
        .unwrap();
    assert_eq!(res.status, 201);
    let sent = &h.http.requests_to("https://api.keyed.example.com/v1/items")[0];
    assert_eq!(sent.header_value("x-api-key"), Some("key-1"));
    assert_eq!(sent.header_value("authorization"), None);
    assert_eq!(sent.body, br#"{"a":1}"#);
    assert_eq!(sent.method, "POST");

    let b = h
        .san
        .connect_basic(
            &token,
            "basic",
            "ann".into(),
            Secret::new("pw"),
            ConnectOptions::default(),
        )
        .await
        .unwrap();
    h.http.reply(
        "GET",
        "https://api.basic.example.com/me",
        HttpResponse::new(200, "{}"),
    );
    h.san
        .proxy("dev", &b.key(), ProxyRequest::get("/me"))
        .await
        .unwrap();
    let sent = &h.http.requests_to("https://api.basic.example.com/me")[0];
    assert_eq!(sent.header_value("authorization"), Some("Basic YW5uOnB3"));
}

#[tokio::test]
async fn interpolated_base_url() {
    let h = ready().await;
    let token = session(&h).await;
    let opts = ConnectOptions {
        connection_config: Map::from_iter([("subdomain".to_owned(), json!("acme"))]),
        ..ConnectOptions::default()
    };
    let url = h.san.oauth_start(&token, "zen", opts).await.unwrap();
    h.http.reply(
        "POST",
        "https://acme.zen.example.com/oauth/token",
        token_reply("zat", None, None),
    );
    let cb = std::collections::BTreeMap::from([
        ("code".to_owned(), "c".to_owned()),
        ("state".to_owned(), query_of(&url)["state"].clone()),
    ]);
    let conn = h.san.oauth_callback(&cb).await.unwrap();
    h.http.reply(
        "GET",
        "https://acme.zen.example.com/api/tickets",
        HttpResponse::new(200, "[]"),
    );
    let res = h
        .san
        .proxy("dev", &conn.key(), ProxyRequest::get("/tickets"))
        .await
        .unwrap();
    assert_eq!(res.status, 200);
}

#[tokio::test]
async fn a_401_refreshes_once_and_retries() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/deals");
    h.http.reply("GET", &url, HttpResponse::new(401, "expired"));
    h.http.reply("GET", &url, HttpResponse::new(200, "[]"));
    h.http.reply(
        "POST",
        "https://auth.crm.example.com/token",
        token_reply("at-2", Some("rt-2"), Some(3_600)),
    );
    let res = h
        .san
        .proxy("dev", &key, ProxyRequest::get("/v3/deals"))
        .await
        .unwrap();
    assert_eq!(res.status, 200);
    let sent = h.http.requests_to(&url);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].header_value("authorization"), Some("Bearer at-1"));
    assert_eq!(sent[1].header_value("authorization"), Some("Bearer at-2"));

    // A second 401 goes back to the caller. Only one refresh happens.
    let url = format!("{API}/v3/forbidden");
    h.http.reply("GET", &url, HttpResponse::new(401, "no"));
    let res = h
        .san
        .proxy("dev", &key, ProxyRequest::get("/v3/forbidden"))
        .await
        .unwrap();
    assert_eq!(res.status, 401);
    assert_eq!(h.http.requests_to(&url).len(), 2);
}

#[tokio::test]
async fn rate_limits_retry_for_all_methods() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/notes");
    h.http.reply(
        "POST",
        &url,
        HttpResponse::new(429, "").header("retry-after", "0"),
    );
    h.http.reply("POST", &url, HttpResponse::new(201, "{}"));
    let res = h
        .san
        .proxy(
            "dev",
            &key,
            ProxyRequest::new("POST", "/v3/notes").retries(2),
        )
        .await
        .unwrap();
    assert_eq!(res.status, 201);
    assert_eq!(h.http.requests_to(&url).len(), 2);

    // A wait above the cap gives up and returns the 429.
    let url = format!("{API}/v3/slow");
    h.http.reply(
        "GET",
        &url,
        HttpResponse::new(429, "").header("retry-after", "3600"),
    );
    let res = h
        .san
        .proxy("dev", &key, ProxyRequest::get("/v3/slow").retries(5))
        .await
        .unwrap();
    assert_eq!(res.status, 429);
    assert_eq!(h.http.requests_to(&url).len(), 1);

    // No retries asked: one attempt.
    let url = format!("{API}/v3/once");
    h.http.reply("GET", &url, HttpResponse::new(429, ""));
    assert_eq!(
        h.san
            .proxy("dev", &key, ProxyRequest::get("/v3/once"))
            .await
            .unwrap()
            .status,
        429
    );
    assert_eq!(h.http.requests_to(&url).len(), 1);
}

#[tokio::test]
async fn server_errors_retry_only_idempotent_calls_or_opt_in() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/things");
    h.http.reply("POST", &url, HttpResponse::new(502, ""));
    let res = h
        .san
        .proxy(
            "dev",
            &key,
            ProxyRequest::new("POST", "/v3/things").retries(3),
        )
        .await
        .unwrap();
    assert_eq!(res.status, 502);
    assert_eq!(
        h.http.requests_to(&url).len(),
        1,
        "no second POST after a 5xx"
    );

    let mut opt_in = ProxyRequest::new("POST", "/v3/things").retries(2);
    opt_in.retry_on = vec![502];
    let res = h.san.proxy("dev", &key, opt_in).await.unwrap();
    assert_eq!(res.status, 502);
    assert_eq!(h.http.requests_to(&url).len(), 4, "1 + 3 attempts");

    let url = format!("{API}/v3/flaky");
    h.http.reply("GET", &url, HttpResponse::new(503, ""));
    h.http.reply("GET", &url, HttpResponse::new(200, "{}"));
    assert_eq!(
        h.san
            .proxy("dev", &key, ProxyRequest::get("/v3/flaky").retries(1))
            .await
            .unwrap()
            .status,
        200
    );
}

#[tokio::test]
async fn provider_error_codes_and_remaining_header() {
    let h = ready().await;
    let key = crm(&h).await;
    // The provider lists 403 as retryable. A 403 with `x-remaining: 0` is a rate limit.
    let url = format!("{API}/v3/limited");
    h.http.reply(
        "POST",
        &url,
        HttpResponse::new(403, "")
            .header("x-remaining", "0")
            .header("x-reset", "1700000000"),
    );
    h.http.reply("POST", &url, HttpResponse::new(200, "{}"));
    assert_eq!(
        h.san
            .proxy(
                "dev",
                &key,
                ProxyRequest::new("POST", "/v3/limited").retries(1)
            )
            .await
            .unwrap()
            .status,
        200
    );
}

#[tokio::test]
async fn network_errors_retry_then_fail() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/down");
    h.http
        .fail("GET", &url, TransportError::Connect("refused".into()));
    let err = h
        .san
        .proxy("dev", &key, ProxyRequest::get("/v3/down").retries(2))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Upstream(_)), "{err:?}");
    assert_eq!(h.http.requests_to(&url).len(), 3);
    h.http.fail(
        "GET",
        &format!("{API}/v3/big"),
        TransportError::BodyTooLarge,
    );
    assert_eq!(
        h.san
            .proxy("dev", &key, ProxyRequest::get("/v3/big"))
            .await
            .unwrap_err(),
        Error::UpstreamTooLarge
    );
}

#[tokio::test]
async fn attempts_have_a_hard_limit() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/always");
    h.http.reply("GET", &url, HttpResponse::new(429, ""));
    let res = h
        .san
        .proxy("dev", &key, ProxyRequest::get("/v3/always").retries(50))
        .await
        .unwrap();
    assert_eq!(res.status, 429);
    assert_eq!(h.http.requests_to(&url).len(), 10);
}

#[tokio::test]
async fn base_url_override_is_checked() {
    let h = ready().await;
    let key = crm(&h).await;
    let mut same = ProxyRequest::get("/v9/x");
    same.base_url_override = Some(API.to_owned());
    h.http
        .reply("GET", &format!("{API}/v9/x"), HttpResponse::new(200, ""));
    assert_eq!(h.san.proxy("dev", &key, same).await.unwrap().status, 200);
    for bad in [
        "https://evil.example.org",
        "http://api.crm.example.com",
        "https://169.254.169.254",
        "https://127.0.0.1:8080",
    ] {
        let mut r = ProxyRequest::get("/x");
        r.base_url_override = Some(bad.to_owned());
        let err = h.san.proxy("dev", &key, r).await.unwrap_err();
        assert!(matches!(err, Error::ForbiddenHost(_)), "{bad}: {err:?}");
    }
    assert!(h.http.requests_to("https://evil.example.org/x").is_empty());
    assert!(matches!(
        h.san
            .proxy(
                "dev",
                &ConnectionKey::new("crm", "nope"),
                ProxyRequest::get("/x")
            )
            .await,
        Err(Error::UnknownConnection(_))
    ));
}

#[tokio::test]
async fn cursor_pagination_reads_all_pages() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/contacts");
    h.http.reply(
        "GET",
        &url,
        HttpResponse::json(
            200,
            &json!({"results": [{"id": 1}, {"id": 2}], "next": "c2"}),
        ),
    );
    h.http.reply(
        "GET",
        &url,
        HttpResponse::json(200, &json!({"results": [{"id": 3}], "next": "c3"})),
    );
    h.http.reply(
        "GET",
        &url,
        HttpResponse::json(200, &json!({"results": [{"id": 4}]})),
    );
    let all = h
        .san
        .paginate(
            "dev",
            &key,
            ProxyRequest::get("/v3/contacts"),
            PaginateOptions {
                limit: Some(2),
                ..PaginateOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        all.iter()
            .map(|r| r["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    let sent = h.http.requests_to(&url);
    assert_eq!(sent[0].url, format!("{url}?limit=2"));
    assert_eq!(sent[1].url, format!("{url}?cursor=c2&limit=2"));
    assert_eq!(sent[2].url, format!("{url}?cursor=c3&limit=2"));
}

#[tokio::test]
async fn pagination_has_a_page_limit_and_checks_replies() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/loop");
    for i in 0..5 {
        h.http.reply(
            "GET",
            &url,
            HttpResponse::json(
                200,
                &json!({"results": [{"id": i}], "next": format!("c{i}")}),
            ),
        );
    }
    let all = h
        .san
        .paginate(
            "dev",
            &key,
            ProxyRequest::get("/v3/loop"),
            PaginateOptions {
                max_pages: Some(3),
                ..PaginateOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    let url = format!("{API}/v3/broken");
    h.http.reply("GET", &url, HttpResponse::new(500, "oops"));
    assert!(
        h.san
            .paginate(
                "dev",
                &key,
                ProxyRequest::get("/v3/broken"),
                PaginateOptions::default()
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_repeated_cursor_stops_pagination() {
    let h = ready().await;
    let key = crm(&h).await;
    let url = format!("{API}/v3/stuck");
    h.http.reply(
        "GET",
        &url,
        HttpResponse::json(200, &json!({"results": [{"id": 1}], "next": "again"})),
    );
    let all = h
        .san
        .paginate(
            "dev",
            &key,
            ProxyRequest::get("/v3/stuck"),
            PaginateOptions {
                max_pages: Some(10),
                ..PaginateOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    let sent = h.http.requests_to(&url);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].url, format!("{url}?cursor=again"));
}

#[tokio::test]
async fn optional_template_values_are_left_out() {
    let h = shop_harness().await;
    let url = "https://api.shop.example.com/v1/orders";
    h.http
        .reply("GET", url, HttpResponse::json(200, &json!([])));
    let bare = shop_connection(&h, "s1", json!({}), None).await;
    let res = h
        .san
        .proxy("dev", &bare, ProxyRequest::get("/v1/orders"))
        .await
        .unwrap();
    assert_eq!(res.status, 200);
    let sent = &h.http.requests_to(url)[0];
    assert_eq!(sent.url, url);
    assert_eq!(sent.header_value("x-login-customer-id"), None);
    assert_eq!(sent.header_value("authorization"), Some("Bearer at-1"));
    let full = shop_connection(
        &h,
        "s2",
        json!({"login_customer_id": "123", "account": "acc-9"}),
        None,
    )
    .await;
    h.san
        .proxy("dev", &full, ProxyRequest::get("/v1/orders"))
        .await
        .unwrap();
    let sent = &h.http.requests_to(url)[1];
    assert_eq!(sent.url, format!("{url}?account=acc-9"));
    assert_eq!(sent.header_value("x-login-customer-id"), Some("123"));
}

/// Nango `OAuth2` templates such as `${clientId}:${clientSecret}` read the
/// integration client.
#[tokio::test]
async fn oauth_templates_see_the_integration_client() {
    let h = shop_harness().await;
    let url = "https://api.shop.example.com/v1/orders";
    h.http
        .reply("GET", url, HttpResponse::json(200, &json!([])));
    let key = shop_connection(&h, "s1", json!({}), None).await;
    h.san
        .proxy("dev", &key, ProxyRequest::get("/v1/orders"))
        .await
        .unwrap();
    let sent = &h.http.requests_to(url)[0];
    assert_eq!(
        sent.header_value("x-api-key"),
        Some("shop-client:shop-secret")
    );
}

#[tokio::test]
async fn link_pagination_stays_on_known_hosts() {
    let h = ready().await;
    let key = crm(&h).await;
    let cfg = serde_json::from_value(json!({"type": "link", "response_path": "items"})).unwrap();
    let url = format!("{API}/v1/items");
    h.http.reply(
        "GET",
        &url,
        HttpResponse::json(200, &json!({"items": [1]}))
            .header("link", format!("<{API}/v1/items?page=2>; rel=\"next\"")),
    );
    h.http.reply(
        "GET",
        &url,
        HttpResponse::json(200, &json!({"items": [2]}))
            .header("link", "<https://evil.example.org/steal>; rel=\"next\""),
    );
    let err = h
        .san
        .paginate(
            "dev",
            &key,
            ProxyRequest::get("/v1/items"),
            PaginateOptions {
                config: Some(cfg),
                ..PaginateOptions::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::ForbiddenHost(_)), "{err:?}");
    assert_eq!(
        h.http.requests_to(&url)[1].url,
        format!("{API}/v1/items?page=2")
    );
    assert!(
        h.http
            .requests_to("https://evil.example.org/steal")
            .is_empty()
    );
}

#[tokio::test]
async fn pagination_needs_a_config() {
    let h = ready().await;
    let token = session(&h).await;
    let o = h
        .san
        .connect_unauthenticated(&token, "open", ConnectOptions::default())
        .await
        .unwrap();
    assert!(matches!(
        h.san
            .paginate(
                "dev",
                &o.key(),
                ProxyRequest::get("/x"),
                PaginateOptions::default()
            )
            .await,
        Err(Error::Invalid(_))
    ));
}
