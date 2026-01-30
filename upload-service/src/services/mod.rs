pub mod ats_optimizer;
pub mod deepseek_client;
pub mod document_classifier;
pub mod document_generator;
pub mod document_processor;
pub mod document_router;
pub mod jd_parser;
pub mod llm_trait;
pub mod local_llm_client;
pub mod mineru_client;
pub mod paddleocr_client;
pub mod pdf_parser;
pub mod pyresparser_client;
pub mod resume_optimizer;
pub mod storage;
pub mod upload_service;

pub use upload_service::UploadService;
// Document router types (只在需要时导出)
// pub use document_router::DocumentRouter;
