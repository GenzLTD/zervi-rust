use axum::{
    middleware,
    routing::{get, put},
    Router,
};
use consul_client::ServiceRegistry;
use sea_orm::DatabaseConnection;
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod dto;
mod handlers;
mod models;
mod services;
mod utils;

// JWT中间件模块
mod jwt_middleware;

use config::Config;
use utils::jwt::JwtService;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub jwt_service: JwtService,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "user_service=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("🚀 Starting User Service...");

    // 加载配置
    let config = Config::load()?;

    // 连接数据库（独立的adirp_user数据库）
    let db = sea_orm::Database::connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!("✅ Connected to database: adirp_user");

    // 初始化JWT服务（用于验证Token）
    let jwt_service = JwtService::new(config.jwt_secret.clone(), config.jwt_expiration);

    // 创建应用状态
    let state = AppState { db, jwt_service };

    // 注册到Consul
    let registry = ServiceRegistry::new("user-service", config.port);
    if let Err(e) = registry.register().await {
        tracing::error!("Failed to register to Consul: {}", e);
    } else {
        tracing::info!("✅ Registered to Consul as 'user-service'");
    }

    // 启动健康检查心跳
    if let Err(e) = registry.start_heartbeat().await {
        tracing::error!("Failed to start heartbeat: {}", e);
    }

    // 配置CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 构建路由
    let app = Router::new()
        .route("/health", get(health_check))
        // 需要JWT验证的路由
        .route("/profile", get(handlers::get_profile))
        .route("/profile", put(handlers::update_profile))
        .route("/avatar", put(handlers::upload_avatar))
        .route("/password", put(handlers::change_password))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            jwt_middleware::auth_middleware,
        ))
        .layer(cors)
        .with_state(state);

    // 启动服务器
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("🚀 User Service listening on http://{}", addr);
    tracing::info!("📍 Consul: {}", config.consul_url);
    tracing::info!("📍 Database: adirp_user");
    tracing::info!("🔍 Endpoints:");
    tracing::info!("   - /health");
    tracing::info!("   - GET  /profile");
    tracing::info!("   - PUT  /profile");
    tracing::info!("   - PUT  /avatar");
    tracing::info!("   - PUT  /password");

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // 使用优雅关闭
    let server = axum::serve(listener, app);

    // 等待关闭信号
    tokio::select! {
        result = server => {
            if let Err(e) = result {
                tracing::error!("Server error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received shutdown signal");
        }
    }

    // 注销Consul
    if let Err(e) = registry.deregister().await {
        tracing::error!("Failed to deregister from Consul: {}", e);
    } else {
        tracing::info!("✅ Deregistered from Consul");
    }

    Ok(())
}

/// 健康检查接口
async fn health_check() -> &'static str {
    "OK"
}
