//! `ReqwestTransport` against a local HTTP server (AC10, AC12).

use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use san_mateo::transport::{HttpRequest, HttpTransport, ReqwestTransport, TransportConfig, TransportError};

async fn serve() -> String {
    let app = Router::new()
        .route("/echo", post(|headers: HeaderMap, body: String| async move {
            let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
            ([("x-seen-auth", auth)], format!("got:{body}"))
        }))
        .route("/redirect", get(|| async { (StatusCode::FOUND, [("location", "http://127.0.0.1:1/stolen")]).into_response() }))
        .route("/big", get(|| async { "x".repeat(10_000) }))
        .route("/slow", get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "late"
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn transport(max_body_bytes: usize) -> ReqwestTransport {
    ReqwestTransport::new(TransportConfig { timeout: Duration::from_millis(800), connect_timeout: Duration::from_millis(500), max_body_bytes }).unwrap()
}

#[tokio::test]
async fn sends_method_headers_and_body() {
    let base = serve().await;
    let t = transport(1024);
    let res = t
        .send(HttpRequest::new("POST", format!("{base}/echo")).header("Authorization", "Bearer tok").body(b"hello".to_vec()))
        .await
        .unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.body, b"got:hello");
    assert_eq!(res.header_value("x-seen-auth"), Some("Bearer tok"));
}

#[tokio::test]
async fn never_follows_redirects() {
    let base = serve().await;
    let res = transport(1024).send(HttpRequest::new("GET", format!("{base}/redirect"))).await.unwrap();
    assert_eq!(res.status, 302);
    assert_eq!(res.header_value("location"), Some("http://127.0.0.1:1/stolen"));
}

#[tokio::test]
async fn caps_the_body_and_times_out() {
    let base = serve().await;
    assert_eq!(transport(1_000).send(HttpRequest::new("GET", format!("{base}/big"))).await, Err(TransportError::BodyTooLarge));
    assert_eq!(transport(20_000).send(HttpRequest::new("GET", format!("{base}/big"))).await.unwrap().body.len(), 10_000);
    assert_eq!(transport(1_000).send(HttpRequest::new("GET", format!("{base}/slow"))).await, Err(TransportError::Timeout));
}
