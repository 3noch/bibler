//! The commit this is built from, for `/api/health` (`api::BUILD`): `BUILD_COMMIT`/`BUILD_COMMITTED_AT` if set,
//! else git (Cloudflare's builds clone the repo); empty when neither is there (e.g. the nix build has no `.git`).
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

fn main() {
    let commit = std::env::var("BUILD_COMMIT").ok().or_else(|| git(&["rev-parse", "HEAD"])).unwrap_or_default();
    let at =
        std::env::var("BUILD_COMMITTED_AT").ok().or_else(|| git(&["log", "-1", "--format=%cI"])).unwrap_or_default();
    println!("cargo:rustc-env=BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=BUILD_COMMITTED_AT={at}");
    println!("cargo:rerun-if-env-changed=BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=BUILD_COMMITTED_AT");
    // A commit or checkout moves HEAD's log. Without git, look again next build (a path that doesn't exist makes
    // cargo rerun this every time), so a build that couldn't run git doesn't stay unstamped.
    let dir = git(&["rev-parse", "--absolute-git-dir"]).unwrap_or_else(|| "no-git".into());
    for f in ["HEAD", "logs/HEAD"] {
        println!("cargo:rerun-if-changed={}", std::path::Path::new(&dir).join(f).display());
    }
}
