//! bible-tags server: serves the client bundle, the scripture text artifacts, sessions and the sync WebSocket.

pub mod auth;
pub mod hub;
pub mod legacy;
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
    /// Data directory (created if missing): `accounts.sqlite` + `spaces/<id>.sqlite`.
    pub data: PathBuf,
    /// A pre-spaces database to import when `data` does not exist yet (it is only read).
    pub migrate_from: Option<PathBuf>,
    /// Mark the session cookie `Secure` (set when serving TLS).
    pub secure_cookies: bool,
    /// Sign in with Google, if configured.
    pub google: Option<auth::Google>,
    /// Public origin for OAuth redirects (default: the request's `Host`, with https under TLS).
    pub public_url: Option<String>,
}

/// Build the app: open the store, load the texts, wire the routes.
///
/// # Errors
/// If the database cannot be opened or the text directory cannot be read.
pub fn router(cfg: &Config) -> Result<Router, String> {
    let hub = Arc::new(hub::Hub::default());
    let listener: store::CommitListener = {
        let hub = hub.clone();
        Box::new(move |space, change| hub.publish(space, change))
    };
    if let Some(old) = cfg.migrate_from.as_deref().filter(|_| !cfg.data.exists()) {
        migrate(old, &cfg.data)?;
    }
    let state = Arc::new(sync::AppState {
        store: Arc::new(store::SqliteStore::open(&cfg.data, listener)?),
        hub,
        texts: texts::Texts::load(&cfg.text_dir)?,
        secure_cookies: cfg.secure_cookies,
        google: cfg.google.clone(),
        public_url: cfg.public_url.clone(),
    });
    let api = Router::new()
        .route("/health", get(health))
        .route("/session", post(sync::session))
        .route("/sync", get(sync::sync))
        .route("/logout", post(auth::logout))
        .route("/auth/google", get(auth::google_start))
        .route("/auth/google/callback", get(auth::google_callback))
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

/// Import `old` into a temporary directory, then move it into place (a failed import leaves nothing behind).
fn migrate(old: &std::path::Path, data: &std::path::Path) -> Result<(), String> {
    let tmp = data.with_extension("migrating");
    let _ = std::fs::remove_dir_all(&tmp);
    let store = store::SqliteStore::open(&tmp, Box::new(|_, _| {}))?;
    let n = legacy::migrate(old, &store).map_err(|e| format!("migrating {}: {e}", old.display()))?;
    store.close();
    std::fs::rename(&tmp, data).map_err(|e| format!("{}: {e}", data.display()))?;
    tracing::info!("imported {n} owners from {} into {}", old.display(), data.display());
    Ok(())
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
