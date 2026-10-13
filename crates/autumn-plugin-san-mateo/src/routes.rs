//! The mounted router and the route declarations.
//!
//! autumn mounts routes before the service exists. So each declared path
//! forwards to a cell that the startup sets.

use autumn_web::reexports::axum::Router;
use autumn_web::reexports::axum::body::Body;
use autumn_web::reexports::axum::response::Response;
use autumn_web::reexports::http::Request;
use autumn_web::route_listing::RouteInfo;

use crate::runtime::Cell;

/// The largest HTML page that the nonce bridge reads.
pub(crate) const MAX_PAGE_BYTES: usize = 1024 * 1024;

/// The route label for routes that need the API secret key.
pub(crate) const SECRET_KEY_LABEL: &str = "san_mateo:secret_key";

/// The core routes that the plugin mounts. At the root, autumn owns
/// `/health`, so the plugin leaves out the core `/health`.
pub(crate) fn mounted_specs(root: bool) -> Vec<&'static san_mateo::api::RouteSpec> {
    let _ = root;
    todo!()
}

/// The router to nest under the prefix. Each path forwards to the core API.
pub(crate) fn mounted_router<S>(cell: &Cell, root: bool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let _ = (cell, root);
    todo!()
}

/// The routes for `autumn routes` and the routes audit.
pub(crate) fn route_infos(prefix: &str) -> Vec<RouteInfo> {
    let _ = prefix;
    todo!()
}

/// Sends one request to the core API.
async fn forward(cell: &Cell, req: Request<Body>) -> Response {
    let _ = (cell, req);
    todo!()
}

/// A 503 with the San Mateo error shape.
pub(crate) fn unavailable(message: &str) -> Response {
    let _ = message;
    todo!()
}

/// The nonce in a core `Content-Security-Policy` value.
fn core_nonce(csp: &str) -> Option<&str> {
    let _ = csp;
    todo!()
}

/// Puts the autumn CSP nonce in a core HTML page.
///
/// autumn replaces the `Content-Security-Policy` header of each response.
/// With `[security.headers.csp_nonce]`, the autumn policy allows scripts
/// with the autumn nonce only. So the page must use that nonce.
async fn bridge_nonce(res: Response, nonce: &str) -> Response {
    let _ = (res, nonce);
    todo!()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use autumn_web::reexports::axum::body::to_bytes;
    use autumn_web::reexports::http::{HeaderValue, StatusCode, header};
    use autumn_web::route_listing::RouteClassification;
    use tower::ServiceExt as _;

    use super::*;
    use crate::runtime::Runtime;
    use crate::test_support::{SECRET, service};

    async fn send(router: Router, method: &str, uri: &str, auth: bool) -> (StatusCode, String) {
        let mut req = Request::builder().method(method).uri(uri);
        if auth {
            req = req.header("authorization", format!("Bearer {SECRET}"));
        }
        let res = router
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn started(cell: &Cell) -> Arc<Runtime> {
        let rt = Arc::new(Runtime::new(service(), None, Duration::from_secs(1)));
        let _ = cell.set(Arc::clone(&rt));
        rt
    }

    #[test]
    fn specs_and_infos_cover_the_core_routes() {
        let all = san_mateo::api::ROUTES;
        assert_eq!(mounted_specs(false).len(), all.len());
        let root = mounted_specs(true);
        assert_eq!(root.len(), all.len() - 1);
        assert!(root.iter().all(|r| r.path != "/health"));
        let infos = route_infos("/sm");
        assert_eq!(infos.len(), all.len());
        for (info, spec) in infos.iter().zip(all) {
            assert_eq!(info.method, spec.method);
            assert_eq!(info.path, format!("/sm{}", spec.path));
            assert_eq!(info.module.as_deref(), Some("san_mateo::api"));
            if spec.secret_key {
                assert_eq!(info.classification, RouteClassification::Gated, "{}", info.path);
                assert_eq!(info.middleware, [SECRET_KEY_LABEL]);
            } else {
                assert_eq!(info.classification, RouteClassification::Public, "{}", info.path);
            }
        }
        let root_infos = route_infos("");
        assert_eq!(root_infos.len(), all.len() - 1);
        assert!(root_infos.iter().any(|i| i.path == "/proxy/{*path}"));
    }

    #[tokio::test]
    async fn routes_reply_503_before_start_and_after_shutdown() {
        let cell: Cell = Arc::new(OnceLock::new());
        let app: Router = mounted_router(&cell, false);
        let (status, body) = send(app.clone(), "GET", "/health", false).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["error"]["code"], "service_unavailable");
        assert!(v["error"]["message"].as_str().unwrap().contains("not started"));
        let rt = started(&cell);
        let (status, body) = send(app.clone(), "GET", "/health", false).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        rt.shutdown().await;
        let (status, body) = send(app, "GET", "/providers", true).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.contains("stopping"), "{body}");
    }

    #[tokio::test]
    async fn path_parameters_reach_the_core_once() {
        // The outer match also adds path parameters. The core must not see them.
        let cell: Cell = Arc::new(OnceLock::new());
        started(&cell);
        let app: Router = Router::new().nest("/sm", mounted_router(&cell, false));
        let (status, body) = send(
            app.clone(),
            "GET",
            "/sm/connections/c-1?provider_config_key=none",
            true,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert!(body.contains("unknown"), "{body}");
        let (status, body) = send(app.clone(), "GET", "/sm/providers/nope", true).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert!(body.contains("unknown_provider"), "{body}");
        // A wrong method reaches the core router, which replies 405.
        let (status, _) = send(app.clone(), "PUT", "/sm/providers", true).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        // A path that the core does not have is not mounted.
        let (status, _) = send(app, "GET", "/sm/nope", true).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_root_mount_leaves_out_health() {
        let cell: Cell = Arc::new(OnceLock::new());
        started(&cell);
        let app: Router = mounted_router(&cell, true);
        let (status, _) = send(app.clone(), "GET", "/health", false).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = send(app, "GET", "/providers", true).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[test]
    fn finds_the_core_nonce() {
        assert_eq!(
            core_nonce("default-src 'none'; script-src 'nonce-abc_1-2'; style-src 'nonce-abc_1-2'"),
            Some("abc_1-2")
        );
        assert_eq!(core_nonce("default-src 'self'"), None);
        assert_eq!(core_nonce("script-src 'nonce-"), None);
        assert_eq!(core_nonce("script-src 'nonce-'"), None);
    }

    fn page(nonce: &str) -> Response {
        let mut res = Response::new(Body::from(format!(
            "<style nonce=\"{nonce}\">a{{}}</style><script nonce=\"{nonce}\">go()</script>"
        )));
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        res.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_str(&format!("script-src 'nonce-{nonce}'")).unwrap(),
        );
        res
    }

    async fn text(res: Response) -> String {
        String::from_utf8(to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn the_bridge_puts_the_autumn_nonce_in_html_pages() {
        let out = bridge_nonce(page("core1"), "autumnN1").await;
        assert!(out.headers().get(header::CONTENT_LENGTH).is_none());
        let csp = out.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().to_owned();
        assert!(csp.contains("'nonce-autumnN1'"), "{csp}");
        let body = text(out).await;
        assert_eq!(body.matches("nonce=\"autumnN1\"").count(), 2, "{body}");
        assert!(!body.contains("core1"), "{body}");
        // A nonce with other characters is not used.
        let body = text(bridge_nonce(page("core2"), "bad\"nonce").await).await;
        assert!(body.contains("nonce=\"core2\""), "{body}");
        // JSON is not changed.
        let mut json = Response::new(Body::from("{\"nonce\":\"x\"}"));
        json.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        assert_eq!(text(bridge_nonce(json, "n").await).await, "{\"nonce\":\"x\"}");
        // HTML with no core nonce is not changed.
        let mut plain = Response::new(Body::from("<p>hi</p>"));
        plain.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html"),
        );
        assert_eq!(text(bridge_nonce(plain, "n").await).await, "<p>hi</p>");
    }

    #[tokio::test]
    async fn a_too_large_page_gives_502() {
        let mut big = page("core3");
        *big.body_mut() = Body::from("x".repeat(MAX_PAGE_BYTES + 1));
        let out = bridge_nonce(big, "n").await;
        assert_eq!(out.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn unavailable_is_json() {
        let res = unavailable("wait");
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            res.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("application/json")
        );
        let v: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
        assert_eq!(v["error"]["message"], "wait");
    }

    #[tokio::test]
    async fn the_connect_page_uses_the_autumn_nonce() {
        use autumn_web::security::{HeadersConfig, SecurityHeadersLayer};
        let cell: Cell = Arc::new(OnceLock::new());
        let rt = started(&cell);
        let token = rt
            .san
            .create_connect_session("dev", san_mateo::ConnectSessionInput::default())
            .await
            .unwrap()
            .token
            .expose()
            .to_owned();
        let uri = format!("/connect?session_token={token}");
        let get = || Request::get(&uri).body(Body::empty()).unwrap();
        // With the autumn nonce on, the page uses the nonce of the autumn policy.
        let mut headers = HeadersConfig::default();
        headers.csp_nonce.enabled = true;
        let app: Router =
            mounted_router(&cell, false).layer(SecurityHeadersLayer::from_config(&headers));
        let res = app.oneshot(get()).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let csp = res.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_owned();
        let nonce = core_nonce(&csp).unwrap().to_owned();
        let body = text(res).await;
        assert!(body.contains(&format!("<script nonce=\"{nonce}\">")), "{csp}\n{body}");
        // With no autumn nonce, the core page is not changed.
        let res = forward(&cell, get()).await;
        let csp = res.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_owned();
        let core = core_nonce(&csp).unwrap().to_owned();
        assert!(text(res).await.contains(&format!("nonce=\"{core}\"")));
    }
}
