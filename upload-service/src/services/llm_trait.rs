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

    /// 提取简历信息，可选注入一段「历史处理记录」（已脱敏的文本块）。
    ///
    /// 默认实现**忽略** `memory_block`，直接转发到 `extract_resume_info` ——
    /// 进程内 GGUF 实现不参与记忆注入，这样加这条能力不用改它。
    /// 只有走 Ollama 协议的 `DeepSeekClient` 覆写它，把记忆拼进 prompt 开头。
    async fn extract_resume_info_with_memory(
        &self,
        document_text: &str,
        memory_block: Option<&str>,
    ) -> Result<ExtractedResumeInfo, anyhow::Error> {
        let _ = memory_block;
        self.extract_resume_info(document_text).await
    }

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

    /// 覆写：真正把记忆拼进 prompt。用 UFCS 明确指向 inherent 方法，
    /// 避免与 trait 同名方法解析歧义（写成 `self.xxx` 也能过，但依赖
    /// 「inherent 优先于 trait」这条隐式规则，不够显眼）。
    async fn extract_resume_info_with_memory(
        &self,
        document_text: &str,
        memory_block: Option<&str>,
    ) -> Result<ExtractedResumeInfo, anyhow::Error> {
        crate::services::deepseek_client::DeepSeekClient::extract_resume_info_with_memory(
            self,
            document_text,
            memory_block,
        )
        .await
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
