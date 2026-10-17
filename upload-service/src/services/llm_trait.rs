use crate::services::deepseek_client::{ExtractedPoetryProfile, ExtractedResumeInfo, QualityScore};

/// LLM 客户端统一接口
/// 支持本地 LLM 和远程 API 两种实现
#[async_trait::async_trait]
pub trait LLMClient: Send + Sync {
    /// 提取简历信息
    async fn extract_resume_info(
        &self,
        document_text: &str,
    ) -> Result<ExtractedResumeInfo, anyhow::Error>;

    /// 提取诗人档案信息
    async fn extract_poetry_profile(
        &self,
        document_text: &str,
    ) -> Result<ExtractedPoetryProfile, anyhow::Error>;

    /// 质量评分
    async fn score_quality(
        &self,
        extracted_info: &ExtractedResumeInfo,
        document_text: &str,
    ) -> Result<QualityScore, anyhow::Error>;
}

// 为 DeepSeekClient 实现 LLMClient trait
#[async_trait::async_trait]
impl LLMClient for crate::services::deepseek_client::DeepSeekClient {
    async fn extract_resume_info(
        &self,
        document_text: &str,
    ) -> Result<ExtractedResumeInfo, anyhow::Error> {
        self.extract_resume_info(document_text).await
    }

    async fn extract_poetry_profile(
        &self,
        document_text: &str,
    ) -> Result<ExtractedPoetryProfile, anyhow::Error> {
        self.extract_poetry_profile(document_text).await
    }

    async fn score_quality(
        &self,
        extracted_info: &ExtractedResumeInfo,
        document_text: &str,
    ) -> Result<QualityScore, anyhow::Error> {
        self.score_quality(extracted_info, document_text).await
    }
}

// 为 LocalLLMClient 实现 LLMClient trait
// （进程内 GGUF 仅在 `--features local-llm` 时存在）
#[cfg(feature = "local-llm")]
#[async_trait::async_trait]
impl LLMClient for crate::services::local_llm_client::LocalLLMClient {
    async fn extract_resume_info(
        &self,
        document_text: &str,
    ) -> Result<ExtractedResumeInfo, anyhow::Error> {
        self.extract_resume_info(document_text).await
    }

    async fn extract_poetry_profile(
        &self,
        document_text: &str,
    ) -> Result<ExtractedPoetryProfile, anyhow::Error> {
        self.extract_poetry_profile(document_text).await
    }

    async fn score_quality(
        &self,
        extracted_info: &ExtractedResumeInfo,
        document_text: &str,
    ) -> Result<QualityScore, anyhow::Error> {
        self.score_quality(extracted_info, document_text).await
    }
}
