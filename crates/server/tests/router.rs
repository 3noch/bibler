mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use server::{https_redirect, router, Config};
use tower::ServiceExt;

async fn get(app: &axum::Router, uri: &str, gzip: bool) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut req = Request::get(uri);
    if gzip {
        req = req.header(header::ACCEPT_ENCODING, "gzip");
    }
    let res = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status, parts.headers, String::from_utf8_lossy(&bytes).into_owned())
}

fn app() -> axum::Router {
    let (dist, text, db) = common::fixture("router", 100);
    router(&Config { static_dir: Some(dist), text_dir: text, db, secure_cookies: false }).unwrap()
}

#[tokio::test]
async fn health() {
    let (status, headers, body) = get(&app(), "/api/health", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"ok\":true"));
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn unknown_api_route_is_404_not_the_spa() {
    let (status, _, body) = get(&app(), "/api/nope", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("doctype"));
}

#[tokio::test]
async fn text_artifacts_cache_by_hash() {
    let app = app();
    let (status, headers, _) = get(&app, "/text/t1/02-GEN.0123456789abcdef.json", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
    let (status, headers, _) = get(&app, "/text/t1/manifest.json", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let (status, _, _) = get(&app, "/text/t1/missing.json", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "missing text files must not fall back to index.html");
}

#[tokio::test]
async fn compresses_when_asked() {
    let (status, headers, _) = get(&app(), "/text/t1/02-GEN.0123456789abcdef.json", true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
}

#[tokio::test]
async fn spa_fallback_and_assets() {
    let app = app();
    let (status, headers, body) = get(&app, "/some/client/route", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("doctype"));
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let (status, headers, _) = get(&app, "/assets/app-AbC123.js", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
}

#[tokio::test]
async fn dev_mode_without_static_dir() {
    let (_, text, db) = common::fixture("router-dev", 100);
    let app = router(&Config { static_dir: None, text_dir: text, db, secure_cookies: false }).unwrap();
    let (status, _, body) = get(&app, "/", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("make dev"));
}

#[tokio::test]
async fn http_redirects_to_https() {
    let app = https_redirect(8443);
    let req = Request::get("/john/3?x=1").header(header::HOST, "example.org:8080").body(Body::empty()).unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(res.headers()[header::LOCATION], "https://example.org:8443/john/3?x=1");
}
