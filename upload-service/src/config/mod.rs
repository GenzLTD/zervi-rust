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
        }
    }
}
