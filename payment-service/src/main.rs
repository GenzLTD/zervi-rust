//! payment-service 入口：监听 `PAYMENT_PORT`（默认 **3009**，网关约定的下一个端口）。
//!
//! 本地联调：
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/genzltd
//! APPLE_ROOT_CA_PEM=<Apple Root CA - G3 的 PEM 内容>
//! PAYMENT_PORT=3009 cargo run -p payment-service
//! ```

use common::looma::AppState;
use payment_service::routes;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let state = AppState::connect()
        .await
        .expect("database connect failed (check DATABASE_URL)");

    let port = std::env::var("PAYMENT_PORT").unwrap_or_else(|_| "3009".to_string());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("bind failed");

    tracing::info!("payment-service listening on 0.0.0.0:{port}");
    axum::serve(listener, routes::router(state))
        .await
        .expect("server error");
}
