use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, post},
    Router,
};
use consul_client::ServiceRegistry;
use sea_orm::DatabaseConnection;
use std::{net::SocketAddr, sync::Arc};
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
use services::{
    deepseek_client::DeepSeekClient, document_processor::DocumentProcessor,
    memory_layer::MemoryLayer, storage::LocalStorage,
};
// 进程内 GGUF 推理客户端（仅 `--features local-llm` 时存在）
#[cfg(feature = "local-llm")]
use services::local_llm_client::LocalLLMClient;
use utils::jwt::JwtService;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub storage: Arc<LocalStorage>,
    pub jwt_service: JwtService,
    pub processor: Arc<DocumentProcessor>,
    // 记忆层（cortex-mcp）。不可用时内部为 None，调用返回「跳过原因」。
    pub memory: Arc<MemoryLayer>,
    // Phase 1改进: 并发控制
    pub upload_semaphore: Arc<tokio::sync::Semaphore>,
    pub config: Config,
}

async fn health_check() -> &'static str {
    "Upload Service is healthy"
}

/// 进程外模型服务（Ollama 协议）→ DocumentProcessor
///
/// `DeepSeekClient` 打的是 `POST {DEEPSEEK_URL}/api/generate`，即 Ollama 原生协议，
/// 默认 `DEEPSEEK_URL=http://localhost:11434` ⇒ 本机模型进程。不需要新的客户端。
fn build_remote_processor(config: &Config) -> Arc<DocumentProcessor> {
    tracing::info!(
        "🌐 使用进程外模型服务（Ollama 协议）: {} ({})",
        config.deepseek_url,
        config.deepseek_model
    );

    let deepseek = DeepSeekClient::with_params(
        config.deepseek_url.clone(),
        config.deepseek_model.clone(),
        config.llm_http_timeout_secs,
        config.llm_temperature,
        config.llm_seed,
    );

    Arc::new(DocumentProcessor::new(
        deepseek,
        config.pyresparser_url.clone(),
        config.paddleocr_url.clone(),
        config.mineru_url.clone(),
        config.enable_pyresparser,
        true, // enable_ocr - 启用OCR自动增强
        config.enable_mineru,
    ))
}

/// 进程内 GGUF（需 `--features local-llm`）
#[cfg(feature = "local-llm")]
fn build_local_processor(config: &Config) -> anyhow::Result<Arc<DocumentProcessor>> {
    tracing::info!("🔒 使用本地 LLM（自主可控模式）");

    let local_llm = LocalLLMClient::new(
        &config.local_llm_model_path,
        Some(config.local_llm_context_size),
        config.local_llm_n_threads,
    )
    .map_err(|e| {
        tracing::error!("❌ 本地 LLM 初始化失败: {}", e);
        anyhow::anyhow!("本地 LLM 初始化失败: {}", e)
    })?;

    Ok(Arc::new(DocumentProcessor::new_with_local_llm(
        local_llm,
        config.pyresparser_url.clone(),
        config.paddleocr_url.clone(),
        config.mineru_url.clone(),
        config.enable_pyresparser,
        true, // enable_ocr - 启用OCR自动增强
        config.enable_mineru,
    )))
}

/// 未编译 `local-llm` 时的兜底：显式拒绝，而不是静默降级到进程外
#[cfg(not(feature = "local-llm"))]
fn build_local_processor(_config: &Config) -> anyhow::Result<Arc<DocumentProcessor>> {
    anyhow::bail!(
        "USE_LOCAL_LLM=true，但本二进制未编译 `local-llm` feature（进程内 GGUF 不在本轮范围）。\
         请改用进程外模型服务：设 USE_LOCAL_LLM=false，并把 DEEPSEEK_URL 指向本机模型进程\
         （默认 http://localhost:11434）。"
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "upload_service=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("🚀 Starting Upload Service...");

    // 加载配置
    let config = Config::from_env();

    // 连接数据库（优化连接池配置）
    let mut opt = sea_orm::ConnectOptions::new(&config.database_url);
    opt.min_connections(config.db_min_connections)
        .max_connections(config.db_max_connections) // ✅ 批量处理时增加连接数
        .connect_timeout(std::time::Duration::from_secs(8))
        .idle_timeout(std::time::Duration::from_secs(600)) // 10分钟
        .max_lifetime(std::time::Duration::from_secs(3600)) // 1小时
        .sqlx_logging(true)
        .sqlx_logging_level(tracing::log::LevelFilter::Debug);

    let db = sea_orm::Database::connect(opt).await?;
    tracing::info!("✅ Connected to database: adirp_upload");
    tracing::info!("   - Min connections: {}", config.db_min_connections);
    tracing::info!("   - Max connections: {}", config.db_max_connections);

    // 初始化存储
    let storage = Arc::new(LocalStorage::new(
        config.upload_dir.clone(),
        config.base_url.clone(),
    ));
    tracing::info!("✅ Storage initialized: {}", config.upload_dir);

    // 初始化JWT服务
    let jwt_service = JwtService::new(config.jwt_secret.clone(), config.jwt_expiration);

    // 初始化文档处理器（智能路由系统）
    //
    //  进程外（默认）—— `USE_LOCAL_LLM=false`：DeepSeekClient 打 `POST {DEEPSEEK_URL}/api/generate`，
    //      即 Ollama 协议，指向本机模型进程（默认 http://localhost:11434）。
    //  进程内 —— `USE_LOCAL_LLM=true`：需以 `--features local-llm` 构建（本轮不编译）。
    let processor = if config.use_local_llm {
        build_local_processor(&config)?
    } else {
        build_remote_processor(&config)
    };

    tracing::info!("✅ Document processor initialized (智能路由系统)");
    tracing::info!("   - PDF Parser: Rust native (pdf-extract)");
    tracing::info!("   - PaddleOCR: {} (自动增强)", config.paddleocr_url);
    tracing::info!(
        "   - pyresparser: {} ({})",
        config.pyresparser_url,
        if config.enable_pyresparser {
            "启用"
        } else {
            "禁用"
        }
    );
    if let Some(ref url) = config.mineru_url {
        tracing::info!(
            "   - MinerU: {} ({})",
            url,
            if config.enable_mineru {
                "启用"
            } else {
                "禁用"
            }
        );
    }
    if config.use_local_llm {
        tracing::info!(
            "   - 本地 LLM: {} (上下文: {}, 线程: {}) ✅ 自主可控",
            config.local_llm_model_path,
            config.local_llm_context_size,
            config
                .local_llm_n_threads
                .map(|n| n.to_string())
                .unwrap_or_else(|| "auto".to_string())
        );
    } else {
        tracing::info!(
            "   - DeepSeek API: {} ({}) ({})",
            config.deepseek_url,
            config.deepseek_model,
            if config.enable_deepseek {
                "启用"
            } else {
                "禁用"
            }
        );
    }

    // 初始化记忆层（cortex-mcp，stdio 子进程）
    //
    //  失败语义：**跳过记忆继续**。连不上只降级，唯一可见的降级信号是
    //  `/upload` 响应体里的 `memory_used:false` + `memory_skip_reason`。
    let memory = Arc::new(MemoryLayer::from_config(&config).await);
    tracing::info!(
        "   - 记忆层(cortex-mcp): {}",
        if memory.is_enabled() {
            "已启用"
        } else {
            "未启用 / 不可用（/upload 将跳过记忆）"
        }
    );
    tracing::info!(
        "   - LLM HTTP 超时: {}s（LLM_HTTP_TIMEOUT_SECS）",
        config.llm_http_timeout_secs
    );
    tracing::info!(
        "   - LLM 采样: temperature={} seed={}（LLM_TEMPERATURE / LLM_SEED）",
        config.llm_temperature,
        config.llm_seed
    );

    // 创建应用状态
    let state = AppState {
        db: db.clone(),
        storage,
        jwt_service,
        processor,
        memory: memory.clone(),
        // Phase 1改进: 添加并发控制
        upload_semaphore: Arc::new(tokio::sync::Semaphore::new(config.max_concurrent_uploads)),
        config: config.clone(),
    };

    // 计算实际的请求体限制
    let max_request_body = (config.max_batch_size as u64 * config.max_file_size) as usize;

    tracing::info!("⚙️  Performance Config:");
    tracing::info!(
        "   - Max concurrent uploads: {}",
        config.max_concurrent_uploads
    );
    tracing::info!("   - Max batch size: {}", config.max_batch_size);
    tracing::info!(
        "   - Max single file: {} MB",
        config.max_file_size / 1024 / 1024
    );
    tracing::info!(
        "   - Max request body: {} MB",
        max_request_body / 1024 / 1024
    );

    // 配置CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 构建路由
    //
    // ⚠️ `/health` 必须留在鉴权中间件**之外**：上传服务的健康检查也不带 Bearer。
    //    用 `route_layer` 只包裹受保护子路由，使无 token 的 `GET /health` = 200，
    //    而坏 token 访问 `/upload` 等仍 401（与 user-service 同构）。
    let protected = Router::new()
        // 需要JWT验证的路由
        .route("/upload", post(handlers::upload_single))
        .route("/upload/batch", post(handlers::batch_upload_and_parse))
        // Phase 1: 批量上传功能 - 暂时注释（编译错误）
        // .route("/upload/batch-resumes", post(handlers::batch_upload_resumes))
        // .route("/upload/batch-resumes-progress", get(handlers::batch_upload_with_progress))
        .route("/uploads", get(handlers::list_uploads))
        .route("/uploads/:id", get(handlers::get_upload))
        .route("/uploads/:id/parsed", get(handlers::get_parsed_result))
        .route("/uploads/:id/resume-info", get(handlers::get_resume_info))
        .route("/uploads/:id", delete(handlers::delete_upload))
        // SmartResume智能优化端点
        .route("/optimize/ats-analysis", post(handlers::analyze_ats))
        .route("/optimize/smart-rewrite", post(handlers::optimize_resume))
        .route(
            "/optimize/generate-word",
            post(handlers::generate_optimized_word),
        )
        .route(
            "/optimize/ats-report",
            post(handlers::generate_ats_report_word),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            jwt_middleware::auth_middleware,
        ));

    let app = Router::new()
        .route("/health", get(health_check))
        .merge(protected)
        // ⭐⭐⭐ 关键修复：设置 Axum multipart 的请求体大小限制
        // 计算：批量数量 × 单文件大小 = 总限制
        // 例如：30个文件 × 20MB = 600MB
        .layer(DefaultBodyLimit::max(max_request_body))
        // ⚠️ 移除 RequestBodyLimitLayer，避免覆盖 DefaultBodyLimit
        // .layer(body_limit)
        .layer(cors)
        .with_state(state);

    // 启动服务器
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("🚀 Upload Service listening on http://{}", addr);
    tracing::info!("📍 Consul: {}", config.consul_url);
    tracing::info!("📍 Database: adirp_upload");
    tracing::info!("📍 Storage: {}", config.upload_dir);
    tracing::info!("🔍 Endpoints:");
    tracing::info!("   - /health");
    tracing::info!("   📤 Upload:");
    tracing::info!("   - POST   /upload              - 单文件上传");
    tracing::info!("   - POST   /upload/batch        - 批量上传+解析");
    tracing::info!("   📦 Phase 1 - 批量上传:");
    tracing::info!("   - POST   /upload/batch-resumes          - 批量上传简历（普通）");
    tracing::info!("   - GET    /upload/batch-resumes-progress - 批量上传简历（SSE进度）");
    tracing::info!("   - GET    /uploads             - 文件列表");
    tracing::info!("   - GET    /uploads/:id         - 文件详情");
    tracing::info!("   - GET    /uploads/:id/parsed  - 解析结果");
    tracing::info!("   - GET    /uploads/:id/resume-info - 简历信息");
    tracing::info!("   - DELETE /uploads/:id         - 删除文件");
    tracing::info!("   🤖 SmartResume:");
    tracing::info!("   - POST   /optimize/ats-analysis     - ATS兼容性分析");
    tracing::info!("   - POST   /optimize/smart-rewrite    - 智能优化改写");
    tracing::info!("   - POST   /optimize/generate-word    - 生成优化Word");
    tracing::info!("   - POST   /optimize/ats-report       - 生成ATS报告");

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // 注册到Consul（可选）
    let mut consul_registry: Option<ServiceRegistry> = None;
    let registry = ServiceRegistry::new("upload-service", config.port);
    match registry.register().await {
        Ok(_) => {
            tracing::info!("✅ Registered to Consul as 'upload-service'");
            consul_registry = Some(registry);
        }
        Err(e) => {
            tracing::warn!(
                "Consul registration skipped: {}. Service will continue without Consul.",
                e
            );
        }
    }

    if let Some(registry) = consul_registry.as_ref() {
        if let Err(e) = registry.start_heartbeat().await {
            tracing::warn!(
                "Consul heartbeat disabled (unable to start heartbeat): {}",
                e
            );
        }
    }

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
    if let Some(registry) = consul_registry {
        if let Err(e) = registry.deregister().await {
            tracing::warn!("Failed to deregister from Consul: {}", e);
        } else {
            tracing::info!("✅ Deregistered from Consul");
        }
    }

    // 关闭记忆层子进程（否则 cortex-mcp 会变成孤儿进程）
    memory.close().await;

    tracing::info!("👋 Upload Service stopped");

    Ok(())
}
