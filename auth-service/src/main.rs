use axum::{
    routing::{get, post},
    Router,
};
use consul_client::ServiceRegistry;
use redis::aio::ConnectionManager;
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

use config::Config;
use utils::jwt::JwtService;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub redis: ConnectionManager,
    pub jwt_service: JwtService,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "auth_service=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("🚀 Starting Auth Service...");

    // 加载配置
    let config = Config::load()?;

    // 连接数据库（独立的adirp_auth数据库）
    let db = sea_orm::Database::connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!("✅ Connected to database: adirp_auth");

    // 连接Redis
    let redis_client =
        redis::Client::open(config.redis_url.as_str()).expect("Failed to create Redis client");
    let redis = ConnectionManager::new(redis_client)
        .await
        .expect("Failed to connect to Redis");

    tracing::info!("✅ Connected to Redis");

    // 初始化JWT服务
    let jwt_service = JwtService::new(config.jwt_secret.clone(), config.jwt_expiration);

    // 创建应用状态
    let state = AppState {
        db,
        redis,
        jwt_service,
    };

    // 注册到Consul
    let registry = ServiceRegistry::new("auth-service", config.port);
    if let Err(e) = registry.register().await {
        tracing::error!("Failed to register to Consul: {}", e);
    } else {
        tracing::info!("✅ Registered to Consul as 'auth-service'");
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
        .route("/send-code", post(handlers::send_code))
        .route("/login/code", post(handlers::login_by_code))
        .route("/login/password", post(handlers::login_by_password))
        .route("/register", post(handlers::register))
        .layer(cors)
        .with_state(state);

    // 启动服务器
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("🚀 Auth Service listening on http://{}", addr);
    tracing::info!("📍 Consul: {}", config.consul_url);
    tracing::info!("📍 Database: adirp_auth");
    tracing::info!("🔍 Endpoints:");
    tracing::info!("   - /health");
    tracing::info!("   - /send-code");
    tracing::info!("   - /login/code");
    tracing::info!("   - /login/password");
    tracing::info!("   - /register");

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
