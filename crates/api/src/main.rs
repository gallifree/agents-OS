use axum::{Router, routing::get};
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let web_dir = std::env::var("WEB_DIR").unwrap_or_else(|_| "web/dist/client".into());
    let spa = ServeDir::new(&web_dir).fallback(ServeFile::new(format!("{web_dir}/index.html")));
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .fallback_service(spa);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
}

async fn shutdown() {
    // Docker stops containers with SIGTERM, not Ctrl-C.
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
