mod connectors;
mod data;
mod error;
mod mcp;
mod orchestrator;
mod routes;
mod scheduler;
mod state;

use std::path::PathBuf;

use axum::Router;
use cortex_executor::Executor;
use cortex_store::Store;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let port: u16 = env_or("CORTEX_PORT", "7420").parse()?;
    let data_dir = PathBuf::from(env_or("CORTEX_DATA_DIR", "./data"));
    let console_dist = PathBuf::from(env_or("CORTEX_CONSOLE_DIST", "./console/dist"));

    std::fs::create_dir_all(&data_dir)?;
    // Workers inherit this and reach back into the platform (cortex.query()
    // etc.) — set before the executor exists so every worker sees it.
    if std::env::var("CORTEX_API_URL").is_err() {
        std::env::set_var("CORTEX_API_URL", format!("http://127.0.0.1:{port}"));
    }
    let store = Store::open(data_dir.join("cortex.db"))?;
    let executor = Executor::new()?;
    let state = state::AppState::new(store, executor, data_dir);

    scheduler::spawn(state.clone());

    let mut app = Router::new()
        .nest("/api", routes::api_router())
        .route("/mcp", axum::routing::post(mcp::handle))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Serve the built console when present (docker / production).
    if console_dist.join("index.html").exists() {
        info!("serving console from {}", console_dist.display());
        let spa = ServeDir::new(&console_dist)
            .fallback(ServeFile::new(console_dist.join("index.html")));
        app = app.fallback_service(spa);
    }

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    info!("cortex-server listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
