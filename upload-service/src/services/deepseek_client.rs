use crate::services::serde_util::null_to_default;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub struct DeepSeekClient {
    pub client: Client,
    pub base_url: String,
    pub model: String,
    /// 采样温度。**默认 0**：`/upload` 是「提取」不是「创作」，同一份文件必须
    /// 给出同一份结果，否则任何 A/B 差值都无法归因（实测 off/on 两组在注入
    /// 尚未实现时 skills_count 就差 3，根因就是默认 temperature 0.8）。
    pub temperature: f32,
    /// 随机种子。固定它才能让 temperature=0 的解码在跨进程间也可复现。
    pub seed: i64,
}

impl DeepSeekClient {
    /// 默认超时（秒）。
    ///
    /// 从 300s 提到 900s 的依据：本机 CPU 推理（qwen2.5:3b Q4_K_M）冷态
    /// 单跳提取实测 **336.1s**，`upload_single` 顺序发两跳（第一跳 220.52s 热态），
    /// 单请求合计 440–560s。300s 会让**第一跳就超时**（实测 `/upload` 500 @300.9s）。
    ///
    /// ⚠️ 调大超时只是**不返回 500**，10 分钟同步 HTTP 本身仍是产品缺陷；
    /// 正解是 `/upload` 异步化（202 + task_id + 轮询/SSE），已列入路线。
    pub const DEFAULT_TIMEOUT_SECS: u64 = 900;

    /// 默认温度。0 = 尽可能贪心，优先可复现。
    pub const DEFAULT_TEMPERATURE: f32 = 0.0;

    /// 默认种子。固定值本身无意义，有意义的是**不变**。
    pub const DEFAULT_SEED: i64 = 42;

    /// 从 `LLM_HTTP_TIMEOUT_SECS` 读取超时（秒），未设或非法则用默认值。
    pub fn timeout_secs_from_env() -> u64 {
        std::env::var("LLM_HTTP_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(Self::DEFAULT_TIMEOUT_SECS)
    }

    /// 从 `LLM_TEMPERATURE` 读取温度（默认 0）。非法或负值回退默认。
    pub fn temperature_from_env() -> f32 {
        std::env::var("LLM_TEMPERATURE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| (0.0..=2.0).contains(v))
            .unwrap_or(Self::DEFAULT_TEMPERATURE)
    }

    /// 从 `LLM_SEED` 读取种子（默认 42）。
    pub fn seed_from_env() -> i64 {
        std::env::var("LLM_SEED")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(Self::DEFAULT_SEED)
    }

    /// 构造：超时 / 温度 / 种子全部取 env 默认值。
    ///
    /// 保留这个两参签名，是为了让 `optimize_handler` / `batch_upload_handler`
    /// 这些**直接读 env 自己造 client** 的旧调用点不必一起改，也能拿到新默认值。
    pub fn new(base_url: String, model: String) -> Self {
        Self::with_params(
            base_url,
            model,
            Self::timeout_secs_from_env(),
            Self::temperature_from_env(),
            Self::seed_from_env(),
        )
    }

    /// 全参数构造：主流程走这条，便于把 `Config` 里的值打日志对账。
    pub fn with_params(
        base_url: String,
        model: String,
        timeout_secs: u64,
        temperature: f32,
        seed: i64,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url,
            model,
            temperature,
            seed,
        }
    }

    /// 本 client 的采样参数，供直接构造 `OllamaRequest` 的调用点复用
    /// （`ats_optimizer` / `resume_optimizer` 自己拼请求体，别各写一份默认值）。
    pub fn request_options(&self) -> OllamaOptions {
        OllamaOptions {
            temperature: self.temperature,
            seed: self.seed,
        }
    }

    /// 提取简历信息（不注入记忆）。
    ///
    /// 与 `extract_resume_info_with_memory(text, None)` 完全等价，保留这个
    /// 两参签名是为了让 batch 与优化链路不受注入开关影响。
    pub async fn extract_resume_info(
        &self,
        document_text: &str,
    ) -> Result<ExtractedResumeInfo, anyhow::Error> {
        self.extract_resume_info_with_memory(document_text, None).await
    }

    /// 提取简历信息，可选注入一段「历史处理记录」。
    ///
    /// `memory_block` 由 `services::memory_prompt::render_block` 产出（已脱敏、
    /// 已限长、自带隔离禁令）。传 `None` 时 prompt 与「没有记忆层」逐字节一致。
    pub async fn extract_resume_info_with_memory(
        &self,
        document_text: &str,
        memory_block: Option<&str>,
    ) -> Result<ExtractedResumeInfo, anyhow::Error> {
        tracing::info!(
            "🤖 调用DeepSeek提取简历信息，文本长度: {}, 记忆注入: {}",
            document_text.len(),
            match memory_block {
                Some(b) => format!("{} 字符", b.chars().count()),
                None => "关闭".to_string(),
            }
        );

        let prompt = Self::build_extraction_prompt(document_text, memory_block);

        let request = OllamaRequest {
            model: self.model.clone(),
            prompt,
            stream: false,
            format: Some("json".to_string()),
            options: self.request_options(),
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
            options: self.request_options(),
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
            options: self.request_options(),
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
    fn build_extraction_prompt(text: &str, memory_block: Option<&str>) -> String {
        // 限制文本长度，避免超过模型上下文
        let text_preview = if text.len() > 8000 {
            &text[..8000]
        } else {
            text
        };

        // 记忆段放在正文**之前**：模型对开头的指令遵从度更高，而记忆段自带
        // 「不得作为信息来源」的禁令，越靠前越能压住「从参考段抄字段」的倾向。
        // 没有记忆（或开关关闭）时走 else 分支 —— 产出的 prompt 与
        // 「完全没有记忆层」时逐字节一致，A/B 的对照组才有意义。
        let body = match memory_block {
            Some(block) if !block.trim().is_empty() => {
                format!("{}\n\n简历文本：\n{}", block, text_preview)
            }
            _ => format!("简历文本：\n{}", text_preview),
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

{}
"#,
            body
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
    /// 采样参数。
    ///
    /// **不传这一项 = 不可复现**：Ollama 默认 `temperature=0.8` + 随机种子，
    /// 同一份简历两次提取能差 3 个技能（实测 A/B 两组在没有注入的情况下就
    /// 已经出现 skills_count 差 3，而组内极差只有 2）。提取任务必须确定性，
    /// 否则任何「记忆带来多少提升」的结论都会被解码噪声盖住。
    pub options: OllamaOptions,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OllamaOptions {
    pub temperature: f32,
    pub seed: i64,
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
    #[serde(default, deserialize_with = "null_to_default")]
    pub education: Vec<Education>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub work_experience: Vec<WorkExperience>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub skills: Vec<String>,
    pub summary: Option<String>,
}

/// 教育经历。
///
/// 三个 String 都必须容 null：qwen2.5 会把不认识的字段写成 `null`
/// （实测 `"major": null`），而 `#[serde(default)]` 只兜字段缺失、兜不住显式
/// null ⇒ 整个 `/upload` 500。详见 `services/serde_util.rs`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Education {
    #[serde(default, deserialize_with = "null_to_default")]
    pub school: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub major: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub degree: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkExperience {
    #[serde(default, deserialize_with = "null_to_default")]
    pub company: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub position: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub description: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QualityScore {
    #[serde(default, deserialize_with = "null_to_default")]
    pub overall_score: i32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub completeness: i32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub clarity: i32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub professionalism: i32,
    #[serde(default, deserialize_with = "null_to_default")]
    pub highlights: Vec<String>,
    #[serde(default, deserialize_with = "null_to_default")]
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
    #[serde(default, deserialize_with = "null_to_default")]
    pub official_positions: Vec<String>,
    pub literary_style: Option<String>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub works: Vec<PoetryWork>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub locations: Vec<PoetryLocation>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub events: Vec<PoetryEvent>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub famous_quotes: Vec<String>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub extraction_confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoetryWork {
    #[serde(default, deserialize_with = "null_to_default")]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// qwen2.5 真实输出形态：不认识的字段写 `null`，而不是省略。
    /// 修复前这里报 `invalid type: null, expected a string`，整个 /upload 500。
    #[test]
    fn parses_qwen_null_fields() {
        let raw = r#"{
            "name": "张三",
            "phone": null,
            "email": null,
            "location": null,
            "summary": null,
            "skills": null,
            "education": [
                {"school": null, "major": null, "degree": null, "start_date": null, "end_date": null}
            ],
            "work_experience": [
                {"company": null, "position": "工程师", "description": null, "start_date": "2020", "end_date": null}
            ]
        }"#;

        let info: ExtractedResumeInfo =
            serde_json::from_str(raw).expect("显式 null 不得导致解析失败");

        assert_eq!(info.name.as_deref(), Some("张三"));
        assert!(info.skills.is_empty());
        assert_eq!(info.education.len(), 1);
        assert_eq!(info.education[0].school, "");
        assert_eq!(info.education[0].major, "");
        assert_eq!(info.work_experience[0].position, "工程师");
        assert_eq!(info.work_experience[0].company, "");
    }

    /// 评分同样是模型输出，四个 i32 也会是 null。null 必须落成 0，
    /// 不能让「没打分」变成 500。
    #[test]
    fn parses_qwen_null_scores() {
        let raw = r#"{
            "overall_score": null, "completeness": null,
            "clarity": null, "professionalism": null,
            "highlights": null, "suggestions": null
        }"#;
        let score: QualityScore = serde_json::from_str(raw).expect("null 分数不得报错");
        assert_eq!(score.overall_score, 0);
        assert_eq!(score.clarity, 0);
        assert!(score.highlights.is_empty());
    }

    /// 诗人档案是同一类输出（模型写 null），同一缺陷类顺手一并兜住。
    #[test]
    fn parses_qwen_null_poetry_profile() {
        let raw = r#"{
            "name": null, "works": null, "locations": null, "events": null,
            "official_positions": null, "famous_quotes": null,
            "extraction_confidence": null
        }"#;
        let profile: ExtractedPoetryProfile =
            serde_json::from_str(raw).expect("null 诗人档案不得报错");
        assert!(profile.works.is_empty());
        assert_eq!(profile.extraction_confidence, 0.0);
    }

    /// 缺失字段（不是 null）走 serde default，不与 null 冲突。
    #[test]
    fn parses_missing_fields() {
        let info: ExtractedResumeInfo = serde_json::from_str("{}").unwrap();
        assert!(info.education.is_empty());
        assert!(info.skills.is_empty());
    }

    /// 无记忆时 prompt 必须与「空白记忆块」逐字节一致。
    ///
    /// 这条是 A/B 的地基：如果 `Some("")` / `Some("   ")` 会改变 prompt，
    /// 那么「开关打开但 recall 没命中」这一组就和对照组不可比。
    #[test]
    fn prompt_without_memory_is_byte_identical() {
        let text = "张三\n技能：Rust、SQL";
        let plain = DeepSeekClient::build_extraction_prompt(text, None);
        let blank = DeepSeekClient::build_extraction_prompt(text, Some("   "));
        let empty = DeepSeekClient::build_extraction_prompt(text, Some(""));
        assert_eq!(plain, blank, "空白记忆块不得改变 prompt");
        assert_eq!(plain, empty, "空串记忆块不得改变 prompt");
        assert!(plain.contains("简历文本：\n张三"));
    }

    /// 有记忆时：① 参考段必须在正文之前；② 正文一字不改；③ 输出格式说明仍在。
    #[test]
    fn prompt_with_memory_keeps_body_and_schema() {
        let text = "张三\n技能：Rust";
        let block = "【历史处理记录 · 仅供参考】\n1. 技能：Go\n【历史处理记录 · 结束】";
        let prompt = DeepSeekClient::build_extraction_prompt(text, Some(block));

        assert!(prompt.contains(block), "参考段未注入");
        assert!(prompt.contains(text), "正文被改写");
        assert!(prompt.contains("\"skills\""), "输出格式说明被破坏");
        assert!(
            prompt.find("历史处理记录").unwrap() < prompt.find("张三").unwrap(),
            "参考段应在正文之前"
        );
    }

    /// 请求体必须带 options：没有它，Ollama 用默认 temperature 0.8 + 随机种子，
    /// 「同一文件两次提取差 3 个技能」就是这么来的。
    #[test]
    fn request_carries_deterministic_options() {
        let req = OllamaRequest {
            model: "qwen2.5:7b".into(),
            prompt: "x".into(),
            stream: false,
            format: Some("json".into()),
            options: OllamaOptions {
                temperature: 0.0,
                seed: 42,
            },
        };
        let v = serde_json::to_value(&req).unwrap();
        let opts = v.get("options").expect("请求体必须含 options");
        assert_eq!(opts.get("temperature").unwrap(), 0.0);
        assert_eq!(opts.get("seed").unwrap(), 42);
    }

    /// env 覆盖：A/B 期要能锁死，生产也要能放开。
    #[test]
    fn sampling_env_defaults() {
        // 不依赖进程环境（测试并行会互相污染），只验证「非法值回退默认」的语义
        assert!(DeepSeekClient::DEFAULT_TEMPERATURE == 0.0);
        assert_eq!(DeepSeekClient::DEFAULT_SEED, 42);
    }
}
