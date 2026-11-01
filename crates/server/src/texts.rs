//! Known texts and their valid ordinal ranges, read from text-build output (`<dir>/<text_id>/manifest.json` + the
//! index it names), and for texts a license limits, their book files: those are served only a page at a time
//! (`page`; `api::pages`), never as files (`guard`).

pub use api::check::Texts;
use api::pages::{slice_book, Limits};
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Every text under the text directory, and each limited text's book files (in manifest order).
pub struct Loaded {
    pub texts: Texts,
    pub limited: HashMap<String, Vec<PathBuf>>,
}

/// Load every `<text_id>/manifest.json` under `dir`.
///
/// # Errors
/// If `dir` is unreadable or an index is malformed.
pub fn load(dir: &Path) -> Result<Loaded, String> {
    let mut texts = Texts::default();
    let mut limited = HashMap::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries.filter_map(Result::ok) {
        let manifest_path = entry.path().join("manifest.json");
        let Ok(raw) = std::fs::read_to_string(&manifest_path) else { continue };
        let manifest: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        let (Some(text_id), Some(index)) = (manifest["text_id"].as_str(), manifest["index"].as_str()) else {
            return Err(format!("{}: missing text_id/index", manifest_path.display()));
        };
        let bytes = std::fs::read(entry.path().join(index)).map_err(|e| format!("{index}: {e}"))?;
        texts.add_index(text_id, &bytes)?;
        if let Ok(limits) = serde_json::from_value::<Limits>(manifest["limits"].clone()) {
            texts.set_limits(text_id, limits);
            let files = manifest["books"].as_array().into_iter().flatten();
            limited.insert(
                text_id.to_string(),
                files.filter_map(|b| Some(entry.path().join(b["file"].as_str()?))).collect(),
            );
        }
    }
    Ok(Loaded { texts, limited })
}

#[derive(Deserialize)]
pub struct PageQuery {
    from: usize,
    to: usize,
}

/// `GET /api/text/<id>/page?from=<slot>&to=<slot>`: slots `from..=to` of a limited text as `{books: [book file
/// parts]}`, if they fit its limits. Not to be stored (`no-store`): a device may keep only what it shows.
pub async fn page(
    State(st): State<Arc<crate::sync::AppState>>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<PageQuery>,
) -> Response {
    let parts = match st.texts.page(&id, q.from, q.to) {
        Ok(parts) => parts,
        Err(e) => {
            let (status, message) = e.status();
            return (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), message).into_response();
        }
    };
    let Some(files) = st.limited.get(&id).cloned() else { return StatusCode::NOT_FOUND.into_response() };
    let read = crate::sync::blocking(move || {
        parts
            .iter()
            .map(|p| {
                let path = files.get(p.book).ok_or("no such book")?;
                let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let book: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
                Ok(slice_book(&book, p))
            })
            .collect::<Result<Vec<_>, String>>()
    })
    .await;
    match read {
        Ok(Ok(books)) => {
            let mut res = Json(serde_json::json!({ "books": books })).into_response();
            res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            res
        }
        Ok(Err(e)) | Err(e) => {
            tracing::error!("page of {id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// In front of `/text`: a limited text's directory serves only its manifest and index (which hold no words).
pub async fn guard(limited: Arc<Vec<String>>, req: Request, next: Next) -> Response {
    let mut parts = req.uri().path().trim_start_matches('/').splitn(2, '/');
    let (id, file) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if limited.iter().any(|l| l == id) && !(file == "manifest.json" || file.starts_with("index.")) {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(req).await
}
