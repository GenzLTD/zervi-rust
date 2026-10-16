mod routes;

use common::looma::AppState;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    dotenvy::dotenv().ok();
    let state = AppState::connect().await?;
    let port: u16 = std::env::var("COMPANY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3006);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("company-service listening on {addr} (external path /v1/enterprise/*)");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, routes::router(state)).await?;
    Ok(())
}
