mod config;
mod cors;
mod headers;
mod proxy;

use std::net::SocketAddr;

use axum::{Router, routing::any};
use tracing::info;

use config::Settings;
use proxy::{AppState, health, proxy_handler};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cors-proxy=info".into()),
        )
        .init();

    let settings = Settings::from_env();
    info!("allowed origins: {:?}", settings.allowed_origins);

    let state = AppState::new(
        AppState::default_client(),
        settings.allowed_origins.clone(),
    );

    let app = Router::new()
        .route("/healthz", any(health))
        .route("/", any(proxy_handler))
        .route("/{*path}", any(proxy_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], settings.port));
    info!("cors-proxy listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
