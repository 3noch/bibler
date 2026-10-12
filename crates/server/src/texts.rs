//! Known texts and their valid ordinal ranges, read from text-build output (`<dir>/<text_id>/manifest.json` + the
//! index it names).

pub use api::check::Texts;
use std::path::Path;

/// Load every `<text_id>/manifest.json` under `dir`.
///
/// # Errors
/// If `dir` is unreadable or an index is malformed.
pub fn load(dir: &Path) -> Result<Texts, String> {
    let mut texts = Texts::default();
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
    }
    Ok(texts)
}
