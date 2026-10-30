mod routes;
mod sweep;

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
    // 过期 grant 的全表清扫（契约划归后台，端点只有惰性清理）。
    // 默认开、启动即跑一轮；RESUME_SWEEP_INTERVAL_SECS=0 关闭。
    sweep::spawn(state.clone());
    let port: u16 = std::env::var("RESUME_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3003);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("resume-service listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, routes::router(state)).await?;
    Ok(())
}
