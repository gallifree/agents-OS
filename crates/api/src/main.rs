mod auth;
mod config;
mod events;
mod gateway;
mod graphql;
mod runtime;
mod services;

use anyhow::Result;
use async_graphql::{Data, http::ALL_WEBSOCKET_PROTOCOLS};
use async_graphql_axum::{GraphQLProtocol, GraphQLRequest, GraphQLResponse, GraphQLWebSocket};
use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, header::COOKIE},
    response::Response,
    routing::get,
};
use services::Services;
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: api [serve|health|schema]";

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("health") => health().await,
        Some("schema") => {
            print!("{}", graphql::sdl());
            Ok(())
        }
        Some(_) => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

#[derive(Clone)]
struct AppState {
    services: Services,
    schema: graphql::AppSchema,
}

async fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cfg = config::Config::from_env()?;
    let (addr, web_dir) = (cfg.bind_addr.clone(), cfg.web_dir.clone());
    let services = Services::connect(cfg)?;

    sqlx::migrate!().run(&services.pg).await?;
    if !auth::has_users(&services.pg).await? {
        let code = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
        let shown = code
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("-");
        // shortcut: the code lives in this process; with several app replicas, store it in Valkey instead.
        tracing::warn!(
            "no users yet: open the app and create the admin account with setup code {shown}"
        );
        *services.setup_code.lock().expect("setup code lock") = Some(shown);
    }

    runtime::spawn(services.clone());

    let state = AppState {
        schema: graphql::schema(services.clone()),
        services,
    };
    let spa = ServeDir::new(&web_dir).fallback(ServeFile::new(format!("{web_dir}/index.html")));
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        // POST: queries and mutations; GET: WebSocket subscriptions (graphql-transport-ws).
        .route("/graphql", get(graphql_ws).post(graphql_http))
        // OpenAI-compatible LLM gateway, for machines (API keys, not cookies).
        .merge(gateway::router())
        .with_state(state)
        .fallback_service(spa);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// The session cookie, resolved to a viewer. A bad or revoked session is just anonymous.
async fn viewer(services: &Services, headers: &HeaderMap) -> Option<auth::Viewer> {
    let token = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(auth::cookie_token)?;
    auth::resolve_session(&services.pg, &services.keys, token)
        .await
        .ok()
        .flatten()
}

async fn graphql_http(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let mut req = req.into_inner();
    if let Some(viewer) = viewer(&state.services, &headers).await {
        req = req.data(viewer);
    }
    state.schema.execute(req).await.into()
}

async fn graphql_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    protocol: GraphQLProtocol,
    upgrade: WebSocketUpgrade,
) -> Response {
    let mut data = Data::default();
    if let Some(viewer) = viewer(&state.services, &headers).await {
        data.insert(viewer);
    }
    upgrade
        .protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |socket| {
            GraphQLWebSocket::new(socket, state.schema, protocol)
                .with_data(data)
                .serve()
        })
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
