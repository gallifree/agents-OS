mod config;
mod graphql;
mod services;

use anyhow::Result;
use async_graphql_axum::GraphQL;
use axum::{
    Router,
    routing::{get, post_service},
};
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: api [serve|health|schema]";

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("health") => health().await,
        Some("schema") => {
            print!(
                "{}",
                graphql::AppSchema::build(graphql::Query, Default::default(), Default::default())
                    .finish()
                    .sdl()
            );
            Ok(())
        }
        Some(_) => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

async fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cfg = config::Config::from_env()?;
    let (addr, web_dir) = (cfg.bind_addr.clone(), cfg.web_dir.clone());
    let schema = graphql::schema(services::Services::connect(cfg)?);

    let spa = ServeDir::new(&web_dir).fallback(ServeFile::new(format!("{web_dir}/index.html")));
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/graphql", post_service(GraphQL::new(schema)))
        .fallback_service(spa);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// Docker health check: the runtime image has no curl, so the binary checks itself.
async fn health() -> Result<()> {
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let port = addr.rsplit(':').next().unwrap_or("8080");
    reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await?
        .error_for_status()?;
    Ok(())
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
