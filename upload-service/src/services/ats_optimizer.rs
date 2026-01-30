use serde::{Deserialize, Serialize};

use super::deepseek_client::DeepSeekClient;

/// ATS优化分析器
pub struct ATSOptimizer {
    deepseek: DeepSeekClient,
}

impl ATSOptimizer {
    pub fn new(deepseek: DeepSeekClient) -> Self {
        Self { deepseek }
    }

    /// ATS兼容性分析
    pub async fn analyze_ats_compatibility(
        &self,
        resume_text: &str,
        job_description: Option<&str>,
    ) -> Result<ATSAnalysis, anyhow::Error> {
        tracing::info!("📊 开始ATS兼容性分析");

        let prompt = Self::build_ats_analysis_prompt(resume_text, job_description);

        let request = super::deepseek_client::OllamaRequest {
            model: "deepseek-r1:7b".to_string(),
            prompt,
            stream: false,
            format: Some("json".to_string()),
        };

        let response = self
            .deepseek
            .client
            .post(&format!("{}/api/generate", self.deepseek.base_url))
            .json(&request)
            .send()
            .await?;

        let result: super::deepseek_client::OllamaResponse = response.json().await?;

        let analysis: ATSAnalysis = serde_json::from_str(&result.response).map_err(|e| {
            tracing::error!("ATS分析结果解析失败: {}", e);
            tracing::debug!("原始响应: {}", result.response);
            anyhow::anyhow!("ATS分析失败: {}", e)
        })?;

        tracing::info!("✅ ATS分析完成，评分: {}/100", analysis.ats_score);

        Ok(analysis)
    }

    /// 构建ATS分析prompt
    fn build_ats_analysis_prompt(resume_text: &str, job_description: Option<&str>) -> String {
        let text_preview = if resume_text.len() > 6000 {
            &resume_text[..6000]
        } else {
            resume_text
        };

        let jd_section = job_description
            .map(|jd| format!("\n\n目标职位描述：\n{}\n", jd))
            .unwrap_or_default();

        format!(
            r#"你是ATS（Applicant Tracking System）优化专家。请分析以下简历的ATS兼容性。

简历内容：
{}{}

请返回JSON格式分析（严格遵循格式）：
{{
  "ats_score": 85,
  "keyword_density": 78,
  "structure_score": 90,
  "missing_keywords": [
    {{"keyword": "Python", "importance": "high", "reason": "技术栈核心"}},
    {{"keyword": "团队协作", "importance": "medium", "reason": "软技能"}}
  ],
  "keyword_suggestions": [
    {{"current": "熟悉Java", "suggested": "精通Java，3年项目经验", "reason": "更具体"}},
    {{"current": "参与项目", "suggested": "主导XX项目，提升效率30%", "reason": "量化成就"}}
  ],
  "structure_issues": [
    {{"issue": "缺少技能总结", "suggestion": "添加核心技能部分"}},
    {{"issue": "工作描述简单", "suggestion": "使用STAR法则"}}
  ],
  "overall_suggestions": [
    "建议1：增加量化数据",
    "建议2：优化关键词密度"
  ]
}}

要求：严格返回JSON，不要有其他说明文字。"#,
            text_preview, jd_section
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ATSAnalysis {
    pub ats_score: i32,
    pub keyword_density: i32,
    pub structure_score: i32,
    pub missing_keywords: Vec<MissingKeyword>,
    pub keyword_suggestions: Vec<KeywordSuggestion>,
    pub structure_issues: Vec<StructureIssue>,
    pub overall_suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingKeyword {
    pub keyword: String,
    pub importance: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordSuggestion {
    pub current: String,
    pub suggested: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureIssue {
    pub issue: String,
    pub suggestion: String,
}
