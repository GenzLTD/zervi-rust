use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadRecord {
    pub id: String,
    pub original_name: String,
    pub file_size: i64,
    pub mime_type: String,
    pub access_url: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<ClassificationDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_info: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadRecordWithMemory {
    #[serde(flatten)]
    pub record: UploadRecord,
    /// 本次请求记忆层是否真的参与了（recall 与 remember 都成功）。
    ///
    /// 为什么进响应体：终端机单用户看不到服务端日志，降级必须让客户端可见，
    /// 否则是静默功能退化 —— 比 500 更难查。
    pub memory_used: bool,
    /// 没参与的原因（未启用 / 启动失败 / 超时 / 子进程已退出）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_skip_reason: Option<String>,
    /// **A/B 测量面**：仅 `UPLOAD_EXTRACTION_DIGEST=true` 时出现。
    ///
    /// 为什么必须有它：`/upload` 原本只返回分类 + JD，`extracted_info` /
    /// `quality_score` 算完就丢（既不回传也不落库），所以「有记忆 vs 无记忆」
    /// 两轮在响应体上看不出任何差异 —— 那种对比只能得出「两个 200 一样」。
    /// 默认关闭 ⇒ 对外契约形状不变。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extraction_digest: Option<ExtractionDigest>,
}

/// 一次提取的**可测量指纹**（A/B 专用，默认不出现）。
///
/// 选这些字段的依据：「记忆有用」在简历场景里通常表现为**多识别出几项技能 /
/// 多认出几段经历**，只看总分容易被抹平。所以技能、条数、四项评分、耗时
/// 全留，且**不含姓名/手机/邮箱**（测量面不该扩大 PII 面）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionDigest {
    /// 本次记忆层是否参与（= 外层 `memory_used` 的副本，为了单条记录自洽，
    /// 便于 A/B 脚本只看 digest 就能判组）。
    pub memory_used: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_skip_reason: Option<String>,
    /// 记忆是否**真的进了 prompt**（不是「recall 成功」，而是注入块非空）。
    ///
    /// 为什么必须单独有这一位：`memory_used=true` 只说明 recall/remember 调通了。
    /// 若 `MEMORY_PROMPT_INJECTION=false`，或 recall 命中 0 条，prompt 与对照组
    /// 逐字节一致 —— 此时 Δ 必然为 0。没有这一位，注入阶段看到 Δ=0 时无法区分
    /// 「记忆没用」和「记忆根本没进去」。
    pub memory_injected: bool,
    /// 注入块字符数（0 = 未注入）。用来确认预算生效、以及为 token 成本留证据。
    pub memory_injected_chars: usize,
    /// 这次**实际注入了哪几条记忆**（cortex-mcp 的记录 id，顺序即 prompt 里的顺序）。
    ///
    /// 为什么需要它：`recall` 没有相似度阈值 —— 库非空时必然返回 k 条，
    /// 所以 `memory_injected=true` 只说明「有东西进了 prompt」，与「进的是不是
    /// 同类岗位」无关。把 id 留下来，事后能按 id 回查内容，才分得清 A/B 的 Δ
    /// 是「记忆有用」还是「preload 内容跑偏把模型带歪」。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub memory_injected_ids: Vec<String>,
    /// `resume` / `job_description` / `poetry_profile` / `other`
    pub doc_type: String,
    /// 传统分类（`Resume` / `CoverLetter` / `Certificate` / `Other`）
    pub category: String,
    pub classification_confidence: f64,
    /// 提取（第一跳 LLM）耗时；非简历/诗人档案为 null
    pub extraction_duration_ms: Option<i64>,
    /// 质量评分（第二跳 LLM）耗时；非简历为 null
    pub scoring_duration_ms: Option<i64>,
    /// 整个 process_document 的墙钟耗时
    pub processing_time_ms: i64,
    pub quality: QualityDigest,
    /// 技能前 10（顺序来自模型输出，不做排序，保持可比性）
    pub skills: Vec<String>,
    pub skills_count: usize,
    pub education_count: usize,
    pub work_count: usize,
    pub extraction_confidence: f64,
    pub quality_confidence: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct QualityDigest {
    pub overall_score: i32,
    pub completeness: i32,
    pub clarity: i32,
    pub professionalism: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationDto {
    pub doc_type: String,
    pub confidence: f64,
    pub matched_features: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadListItem {
    pub id: String,
    pub original_name: String,
    pub file_size: i64,
    pub mime_type: String,
    pub storage_type: String,
    pub status: String,
    pub access_url: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadDetail {
    pub id: String,
    pub user_id: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_path: String,
    pub file_size: i64,
    pub mime_type: String,
    pub storage_type: String,
    pub status: String,
    pub access_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUploadResult {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub processing: usize,
    pub results: Vec<BatchUploadItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUploadItem {
    pub file_name: String,
    pub status: String,
    pub upload_id: Option<String>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<ClassificationDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_info: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParsedDocumentDto {
    pub id: String,
    pub upload_id: String,
    pub text_content: String,
    pub page_count: i32,
    pub tables_count: i32,
    pub parsed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_text_length: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_confidence: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumeInfoDto {
    pub id: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub location: Option<String>,
    pub summary: Option<String>,
    pub education: Vec<serde_json::Value>,
    pub work_experience: Vec<serde_json::Value>,
    pub skills: Vec<String>,
    pub quality_score: i32,
    pub completeness_score: i32,
    pub clarity_score: i32,
    pub professionalism_score: i32,
    pub quality_confidence: f64,
    pub extraction_confidence: f64,
    pub category: String,
    pub extracted_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_record() -> UploadRecord {
        UploadRecord {
            id: "up-1".into(),
            original_name: "cv.pdf".into(),
            file_size: 1024,
            mime_type: "application/pdf".into(),
            access_url: "/uploads/up-1".into(),
            created_at: "2026-10-17 10:00:00".into(),
            classification: None,
            jd_info: None,
        }
    }

    fn base_digest() -> ExtractionDigest {
        ExtractionDigest {
            memory_used: true,
            memory_skip_reason: None,
            memory_injected: true,
            memory_injected_chars: 512,
            memory_injected_ids: vec!["m-1".into(), "m-2".into()],
            doc_type: "Resume".into(),
            category: "Resume".into(),
            classification_confidence: 0.93,
            extraction_duration_ms: Some(1200),
            scoring_duration_ms: Some(800),
            processing_time_ms: 2400,
            quality: QualityDigest {
                overall_score: 78,
                completeness: 80,
                clarity: 75,
                professionalism: 79,
            },
            skills: vec!["Rust".into(), "SQL".into()],
            skills_count: 12,
            education_count: 2,
            work_count: 3,
            extraction_confidence: 0.88,
            quality_confidence: 0.78,
        }
    }

    /// 默认（测量面关闭）⇒ 响应体形状必须与上一版**逐键一致**：
    /// 多出 `extraction_digest` 就是契约破坏。
    #[test]
    fn digest_absent_keeps_legacy_shape() {
        let body = serde_json::to_value(UploadRecordWithMemory {
            record: base_record(),
            memory_used: false,
            memory_skip_reason: Some("MCP_ENABLED=false（记忆层未启用）".into()),
            extraction_digest: None,
        })
        .unwrap();

        assert!(body.get("extraction_digest").is_none());
        // flatten 的老字段仍在顶层
        assert_eq!(body.get("id").unwrap(), "up-1");
        assert_eq!(body.get("memory_used").unwrap(), false);
        assert!(body.get("memory_skip_reason").is_some());
    }

    /// 开启后 digest 必须带够 A/B 判读要的字段（少一个都会让基线无法解释）。
    #[test]
    fn digest_carries_ab_signals() {
        let body = serde_json::to_value(UploadRecordWithMemory {
            record: base_record(),
            memory_used: true,
            memory_skip_reason: None,
            extraction_digest: Some(base_digest()),
        })
        .unwrap();

        let digest = body.get("extraction_digest").expect("应有 extraction_digest");
        for key in [
            "memory_used",
            "memory_injected",
            "memory_injected_chars",
            "memory_injected_ids",
            "doc_type",
            "category",
            "classification_confidence",
            "extraction_duration_ms",
            "scoring_duration_ms",
            "processing_time_ms",
            "quality",
            "skills",
            "skills_count",
            "education_count",
            "work_count",
            "extraction_confidence",
            "quality_confidence",
        ] {
            assert!(digest.get(key).is_some(), "digest 缺字段: {key}");
        }
        for key in ["overall_score", "completeness", "clarity", "professionalism"] {
            assert!(
                digest.get("quality").unwrap().get(key).is_some(),
                "quality 缺字段: {key}"
            );
        }
        // 测量面不得扩 PII：digest 里不允许出现姓名/手机/邮箱
        let text = serde_json::to_string(&digest).unwrap();
        assert!(!text.contains("phone") && !text.contains("email"));
    }
}
