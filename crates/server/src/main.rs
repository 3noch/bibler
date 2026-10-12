//! Usage:
//!   bible-tags-server --text DIR --data DIR [--migrate-from OLD.db] [--static DIR] [--addr HOST:PORT]
//!                     [--tls-cert PEM --tls-key PEM [--http-redirect HOST:PORT]]
//! Without TLS flags it serves plain HTTP (dev). With them it terminates TLS itself (rustls); SIGHUP reloads the
//! certificate files (for external renewal, e.g. ACME clients).

use axum_server::tls_rustls::RustlsConfig;
use server::{https_redirect, router, Config};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

struct Args {
    addr: SocketAddr,
    cfg: Config,
    tls: Option<(PathBuf, PathBuf)>,
    http_redirect: Option<SocketAddr>,
}

fn parse_args() -> Result<Args, String> {
    let mut addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let (mut static_dir, mut text_dir, mut cert, mut key, mut redirect) = (None, None, None, None, None);
    let (mut data, mut migrate_from) = (None, None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--addr" => addr = val()?.parse().map_err(|e| format!("--addr: {e}"))?,
            "--static" => static_dir = Some(PathBuf::from(val()?)),
            "--text" => text_dir = Some(PathBuf::from(val()?)),
            "--data" => data = Some(PathBuf::from(val()?)),
            "--migrate-from" => migrate_from = Some(PathBuf::from(val()?)),
            "--tls-cert" => cert = Some(PathBuf::from(val()?)),
            "--tls-key" => key = Some(PathBuf::from(val()?)),
            "--http-redirect" => redirect = Some(val()?.parse().map_err(|e| format!("--http-redirect: {e}"))?),
            "-h" | "--help" => return Err(include_str!("main.rs").lines().take(5).collect::<Vec<_>>().join("\n")),
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    let tls = match (cert, key) {
        (Some(c), Some(k)) => Some((c, k)),
        (None, None) => None,
        _ => return Err("--tls-cert and --tls-key go together".into()),
    };
    if redirect.is_some() && tls.is_none() {
        return Err("--http-redirect requires TLS".into());
    }
    let text_dir = text_dir.ok_or("--text DIR is required (text-build output, e.g. build/text)")?;
    let data = data.ok_or("--data DIR is required (SQLite databases; created if missing)")?;
    let secure_cookies = tls.is_some();
    Ok(Args {
        addr,
        cfg: Config { static_dir, text_dir, data, migrate_from, secure_cookies },
        tls,
        http_redirect: redirect,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into()))
        .init();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), String> {
    let app = router(&args.cfg)?;
    let handle = axum_server::Handle::new();
    tokio::spawn({
        let handle = handle.clone();
        async move {
            shutdown_signal().await;
            handle.graceful_shutdown(Some(Duration::from_secs(10)));
        }
    });

    match args.tls {
        None => {
            tracing::info!("serving http://{} (no TLS)", args.addr);
            axum_server::bind(args.addr).handle(handle).serve(app.into_make_service()).await.map_err(|e| e.to_string())
        }
        Some((cert, key)) => {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let tls = RustlsConfig::from_pem_file(&cert, &key).await.map_err(|e| format!("loading TLS files: {e}"))?;
            spawn_cert_reloader(tls.clone(), cert, key);
            if let Some(raddr) = args.http_redirect {
                let redirect = https_redirect(args.addr.port());
                let h = handle.clone();
                tokio::spawn(async move {
                    tracing::info!("redirecting http://{raddr} -> https");
                    if let Err(e) = axum_server::bind(raddr).handle(h).serve(redirect.into_make_service()).await {
                        tracing::error!("http redirect listener: {e}");
                    }
                });
            }
            tracing::info!("serving https://{}", args.addr);
            axum_server::bind_rustls(args.addr, tls)
                .handle(handle)
                .serve(app.into_make_service())
                .await
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(unix)]
fn spawn_cert_reloader(tls: RustlsConfig, cert: PathBuf, key: PathBuf) {
    use tokio::signal::unix::{signal, SignalKind};
    tokio::spawn(async move {
        let Ok(mut hup) = signal(SignalKind::hangup()) else { return };
        while hup.recv().await.is_some() {
            match tls.reload_from_pem_file(&cert, &key).await {
                Ok(()) => tracing::info!("reloaded TLS certificate"),
                Err(e) => tracing::error!("TLS reload failed (keeping the old certificate): {e}"),
            }
        }
    });
}

#[cfg(not(unix))]
fn spawn_cert_reloader(_: RustlsConfig, _: PathBuf, _: PathBuf) {}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    tracing::info!("shutting down");
}
