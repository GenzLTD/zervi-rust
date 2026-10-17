use std::env;

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub consul_url: String,
    pub jwt_secret: String,
    pub jwt_expiration: i64,
    pub upload_dir: String,
    pub base_url: String,
    pub max_file_size: u64,
    // 智能路由：多引擎支持
    pub pyresparser_url: String,
    pub paddleocr_url: String,
    pub mineru_url: Option<String>,
    pub deepseek_url: String,
    pub deepseek_model: String,
    pub enable_parsing: bool,
    // 引擎开关
    pub enable_pyresparser: bool,
    pub enable_mineru: bool,
    pub enable_deepseek: bool,
    // 本地 LLM 配置（自主可控）
    pub use_local_llm: bool,
    pub local_llm_model_path: String,
    pub local_llm_context_size: usize,
    pub local_llm_n_threads: Option<usize>,
    // Phase 1改进: 批量上传配置
    pub db_min_connections: u32,
    pub db_max_connections: u32,
    pub max_concurrent_uploads: usize,
    pub max_batch_size: usize,
    // LLM HTTP 超时（秒）
    //
    // 为什么需要它：`deepseek_client.rs` 原来硬编码 300s，而本机 CPU 推理冷态
    // 单跳提取实测 336.1s、`upload_single` 顺序发两跳（第一跳 220.52s）⇒ 300s
    // 必然超时。原「300s」既不成立也不可调，所以提为 env。
    pub llm_http_timeout_secs: u64,
    // 采样参数（决定「可复现性」）
    //
    // `/upload` 是提取任务：同一份文件必须给同一份结果。Ollama 默认
    // temperature=0.8 + 随机种子 ⇒ 实测同一文件两次提取 skills_count 差 3，
    // A/B 还没接注入就已经「差 ≠ 0」，仪器不合格。
    pub llm_temperature: f32,
    pub llm_seed: i64,
    // 记忆层（cortex-mcp，作为 stdio 子进程随终端起停）
    pub mcp_enabled: bool,
    pub mcp_python: String,
    pub mcp_module: String,
    pub mcp_cwd: Option<String>,
    // 注入给 cortex-mcp 子进程的 DSN
    //
    // ⚠️ 必须注入：cortex-mcp 的 `config.py` 默认 DSN 是 `...:5432/cortex`，
    // 而 compose 把宿主 5433 映射进容器 ⇒ 不覆盖就会连到错端口（或连上 ServBay
    // 上另一个库）。握手不碰库所以看不出来，一落库就错。
    pub cortex_database_url: Option<String>,
    pub mcp_connect_timeout_ms: u64,
    pub mcp_recall_timeout_ms: u64,
    pub mcp_write_timeout_ms: u64,
    // A/B 测量面（默认 false）
    //
    // 开启后 `/upload` 响应体多一个 `extraction_digest`，并且**允许**用请求头
    // `X-Memory-Mode: off` 单请求跳过记忆（只在开启时生效）。
    // 为什么用请求头而不是重启换配置：A/B 两组必须共享同一个进程 / 同一份预热
    // 过的 Ollama，否则测到的是冷启动差异，不是记忆的收益。
    pub upload_extraction_digest: bool,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv::dotenv().ok();

        Self {
            port: env::var("PORT")
                .unwrap_or_else(|_| "3007".to_string())
                .parse()
                .expect("PORT must be a number"),
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| {
                "mysql://root:adirp_password@127.0.0.1:3307/adirp_upload".to_string()
            }),
            consul_url: env::var("CONSUL_URL")
                .unwrap_or_else(|_| "http://localhost:8500".to_string()),
            jwt_secret: env::var("JWT_SECRET")
                .unwrap_or_else(|_| "adirp-secret-key-2024".to_string()),
            jwt_expiration: env::var("JWT_EXPIRATION")
                .unwrap_or_else(|_| "86400".to_string())
                .parse()
                .expect("JWT_EXPIRATION must be a number"),
            upload_dir: env::var("UPLOAD_DIR").unwrap_or_else(|_| "./uploads".to_string()),
            base_url: env::var("BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3007/files".to_string()),
            max_file_size: env::var("MAX_FILE_SIZE")
                .unwrap_or_else(|_| "104857600".to_string()) // 100MB
                .parse()
                .expect("MAX_FILE_SIZE must be a number"),
            // 智能路由：多引擎配置
            pyresparser_url: env::var("PYRESPARSER_URL")
                .unwrap_or_else(|_| "http://localhost:5001".to_string()),
            paddleocr_url: env::var("PADDLEOCR_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            mineru_url: env::var("MINERU_URL").ok(), // 可选
            deepseek_url: env::var("DEEPSEEK_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            deepseek_model: env::var("DEEPSEEK_MODEL")
                .unwrap_or_else(|_| "deepseek-r1:7b".to_string()),
            enable_parsing: env::var("ENABLE_PARSING")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
            // 引擎开关
            enable_pyresparser: env::var("ENABLE_PYRESPARSER")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
            enable_mineru: env::var("ENABLE_MINERU")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            enable_deepseek: env::var("ENABLE_DEEPSEEK")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
            // 本地 LLM 配置
            use_local_llm: env::var("USE_LOCAL_LLM")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            local_llm_model_path: env::var("LOCAL_LLM_MODEL_PATH")
                .unwrap_or_else(|_| "./models/llama-2-7b-chat.Q4_K_M.gguf".to_string()),
            local_llm_context_size: env::var("LOCAL_LLM_CONTEXT_SIZE")
                .unwrap_or_else(|_| "4096".to_string())
                .parse()
                .unwrap_or(4096),
            local_llm_n_threads: env::var("LOCAL_LLM_N_THREADS")
                .ok()
                .and_then(|s| s.parse().ok()),
            // Phase 1改进: 批量上传配置
            db_min_connections: env::var("DB_MIN_CONNECTIONS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .expect("DB_MIN_CONNECTIONS must be a number"),
            db_max_connections: env::var("DB_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "30".to_string()) // ✅ 批量处理增加到30
                .parse()
                .expect("DB_MAX_CONNECTIONS must be a number"),
            max_concurrent_uploads: env::var("MAX_CONCURRENT_UPLOADS")
                .unwrap_or_else(|_| "10".to_string()) // ✅ 最多10个并发任务
                .parse()
                .expect("MAX_CONCURRENT_UPLOADS must be a number"),
            max_batch_size: env::var("MAX_BATCH_SIZE")
                .unwrap_or_else(|_| "20".to_string()) // ✅ 一次最多20份
                .parse()
                .expect("MAX_BATCH_SIZE must be a number"),
            // LLM HTTP 超时：默认 900s（两跳冷态的余量；300s 已实测不够）
            llm_http_timeout_secs: env::var("LLM_HTTP_TIMEOUT_SECS")
                .unwrap_or_else(|_| "900".to_string())
                .parse()
                .expect("LLM_HTTP_TIMEOUT_SECS must be a number"),
            // 默认 temperature=0 / seed=42：先可复现，再谈优化。
            llm_temperature: env::var("LLM_TEMPERATURE")
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|v| (0.0..=2.0).contains(v))
                .unwrap_or(0.0),
            llm_seed: env::var("LLM_SEED")
                .ok()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(42),
            // 记忆层：默认关闭（opt-in）。启用后连不上只降级、不影响主链路。
            mcp_enabled: env::var("MCP_ENABLED")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
            mcp_python: env::var("MCP_PYTHON").unwrap_or_else(|_| "python".to_string()),
            mcp_module: env::var("MCP_MODULE")
                .unwrap_or_else(|_| "cortex_mcp.server".to_string()),
            mcp_cwd: env::var("MCP_CWD").ok().filter(|s| !s.trim().is_empty()),
            cortex_database_url: env::var("CORTEX_DATABASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            mcp_connect_timeout_ms: env::var("MCP_CONNECT_TIMEOUT_MS")
                .unwrap_or_else(|_| "20000".to_string())
                .parse()
                .unwrap_or(20_000),
            // recall 超时上限。默认 2000ms 来自真机标定（Mac / 真库 5433 /
            // nomic-embed-text）：recall 端到端热 59–72ms、冷 59ms，embedding
            // 模型冷加载单独 0.65s ⇒ 最坏 ≈ 0.7s，2000ms 有约 3× 余量。
            // 不建议低于 1000ms：一次冷加载就会贴着上限打满。
            mcp_recall_timeout_ms: env::var("MCP_RECALL_TIMEOUT_MS")
                .unwrap_or_else(|_| "2000".to_string())
                .parse()
                .unwrap_or(2_000),
            mcp_write_timeout_ms: env::var("MCP_WRITE_TIMEOUT_MS")
                .unwrap_or_else(|_| "15000".to_string())
                .parse()
                .unwrap_or(15_000),
            // A/B 测量面：默认关 ⇒ 对外契约形状完全不变。
            upload_extraction_digest: env::var("UPLOAD_EXTRACTION_DIGEST")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),
        }
    }
}
