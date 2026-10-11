//! Usage: text-build [--src DIR] [--out DIR] [--golden FILE] [--write-golden]
//! Builds artifacts into `build/text/<text_id>/` and verifies the committed golden file (or rewrites it).

use std::path::PathBuf;
use std::process::ExitCode;
use text_build::{build, default_source, golden, golden_path, repo_root, write_artifacts, Golden, TEXT_ID};

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
    let mut src = default_source();
    let mut out = repo_root().join("build/text").join(TEXT_ID);
    let mut gp = golden_path();
    let mut write_golden = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--src" => src = PathBuf::from(args.next().ok_or("--src needs a value")?),
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a value")?),
            "--golden" => gp = PathBuf::from(args.next().ok_or("--golden needs a value")?),
            "--write-golden" => write_golden = true,
            _ => return Err(format!("unknown argument {a}")),
        }
    }

    let text = build(&src)?;
    let g = golden(&text);
    if write_golden {
        let json = serde_json::to_string_pretty(&g).map_err(|e| e.to_string())?;
        std::fs::write(&gp, json + "\n").map_err(|e| format!("{}: {e}", gp.display()))?;
        eprintln!("wrote {}", gp.display());
    } else {
        let raw = std::fs::read_to_string(&gp).map_err(|e| format!("{}: {e}", gp.display()))?;
        let want: Golden = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if want != g {
            return Err(format!(
                "golden mismatch: tokenization changed. This shifts user ordinals; see docs/text-format.md.\n\
                 want {want:?}\n got {g:?}"
            ));
        }
    }

    let m = write_artifacts(&text, &out)?;
    eprintln!("{} books, {} tokens -> {} (index {})", m.books.len(), m.tokens, out.display(), m.index);
    Ok(())
}
