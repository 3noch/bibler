//! bible-tags server: serves the client bundle, the scripture text artifacts, sessions and the sync WebSocket.

pub mod hub;
pub mod store;
pub mod sync;
pub mod texts;

use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

#[derive(Debug, Clone)]
pub struct Config {
    /// Built client (Vite `dist/`). `None` in dev, where Vite serves the client and proxies to us.
    pub static_dir: Option<PathBuf>,
    /// Directory containing `<text_id>/manifest.json` etc. (text-build output), served under `/text/`.
    pub text_dir: PathBuf,
    /// SQLite database file (created if missing).
    pub db: PathBuf,
    /// Mark the session cookie `Secure` (set when serving TLS).
    pub secure_cookies: bool,
}

/// Build the app: open the store, load the texts, wire the routes.
///
/// # Errors
/// If the database cannot be opened or the text directory cannot be read.
pub fn router(cfg: &Config) -> Result<Router, String> {
    let hub = Arc::new(hub::Hub::default());
    let listener: store::CommitListener = {
        let hub = hub.clone();
        Box::new(move |owner, change| hub.publish(owner, change))
    };
    let state = Arc::new(sync::AppState {
        store: Arc::new(store::SqliteStore::open(&cfg.db, listener)?),
        hub,
        texts: texts::Texts::load(&cfg.text_dir)?,
        secure_cookies: cfg.secure_cookies,
    });
    let api = Router::new()
        .route("/health", get(health))
        .route("/session", post(sync::session))
        .route("/sync", get(sync::sync))
        .fallback(|| async { (StatusCode::NOT_FOUND, "no such API route") })
        .with_state(state);

    let app = Router::new().nest("/api", api).nest_service("/text", ServeDir::new(&cfg.text_dir));
    let app = match &cfg.static_dir {
        // Unknown paths get index.html so client-side routes survive a reload.
        Some(dir) => app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => app.fallback(|| async {
            (StatusCode::NOT_FOUND, "no client bundle configured (in dev, open the Vite server: make dev)")
        }),
    };
    Ok(app.layer(middleware::from_fn(cache_control)).layer(CompressionLayer::new()).layer(TraceLayer::new_for_http()))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// Content-hashed files (`/assets/*` from Vite, `name.<16 hex>.ext` from text-build) are immutable; everything
/// else (index.html, manifest.json, API) must be revalidated.
async fn cache_control(req: Request, next: Next) -> Response {
    let immutable = is_content_hashed(req.uri().path());
    let mut res = next.run(req).await;
    if res.status().is_success() && !res.headers().contains_key(header::CACHE_CONTROL) {
        let v = if immutable { "public, max-age=31536000, immutable" } else { "no-cache" };
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(v));
    }
    res
}

fn is_content_hashed(path: &str) -> bool {
    if path.starts_with("/assets/") {
        return true;
    }
    let name = path.rsplit('/').next().unwrap_or_default();
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() >= 3 && {
        let h = parts[parts.len() - 2];
        h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

/// Plain-HTTP listener that redirects everything to HTTPS (used alongside TLS in production).
pub fn https_redirect(https_port: u16) -> Router {
    Router::new().fallback(move |req: Request| async move {
        let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
        let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
        let port = if https_port == 443 { String::new() } else { format!(":{https_port}") };
        let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
        (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, format!("https://{host}{port}{path}"))]).into_response()
    })
}
