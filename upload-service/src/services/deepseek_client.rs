use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct DeepSeekClient {
    pub client: Client,
    pub base_url: String,
    pub model: String,
}

impl DeepSeekClient {
    pub fn new(base_url: String, model: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300)) // ⭐ 从120秒增加到300秒（5分钟）
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url,
            model,
        }
    }

    /// 提取简历信息
    pub async fn extract_resume_info(
        &self,
        document_text: &str,
    ) -> Result<ExtractedResumeInfo, anyhow::Error> {
        tracing::info!(
            "🤖 调用DeepSeek提取简历信息，文本长度: {}",
            document_text.len()
        );

        let prompt = Self::build_extraction_prompt(document_text);

        let request = OllamaRequest {
            model: self.model.clone(),
            prompt,
            stream: false,
            format: Some("json".to_string()),
        };

        let response = self
            .client
            .post(&format!("{}/api/generate", self.base_url))
            .json(&request)
            .send()
            .await?;

        let result: OllamaResponse = response.json().await?;

        // 解析JSON响应
        let info: ExtractedResumeInfo = serde_json::from_str(&result.response).map_err(|e| {
            tracing::error!("DeepSeek返回格式错误: {}", e);
            tracing::debug!("原始响应: {}", result.response);
            anyhow::anyhow!("解析DeepSeek响应失败: {}", e)
        })?;

        tracing::info!(
            "✅ DeepSeek提取成功: 姓名={:?}, 技能数={}",
            info.name,
            info.skills.len()
        );

        Ok(info)
    }

    /// 提取诗人档案信息
    pub async fn extract_poetry_profile(
        &self,
        document_text: &str,
    ) -> Result<ExtractedPoetryProfile, anyhow::Error> {
        tracing::info!(
            "🤖 调用DeepSeek提取诗人档案信息，文本长度: {}",
            document_text.len()
        );

        let prompt = Self::build_poetry_extraction_prompt(document_text);

        let request = OllamaRequest {
            model: self.model.clone(),
            prompt,
            stream: false,
            format: Some("json".to_string()),
        };

        let response = self
            .client
            .post(&format!("{}/api/generate", self.base_url))
            .json(&request)
            .send()
            .await?;

        let result: OllamaResponse = response.json().await?;

        // 解析JSON响应
        let profile: ExtractedPoetryProfile =
            serde_json::from_str(&result.response).map_err(|e| {
                tracing::error!("DeepSeek返回格式错误: {}", e);
                tracing::debug!("原始响应: {}", result.response);
                anyhow::anyhow!("解析DeepSeek响应失败: {}", e)
            })?;

        tracing::info!(
            "✅ DeepSeek提取成功: 诗人={:?}, 作品数={}, 地点数={}",
            profile.name,
            profile.works.len(),
            profile.locations.len()
        );

        Ok(profile)
    }

    /// 质量评分
    pub async fn score_quality(
        &self,
        extracted_info: &ExtractedResumeInfo,
        document_text: &str,
    ) -> Result<QualityScore, anyhow::Error> {
        tracing::info!("📊 调用DeepSeek进行质量评分");

        let prompt = Self::build_quality_prompt(extracted_info, document_text);

        let request = OllamaRequest {
            model: self.model.clone(),
            prompt,
            stream: false,
            format: Some("json".to_string()),
        };

        let response = self
            .client
            .post(&format!("{}/api/generate", self.base_url))
            .json(&request)
            .send()
            .await?;

        let result: OllamaResponse = response.json().await?;
        let score: QualityScore = serde_json::from_str(&result.response)?;

        tracing::info!("✅ 质量评分完成: {}/100", score.overall_score);

        Ok(score)
    }

    /// 构建信息提取prompt
    fn build_extraction_prompt(text: &str) -> String {
        // 限制文本长度，避免超过模型上下文
        let text_preview = if text.len() > 8000 {
            &text[..8000]
        } else {
            text
        };

        format!(
            r#"请从以下简历文本中提取关键信息，以JSON格式返回。

要求：
1. 严格按照JSON格式返回
2. 如果某些信息未找到，使用null
3. 技能列表提取所有相关技能
4. 工作经验按时间倒序

返回格式：
{{
  "name": "姓名",
  "phone": "手机号",
  "email": "邮箱",
  "location": "所在城市",
  "education": [
    {{
      "school": "学校名称",
      "major": "专业",
      "degree": "学历(本科/硕士/博士)",
      "start_date": "2018-09",
      "end_date": "2022-06"
    }}
  ],
  "work_experience": [
    {{
      "company": "公司名称",
      "position": "职位",
      "description": "工作描述",
      "start_date": "2022-07",
      "end_date": "2024-10"
    }}
  ],
  "skills": ["技能1", "技能2", "技能3"],
  "summary": "个人简介摘要"
}}

简历文本：
{}
"#,
            text_preview
        )
    }

    /// 构建诗人档案提取prompt
    fn build_poetry_extraction_prompt(text: &str) -> String {
        // 限制文本长度，避免超过模型上下文
        let text_preview = if text.len() > 8000 {
            &text[..8000]
        } else {
            text
        };

        format!(
            r#"请从以下古代诗人档案文本中提取关键信息，以JSON格式返回。

要求：
1. 严格按照JSON格式返回
2. 如果某些信息未找到，使用null或空数组
3. 作品列表提取所有提到的诗词作品
4. 地点列表提取所有提到的游历、居住、出生、逝世地点
5. 事件列表提取重要的生平事件（如及第、任职、隐居等）
6. 时间信息尽量提取具体年份或朝代

返回格式：
{{
  "name": "诗人姓名",
  "courtesy_name": "字号",
  "pseudonym": "别号",
  "dynasty": "朝代（如：唐代、宋代）",
  "birth_year": "出生年份",
  "death_year": "逝世年份",
  "birthplace": "出生地/籍贯",
  "deathplace": "逝世地",
  "biography": "生平简介",
  "official_positions": ["官职1", "官职2"],
  "literary_style": "文学风格/诗派",
  "works": [
    {{
      "title": "作品标题",
      "excerpt": "名句/片段",
      "category": "作品类型（如：五言诗、七言绝句）"
    }}
  ],
  "locations": [
    {{
      "name": "地点名称",
      "type": "地点类型（如：出生地、游历地、隐居地）",
      "description": "相关描述"
    }}
  ],
  "events": [
    {{
      "year": "年份（如：天宝初年）",
      "event": "事件描述",
      "type": "事件类型（如：及第、任职、游历、隐居）"
    }}
  ],
  "famous_quotes": ["名句1", "名句2"],
  "extraction_confidence": 0.85
}}

诗人档案文本：
{}
"#,
            text_preview
        )
    }

    /// 构建质量评分prompt
    fn build_quality_prompt(info: &ExtractedResumeInfo, text: &str) -> String {
        let text_preview = if text.len() > 3000 {
            &text[..3000]
        } else {
            text
        };

        format!(
            r#"请评估以下简历的质量，返回JSON格式评分。

评分维度：
1. overall_score: 总分 (0-100)
2. completeness: 完整性 (0-100) - 信息是否齐全
3. clarity: 清晰度 (0-100) - 表达是否清晰
4. professionalism: 专业性 (0-100) - 内容是否专业

已提取信息：
{}

简历文本摘要：
{}

返回格式：
{{
  "overall_score": 85,
  "completeness": 90,
  "clarity": 80,
  "professionalism": 85,
  "highlights": ["亮点1", "亮点2"],
  "suggestions": ["建议1", "建议2"]
}}
"#,
            serde_json::to_string_pretty(info).unwrap_or_default(),
            text_preview
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaRequest {
    pub model: String,
    pub prompt: String,
    pub stream: bool,
    pub format: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaResponse {
    pub response: String,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractedResumeInfo {
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub location: Option<String>,
    #[serde(default)]
    pub education: Vec<Education>,
    #[serde(default)]
    pub work_experience: Vec<WorkExperience>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Education {
    pub school: String,
    pub major: String,
    pub degree: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkExperience {
    pub company: String,
    pub position: String,
    pub description: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QualityScore {
    #[serde(default)]
    pub overall_score: i32,
    #[serde(default)]
    pub completeness: i32,
    #[serde(default)]
    pub clarity: i32,
    #[serde(default)]
    pub professionalism: i32,
    #[serde(default)]
    pub highlights: Vec<String>,
    #[serde(default)]
    pub suggestions: Vec<String>,
}

/// 提取的诗人档案信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractedPoetryProfile {
    pub name: Option<String>,
    pub courtesy_name: Option<String>,
    pub pseudonym: Option<String>,
    pub dynasty: Option<String>,
    pub birth_year: Option<String>,
    pub death_year: Option<String>,
    pub birthplace: Option<String>,
    pub deathplace: Option<String>,
    pub biography: Option<String>,
    #[serde(default)]
    pub official_positions: Vec<String>,
    pub literary_style: Option<String>,
    #[serde(default)]
    pub works: Vec<PoetryWork>,
    #[serde(default)]
    pub locations: Vec<PoetryLocation>,
    #[serde(default)]
    pub events: Vec<PoetryEvent>,
    #[serde(default)]
    pub famous_quotes: Vec<String>,
    #[serde(default)]
    pub extraction_confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoetryWork {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoetryLocation {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoetryEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}
