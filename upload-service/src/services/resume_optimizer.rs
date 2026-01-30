use serde::{Deserialize, Serialize};

use super::{
    ats_optimizer::ATSAnalysis,
    deepseek_client::{DeepSeekClient, ExtractedResumeInfo},
};

/// 简历优化引擎
pub struct ResumeOptimizer {
    deepseek: DeepSeekClient,
}

impl ResumeOptimizer {
    pub fn new(deepseek: DeepSeekClient) -> Self {
        Self { deepseek }
    }

    /// 智能优化简历
    pub async fn optimize_resume(
        &self,
        original: &ExtractedResumeInfo,
        ats_analysis: &ATSAnalysis,
        config: OptimizationConfig,
    ) -> Result<OptimizedResume, anyhow::Error> {
        tracing::info!("🎯 开始智能优化简历");

        let prompt = Self::build_optimization_prompt(original, ats_analysis, &config);

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

        let optimized: OptimizedResume = serde_json::from_str(&result.response).map_err(|e| {
            tracing::error!("优化结果解析失败: {}", e);
            anyhow::anyhow!("优化失败: {}", e)
        })?;

        tracing::info!(
            "✅ 简历优化完成，预估ATS提升: {} → {}",
            ats_analysis.ats_score,
            optimized.estimated_ats_score
        );

        Ok(optimized)
    }

    /// 构建优化prompt
    fn build_optimization_prompt(
        original: &ExtractedResumeInfo,
        ats_analysis: &ATSAnalysis,
        config: &OptimizationConfig,
    ) -> String {
        format!(
            r#"你是专业的简历优化专家。请根据以下信息优化简历。

原始简历信息：
{}

ATS分析结果：
- 当前ATS评分: {}/100
- 缺失关键词: {}个
- 结构问题: {}个

目标：
- 行业: {}
- 职位: {}
- 经验级别: {}

优化要求：
1. 自然嵌入缺失的关键词
2. 量化所有成就（添加数字、百分比）
3. 使用动作动词开头
4. 使用STAR法则重写工作经验
5. 突出核心竞争力

返回JSON格式（严格遵循）：
{{
  "optimized_summary": "优化后的个人简介（50-100字）",
  "optimized_skills": [
    {{"skill": "Python", "description": "精通Python开发，3年项目经验，主导过XX项目"}},
    {{"skill": "团队协作", "description": "领导5人团队，完成XX项目，提升效率30%"}}
  ],
  "optimized_experience": [
    {{
      "company": "XX公司",
      "position": "高级开发工程师",
      "duration": "2020-2023",
      "achievements": [
        "【情境】负责XX产品后端架构设计，支撑日活100万用户",
        "【任务】优化系统性能，降低响应时间",
        "【行动】采用微服务架构，引入Redis缓存，重构核心模块",
        "【结果】响应时间降低60%，成本节省40%，用户满意度提升25%"
      ]
    }}
  ],
  "added_keywords": ["关键词1", "关键词2"],
  "quantified_count": 8,
  "estimated_ats_score": 92,
  "improvement_highlights": [
    "关键词优化: 15处",
    "成就量化: 8处",
    "STAR法则重写: 3个经历"
  ]
}}

要求：严格返回JSON，不要其他说明。"#,
            serde_json::to_string_pretty(original).unwrap_or_default(),
            ats_analysis.ats_score,
            ats_analysis.missing_keywords.len(),
            ats_analysis.structure_issues.len(),
            config.target_industry.as_deref().unwrap_or("通用"),
            config.target_position.as_deref().unwrap_or("通用职位"),
            config.experience_level.as_deref().unwrap_or("中级")
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationConfig {
    pub target_industry: Option<String>,
    pub target_position: Option<String>,
    pub experience_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedResume {
    pub optimized_summary: String,
    pub optimized_skills: Vec<OptimizedSkill>,
    pub optimized_experience: Vec<OptimizedExperience>,
    pub added_keywords: Vec<String>,
    pub quantified_count: i32,
    pub estimated_ats_score: i32,
    pub improvement_highlights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedSkill {
    pub skill: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedExperience {
    pub company: String,
    pub position: String,
    pub duration: String,
    pub achievements: Vec<String>,
}
