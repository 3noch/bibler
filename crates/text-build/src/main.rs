//! Usage: text-build [--id ID]... [--sources DIR] [--fixtures DIR] [--out DIR] [--write-golden]
//! Builds each text (default: every one in `SOURCES`) from `<sources>/<its dir>` into `<out>/<text_id>/`, verifies
//! its golden file `<fixtures>/<text_id>.golden.json` (or rewrites it), then writes `<out>/texts.json`.
//! Defaults: this repo's `data/source`, `fixtures/text` and `build/text`.

use std::path::PathBuf;
use std::process::ExitCode;
use text_build::{build, golden, repo_root, source, write_artifacts, write_catalog, Golden, Source, SOURCES};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("text-build: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut ids: Vec<&'static Source> = vec![];
    let mut sources = repo_root().join("data/source");
    let mut fixtures = repo_root().join("fixtures/text");
    let mut out = repo_root().join("build/text");
    let mut write_golden = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--id" => {
                let id = value()?;
                ids.push(source(&id).ok_or(format!("unknown text {id}"))?);
            }
            "--sources" => sources = PathBuf::from(value()?),
            "--fixtures" => fixtures = PathBuf::from(value()?),
            "--out" => out = PathBuf::from(value()?),
            "--write-golden" => write_golden = true,
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    if ids.is_empty() {
        ids = SOURCES.iter().collect();
    }

    for src in ids {
        let text = build(&sources.join(src.dir)).map_err(|e| format!("{}: {e}", src.id))?;
        let g = golden(&text, src.id);
        let gp = fixtures.join(format!("{}.golden.json", src.id));
        if write_golden {
            let json = serde_json::to_string_pretty(&g).map_err(|e| e.to_string())?;
            std::fs::write(&gp, json + "\n").map_err(|e| format!("{}: {e}", gp.display()))?;
            eprintln!("wrote {}", gp.display());
        } else {
            let raw = std::fs::read_to_string(&gp).map_err(|e| format!("{}: {e}", gp.display()))?;
            let want: Golden = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            if want != g {
                return Err(format!(
                    "{}: golden mismatch: tokenization changed. This shifts user ordinals; see docs/text-format.md.\n\
                     want {want:?}\n got {g:?}",
                    src.id
                ));
            }
        }
        let dir = out.join(src.id);
        let m = write_artifacts(&text, src, &dir)?;
        eprintln!("{}: {} books, {} tokens -> {} (index {})", src.id, m.books.len(), m.tokens, dir.display(), m.index);
    }
    let texts = write_catalog(&out)?;
    eprintln!("{} texts in {}", texts.len(), out.join("texts.json").display());
    Ok(())
}
