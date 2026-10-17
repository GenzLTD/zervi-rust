pub mod ats_optimizer;
pub mod deepseek_client;
pub mod document_classifier;
pub mod document_generator;
pub mod document_processor;
pub mod document_router;
pub mod jd_parser;
pub mod llm_trait;
// 记忆层门面（托管 cortex-mcp stdio 子进程；失败降级为「跳过记忆」）
pub mod memory_layer;
// 记忆 → 提取 prompt 的注入层（脱敏 / 预算 / 强隔离）
pub mod memory_prompt;
// 进程内 GGUF 推理：仅 `--features local-llm` 时编译（需 libclang）
#[cfg(feature = "local-llm")]
pub mod local_llm_client;
pub mod mineru_client;
pub mod paddleocr_client;
pub mod pdf_parser;
pub mod pyresparser_client;
pub mod resume_optimizer;
// 模型输出的 null 兼容（显式 null → Default）
pub mod serde_util;
pub mod storage;
pub mod upload_service;

pub use upload_service::UploadService;
// Document router types (只在需要时导出)
// pub use document_router::DocumentRouter;
