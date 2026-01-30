use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;

use super::{
    deepseek_client::{DeepSeekClient, ExtractedPoetryProfile, ExtractedResumeInfo, QualityScore},
    document_classifier::{ClassificationResult, DocumentClassifier, DocumentType},
    document_router::DocumentRouter,
    jd_parser::{JDParser, JobDescription},
    llm_trait::LLMClient,
    local_llm_client::LocalLLMClient,
    mineru_client::MinerUClient,
    paddleocr_client::PaddleOCRClient,
    pdf_parser::PdfParser,
    pyresparser_client::PyResParserClient,
};

#[derive(Clone)]
pub struct DocumentProcessor {
    // 智能路由器
    _router: Arc<DocumentRouter>,
    // 文档分类器
    classifier: Arc<DocumentClassifier>,
    // JD解析器
    jd_parser: Arc<JDParser>,
    // 多引擎客户端
    _pyresparser: Arc<PyResParserClient>,
    paddleocr: Arc<PaddleOCRClient>, // OCR客户端
    _mineru: Option<Arc<MinerUClient>>,
    llm_client: Arc<dyn LLMClient>, // 统一的 LLM 客户端（支持本地/远程）
    temp_dir: PathBuf,
    // 配置
    _enable_intelligent_routing: bool,
    enable_ocr: bool, // 是否启用OCR
}

// 安全实现 Send 和 Sync：虽然 LLMClient trait object 可能包含非 Send/Sync 类型（如 LocalLLMClient），
// 但所有实现都通过 Arc<Mutex<>> 保护，可以安全跨线程传递和共享
unsafe impl Send for DocumentProcessor {}
unsafe impl Sync for DocumentProcessor {}

impl DocumentProcessor {
    /// 使用远程 DeepSeek API 创建 DocumentProcessor
    pub fn new(
        deepseek: DeepSeekClient,
        pyresparser_url: String,
        paddleocr_url: String,
        mineru_url: Option<String>,
        enable_pyresparser: bool,
        enable_ocr: bool,
        enable_mineru: bool,
    ) -> Self {
        Self::new_with_llm(
            Arc::new(deepseek) as Arc<dyn LLMClient>,
            pyresparser_url,
            paddleocr_url,
            mineru_url,
            enable_pyresparser,
            enable_ocr,
            enable_mineru,
        )
    }

    /// 使用本地 LLM 创建 DocumentProcessor
    pub fn new_with_local_llm(
        local_llm: LocalLLMClient,
        pyresparser_url: String,
        paddleocr_url: String,
        mineru_url: Option<String>,
        enable_pyresparser: bool,
        enable_ocr: bool,
        enable_mineru: bool,
    ) -> Self {
        Self::new_with_llm(
            Arc::new(local_llm) as Arc<dyn LLMClient>,
            pyresparser_url,
            paddleocr_url,
            mineru_url,
            enable_pyresparser,
            enable_ocr,
            enable_mineru,
        )
    }

    /// 使用统一的 LLM 客户端创建 DocumentProcessor（内部方法）
    fn new_with_llm(
        llm_client: Arc<dyn LLMClient>,
        pyresparser_url: String,
        paddleocr_url: String,
        mineru_url: Option<String>,
        enable_pyresparser: bool,
        enable_ocr: bool,
        enable_mineru: bool,
    ) -> Self {
        let temp_dir = PathBuf::from("./temp_uploads");
        std::fs::create_dir_all(&temp_dir).ok();

        // 初始化路由器
        let mut router = DocumentRouter::new();
        router.enable_pyresparser = enable_pyresparser;
        router.enable_mineru = enable_mineru;
        router.enable_deepseek = true; // DeepSeek始终启用作为兜底

        // 初始化分类器和JD解析器
        let classifier = Arc::new(DocumentClassifier::new());
        let jd_parser = Arc::new(JDParser::new());

        // 初始化客户端
        let pyresparser = Arc::new(PyResParserClient::new(pyresparser_url));
        let paddleocr = Arc::new(PaddleOCRClient::new(paddleocr_url));
        let mineru = mineru_url.map(|url| Arc::new(MinerUClient::new(Some(url))));

        Self {
            _router: Arc::new(router),
            classifier,
            jd_parser,
            _pyresparser: pyresparser,
            paddleocr,
            _mineru: mineru,
            llm_client,
            temp_dir,
            _enable_intelligent_routing: enable_pyresparser || enable_mineru,
            enable_ocr,
        }
    }

    /// 完整的文档处理流程
    pub async fn process_document(
        &self,
        file_data: Bytes,
        file_name: &str,
    ) -> Result<ProcessedDocument, anyhow::Error> {
        let start = std::time::Instant::now();

        // 1. 保存临时文件
        let temp_file = self.temp_dir.join(file_name);
        fs::write(&temp_file, &file_data).await?;

        // 2. Rust原生解析（PDF/DOCX/Excel/HTML/Markdown等）
        tracing::info!("📄 开始处理文档: {}", file_name);
        let mut parsed = PdfParser::parse(&temp_file).await?;
        let parse_duration = start.elapsed();
        tracing::info!(
            "⏱️ 文档解析耗时: {:?}, 提取文本: {} 字符",
            parse_duration,
            parsed.text.len()
        );

        // 3. 【关键优化】智能OCR决策：检查文本量和质量
        let text_len = parsed.text.trim().len();
        let mut classification_text = parsed.text.clone(); // 用于分类的文本
        let mut should_ocr = false;
        let mut ocr_used = false;
        let mut ocr_text_length = None;

        // 策略1: 文本过少，明显的扫描件
        if text_len < 100 {
            should_ocr = true;
            tracing::warn!("⚠️  文本过少({} 字符)，疑似扫描件，需要OCR", text_len);
        }
        // 策略2: 文本很多但都是乱码（快速检测）
        else if text_len > 1000 {
            // 快速质量检测：前500字符中是否包含任何中英文关键词
            let sample_text = parsed
                .text
                .chars()
                .take(500)
                .collect::<String>()
                .to_lowercase();
            tracing::info!("🔍 文本质量检测: 采样前500字符，文件: {}", file_name);

            let has_keywords = sample_text.contains("招聘")
                || sample_text.contains("岗位")
                || sample_text.contains("职位")
                || sample_text.contains("简历")
                || sample_text.contains("工作")
                || sample_text.contains("job")
                || sample_text.contains("resume")
                || sample_text.contains("position");

            if !has_keywords {
                should_ocr = true;
                tracing::warn!(
                    "⚠️  文本量大({} 字符)但无有效关键词，可能是乱码，需要OCR",
                    text_len
                );
                tracing::warn!(
                    "   采样文本前100字: {}",
                    &sample_text.chars().take(100).collect::<String>()
                );
            } else {
                tracing::info!("✅ 文本质量检测通过: 发现关键词，无需OCR");
            }
        }

        tracing::info!(
            "🔧 OCR决策: should_ocr={}, text_len={}, enable_ocr={}",
            should_ocr,
            text_len,
            self.enable_ocr
        );

        // 执行快速OCR（如果需要）
        if should_ocr && self.enable_ocr {
            tracing::info!("🔄 开始快速OCR识别关键信息...");

            match self
                .paddleocr
                .quick_ocr_for_classification(&file_data, file_name)
                .await
            {
                Ok(quick_ocr_text) => {
                    let ocr_len = quick_ocr_text.trim().len();
                    if ocr_len > 50 {
                        // 至少有一些文本
                        tracing::info!("✅ 快速OCR成功: {} 字符（用于分类）", ocr_len);
                        classification_text = quick_ocr_text.clone(); // 用OCR文本进行分类
                        parsed.text = quick_ocr_text; // 保存OCR文本
                        ocr_used = true;
                        ocr_text_length = Some(parsed.text.len() as i32);
                    } else {
                        tracing::warn!("⚠️  快速OCR文本量不足，保持原文本");
                    }
                }
                Err(e) => {
                    tracing::warn!("⚠️  快速OCR失败: {}，使用原文本分类", e);
                    // 继续使用原文本（可能是乱码或空）
                }
            }
        } else if text_len >= 100 {
            tracing::info!("✅ 文本充足({} 字符)且质量良好，无需OCR", text_len);
        }

        // 清理临时文件
        fs::remove_file(&temp_file).await.ok();

        // 4. 文档智能分类（使用OCR增强后的文本）
        let classify_start = std::time::Instant::now();
        let mut classification = self.classifier.classify(&classification_text, file_name);
        let classify_duration = classify_start.elapsed();
        tracing::info!(
            "📊 文档分类结果: {:?}, 置信度: {:.2}%, 耗时: {:?}",
            classification.doc_type,
            classification.confidence * 100.0,
            classify_duration
        );

        // 5. 【二次OCR策略】如果分类置信度过低且未做过OCR，尝试OCR重试
        // 注意：文本量大更可能是乱码（扫描件不会提取出大量文本）
        tracing::info!(
            "🔧 二次OCR检查: confidence={:.2}%, should_ocr={}, enable_ocr={}",
            classification.confidence * 100.0,
            should_ocr,
            self.enable_ocr
        );

        if classification.confidence < 0.4 && !should_ocr && self.enable_ocr {
            tracing::warn!(
                "⚠️  分类置信度过低({:.2}%)，文本{}字符，可能是乱码PDF，尝试OCR重新识别...",
                classification.confidence * 100.0,
                text_len
            );

            match self
                .paddleocr
                .quick_ocr_for_classification(&file_data, file_name)
                .await
            {
                Ok(ocr_text) => {
                    if ocr_text.trim().len() > 100 {
                        tracing::info!("✅ OCR重识别成功: {} 字符，重新分类...", ocr_text.len());

                        // 使用OCR文本重新分类
                        classification = self.classifier.classify(&ocr_text, file_name);
                        tracing::info!(
                            "📊 重新分类结果: {:?}, 置信度: {:.2}%",
                            classification.doc_type,
                            classification.confidence * 100.0
                        );

                        // 如果OCR结果更好，使用OCR文本
                        if classification.confidence > 0.4 {
                            parsed.text = ocr_text;
                            tracing::info!("✅ OCR文本质量更好，已替换原文本");
                            ocr_used = true;
                            ocr_text_length = Some(parsed.text.len() as i32);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("⚠️  OCR重识别失败: {}", e);
                }
            }
        }

        // 5. 根据文档类型选择处理方式
        let (extracted_info, quality_score, jd_info, poetry_profile) = match classification.doc_type
        {
            DocumentType::Resume => {
                // 简历文档 - 使用现有的简历解析流程
                tracing::info!("🎓 识别为简历文档，使用简历解析引擎");

                // 注意：这里使用的是已经过OCR增强的 parsed.text
                // 如果已经做过快速OCR，就不需要重复OCR了

                // LLM提取信息（支持本地/远程）
                let extraction_start = std::time::Instant::now();
                let extracted = self.llm_client.extract_resume_info(&parsed.text).await?;
                let extraction_duration = extraction_start.elapsed();
                tracing::info!("⏱️ LLM提取耗时: {:?}", extraction_duration);

                // LLM质量评分
                let score_start = std::time::Instant::now();
                let quality = self
                    .llm_client
                    .score_quality(&extracted, &parsed.text)
                    .await?;
                let score_duration = score_start.elapsed();
                tracing::info!("⏱️ LLM评分耗时: {:?}", score_duration);

                (Some(extracted), Some(quality), None, None)
            }
            DocumentType::JobDescription => {
                // JD文档 - 使用JD规则解析器 (Phase 2新增)
                tracing::info!("💼 识别为JD文档，使用JD规则解析引擎");

                // JD使用规则解析，速度快，不需要完整文本
                let jd_start = std::time::Instant::now();
                let jd = self.jd_parser.parse(&parsed.text)?;
                let jd_duration = jd_start.elapsed();
                tracing::info!("⏱️ JD规则解析耗时: {:?}", jd_duration);
                tracing::info!(
                    "📋 JD信息: 岗位={:?}, 职责数={}, 要求数={}",
                    jd.title,
                    jd.responsibilities.len(),
                    jd.requirements.len()
                );

                (None, None, Some(jd), None)
            }
            DocumentType::PoetryProfile => {
                // 诗人档案文档 - 使用DeepSeek提取诗人信息
                tracing::info!("📜 识别为诗人档案文档，使用诗人档案解析引擎");

                // LLM提取诗人档案信息（支持本地/远程）
                let extraction_start = std::time::Instant::now();
                let profile = self.llm_client.extract_poetry_profile(&parsed.text).await?;
                let extraction_duration = extraction_start.elapsed();
                tracing::info!("⏱️ LLM提取诗人档案耗时: {:?}", extraction_duration);
                tracing::info!(
                    "📜 诗人档案信息: 姓名={:?}, 朝代={:?}, 作品数={}, 地点数={}, 事件数={}",
                    profile.name,
                    profile.dynasty,
                    profile.works.len(),
                    profile.locations.len(),
                    profile.events.len()
                );

                (None, None, None, Some(profile))
            }
            DocumentType::Other => {
                // 其他文档 - 仅提取文本，不做深度解析
                tracing::info!("📄 识别为其他类型文档，跳过专业解析");
                (None, None, None, None)
            }
        };

        let has_resume_info = extracted_info.is_some();
        let has_poetry_profile = poetry_profile.is_some();
        let extracted_info = extracted_info.unwrap_or_default();
        let quality_score = quality_score.unwrap_or_default();

        // 5. 传统分类（向后兼容）
        let category = if has_resume_info {
            Self::classify_document(&extracted_info, &parsed.text)
        } else if has_poetry_profile {
            DocumentCategory::Other // 诗人档案归类为Other，但会单独存储
        } else {
            DocumentCategory::Other
        };

        let extraction_confidence = if has_resume_info {
            Self::estimate_extraction_confidence(&extracted_info)
        } else if has_poetry_profile {
            // 使用诗人档案的置信度
            poetry_profile
                .as_ref()
                .map(|p| p.extraction_confidence)
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let quality_confidence = if has_resume_info {
            (quality_score.overall_score as f64 / 100.0).clamp(0.0, 1.0)
        } else if has_poetry_profile {
            // 基于诗人档案的完整性计算质量置信度
            poetry_profile
                .as_ref()
                .map(|p| Self::estimate_poetry_confidence(p))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let final_ocr_text_length = if ocr_used {
            ocr_text_length.or(Some(parsed.text.len() as i32))
        } else {
            None
        };
        let ocr_confidence =
            final_ocr_text_length.map(|len| Self::estimate_ocr_confidence(len.max(0) as usize));
        let jd_confidence = jd_info.as_ref().map(|jd| Self::estimate_jd_confidence(jd));

        let total_duration = start.elapsed();
        tracing::info!(
            "✅ 文档处理完成: {}, 总耗时: {:?}",
            file_name,
            total_duration
        );

        Ok(ProcessedDocument {
            text: parsed.text,
            page_count: parsed.page_count,
            extracted_info,
            quality_score,
            category,
            classification: Some(classification),
            jd_info,
            poetry_profile,
            ocr_confidence,
            ocr_text_length: final_ocr_text_length,
            quality_confidence,
            extraction_confidence,
            jd_confidence,
            processing_time_ms: total_duration.as_millis() as i64,
        })
    }

    /// 批量处理（控制并发）
    pub async fn process_batch(&self, files: Vec<(String, Bytes)>) -> Vec<ProcessResult> {
        let mut results = Vec::new();
        let total = files.len();

        tracing::info!("📦 开始批量处理 {} 个文件", total);

        // 控制并发数为3（避免服务过载）
        let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
        let mut tasks = Vec::new();

        for (i, (name, data)) in files.into_iter().enumerate() {
            let processor = self.clone();
            let permit = semaphore.clone().acquire_owned().await.unwrap();

            let task = tokio::spawn(async move {
                tracing::info!("[{}/{}] 处理文档: {}", i + 1, total, name);

                let result = match processor.process_document(data, &name).await {
                    Ok(processed) => ProcessResult {
                        file_name: name.clone(),
                        status: "success".to_string(),
                        processed: Some(processed),
                        error: None,
                    },
                    Err(e) => ProcessResult {
                        file_name: name.clone(),
                        status: "failed".to_string(),
                        processed: None,
                        error: Some(e.to_string()),
                    },
                };

                drop(permit);
                result
            });

            tasks.push(task);
        }

        for task in tasks {
            if let Ok(result) = task.await {
                results.push(result);
            }
        }

        let success_count = results.iter().filter(|r| r.status == "success").count();
        tracing::info!("✅ 批量处理完成: {}/{} 成功", success_count, total);

        results
    }

    /// 文档分类
    fn classify_document(info: &ExtractedResumeInfo, text: &str) -> DocumentCategory {
        let text_lower = text.to_lowercase();

        // 基于提取的信息判断
        let has_work = !info.work_experience.is_empty();
        let has_edu = !info.education.is_empty();
        let has_skills = info.skills.len() > 3;
        let has_contact = info.phone.is_some() || info.email.is_some();

        if has_work && has_edu && has_skills && has_contact {
            DocumentCategory::Resume
        } else if text_lower.contains("cover letter") || text_lower.contains("求职信") {
            DocumentCategory::CoverLetter
        } else if text_lower.contains("certificate") || text_lower.contains("证书") {
            DocumentCategory::Certificate
        } else {
            DocumentCategory::Other
        }
    }

    fn estimate_ocr_confidence(text_length: usize) -> f64 {
        match text_length {
            0 => 0.0,
            1..=100 => 0.35,
            101..=300 => 0.55,
            301..=700 => 0.7,
            701..=1500 => 0.85,
            _ => 0.95,
        }
    }

    fn estimate_extraction_confidence(info: &ExtractedResumeInfo) -> f64 {
        let mut score = 0.0;
        let mut total = 5.0;

        if info.name.is_some() {
            score += 1.0;
        }
        if info.phone.is_some() || info.email.is_some() {
            score += 1.0;
        }
        if !info.education.is_empty() {
            score += 1.0;
        }
        if !info.work_experience.is_empty() {
            score += 1.0;
        }
        if !info.skills.is_empty() {
            score += 1.0;
        }
        if info.summary.is_some() {
            total += 0.5;
            score += 0.5;
        }

        if total <= 0.0 {
            0.0
        } else {
            let ratio: f64 = score / total;
            ratio.clamp(0.0_f64, 1.0_f64)
        }
    }

    fn estimate_jd_confidence(jd: &JobDescription) -> f64 {
        let mut score = 0.0;
        let mut total = 5.0;

        if jd.title.is_some() {
            score += 1.0;
        }
        if !jd.responsibilities.is_empty() {
            score += 1.0;
        }
        if !jd.requirements.is_empty() {
            score += 1.0;
        }
        if jd.location.is_some() {
            score += 0.5;
        }
        if jd.salary_range.is_some() {
            score += 0.5;
        }
        if jd.education.is_some() {
            total += 0.5;
            score += 0.5;
        }
        if !jd.skills.is_empty() {
            total += 0.5;
            score += 0.5;
        }

        if total <= 0.0 {
            0.0
        } else {
            let ratio: f64 = score / total;
            ratio.clamp(0.0_f64, 1.0_f64)
        }
    }

    fn estimate_poetry_confidence(profile: &ExtractedPoetryProfile) -> f64 {
        let mut score = 0.0;
        let mut total = 8.0;

        // 基本信息
        if profile.name.is_some() {
            score += 1.0;
        }
        if profile.dynasty.is_some() {
            score += 1.0;
        }
        if profile.biography.is_some() {
            score += 1.0;
        }
        // 时间信息
        if profile.birth_year.is_some() || profile.death_year.is_some() {
            score += 1.0;
        }
        // 地点信息
        if profile.birthplace.is_some() || !profile.locations.is_empty() {
            score += 1.0;
        }
        // 作品信息
        if !profile.works.is_empty() {
            score += 1.0;
            total += 0.5; // 作品是重要指标
        }
        // 事件信息
        if !profile.events.is_empty() {
            score += 1.0;
            total += 0.5;
        }
        // 名句
        if !profile.famous_quotes.is_empty() {
            score += 0.5;
            total += 0.5;
        }
        // 官职/风格
        if !profile.official_positions.is_empty() || profile.literary_style.is_some() {
            score += 0.5;
            total += 0.5;
        }

        if total <= 0.0 {
            0.0
        } else {
            let ratio: f64 = score / total;
            // 结合DeepSeek返回的置信度
            let combined =
                (ratio * 0.6 + profile.extraction_confidence * 0.4).clamp(0.0_f64, 1.0_f64);
            combined
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedDocument {
    pub text: String,
    pub page_count: u32,
    pub extracted_info: ExtractedResumeInfo,
    pub quality_score: QualityScore,
    pub category: DocumentCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<ClassificationResult>, // Phase 2新增
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_info: Option<JobDescription>, // Phase 2新增
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poetry_profile: Option<ExtractedPoetryProfile>, // 诗人档案
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_text_length: Option<i32>,
    pub quality_confidence: f64,
    pub extraction_confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_confidence: Option<f64>,
    pub processing_time_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DocumentCategory {
    Resume,      // 简历
    CoverLetter, // 求职信
    Certificate, // 证书
    Other,       // 其他
}

impl std::fmt::Display for DocumentCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentCategory::Resume => write!(f, "resume"),
            DocumentCategory::CoverLetter => write!(f, "cover_letter"),
            DocumentCategory::Certificate => write!(f, "certificate"),
            DocumentCategory::Other => write!(f, "other"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessResult {
    pub file_name: String,
    pub status: String,
    pub processed: Option<ProcessedDocument>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
    format: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct OllamaResponse {
    response: String,
    done: bool,
}
