use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;

use crate::{
    services::{
        ats_optimizer::ATSOptimizer,
        deepseek_client::{DeepSeekClient, ExtractedResumeInfo},
        document_generator::DocumentGenerator,
        resume_optimizer::{OptimizationConfig, ResumeOptimizer},
    },
    utils::{
        error::{AppError, Result},
        jwt::Claims,
        response::ApiResponse,
    },
    AppState,
};

/// ATS分析请求
#[derive(Debug, Deserialize)]
pub struct ATSAnalysisRequest {
    pub resume_text: String,
    pub job_description: Option<String>,
}

/// 智能优化请求
#[derive(Debug, Deserialize)]
pub struct OptimizeRequest {
    pub resume_info: ExtractedResumeInfo,
    pub ats_score: i32,
    pub target_industry: Option<String>,
    pub target_position: Option<String>,
    pub experience_level: Option<String>,
}

/// 生成Word请求
#[derive(Debug, Deserialize)]
pub struct GenerateWordRequest {
    pub optimized_resume: serde_json::Value,
    pub ats_analysis: serde_json::Value,
    pub original_name: String,
}

/// ATS分析
pub async fn analyze_ats(
    State(_state): State<AppState>,
    Extension(_claims): Extension<Claims>,
    Json(req): Json<ATSAnalysisRequest>,
) -> Result<ApiResponse<serde_json::Value>> {
    tracing::info!("📊 收到ATS分析请求");

    let deepseek = DeepSeekClient::new(
        std::env::var("DEEPSEEK_URL").unwrap_or_else(|_| "http://localhost:11434".to_string()),
        std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-r1:7b".to_string()),
    );

    let optimizer = ATSOptimizer::new(deepseek);

    let analysis = optimizer
        .analyze_ats_compatibility(&req.resume_text, req.job_description.as_deref())
        .await
        .map_err(|e| AppError::BadRequest(format!("ATS分析失败: {}", e)))?;

    let result = serde_json::to_value(&analysis)
        .map_err(|e| AppError::BadRequest(format!("序列化失败: {}", e)))?;

    Ok(ApiResponse::success(result))
}

/// 智能优化
pub async fn optimize_resume(
    State(_state): State<AppState>,
    Extension(_claims): Extension<Claims>,
    Json(req): Json<OptimizeRequest>,
) -> Result<ApiResponse<serde_json::Value>> {
    tracing::info!("🎯 收到智能优化请求");

    let deepseek = DeepSeekClient::new(
        std::env::var("DEEPSEEK_URL").unwrap_or_else(|_| "http://localhost:11434".to_string()),
        std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-r1:7b".to_string()),
    );

    let optimizer = ResumeOptimizer::new(deepseek);

    // 构造ATS分析结果
    let ats_analysis = crate::services::ats_optimizer::ATSAnalysis {
        ats_score: req.ats_score,
        keyword_density: 0,
        structure_score: 0,
        missing_keywords: vec![],
        keyword_suggestions: vec![],
        structure_issues: vec![],
        overall_suggestions: vec![],
    };

    let config = OptimizationConfig {
        target_industry: req.target_industry,
        target_position: req.target_position,
        experience_level: req.experience_level,
    };

    let optimized = optimizer
        .optimize_resume(&req.resume_info, &ats_analysis, config)
        .await
        .map_err(|e| AppError::BadRequest(format!("优化失败: {}", e)))?;

    let result = serde_json::to_value(&optimized)
        .map_err(|e| AppError::BadRequest(format!("序列化失败: {}", e)))?;

    Ok(ApiResponse::success(result))
}

/// 生成优化后的Word文档
pub async fn generate_optimized_word(
    State(_state): State<AppState>,
    Extension(_claims): Extension<Claims>,
    Json(req): Json<GenerateWordRequest>,
) -> Result<Response> {
    tracing::info!("📝 收到生成Word文档请求: {}", req.original_name);

    // 反序列化
    let optimized: super::super::services::resume_optimizer::OptimizedResume =
        serde_json::from_value(req.optimized_resume)
            .map_err(|e| AppError::BadRequest(format!("优化数据格式错误: {}", e)))?;

    let ats_analysis: super::super::services::ats_optimizer::ATSAnalysis =
        serde_json::from_value(req.ats_analysis)
            .map_err(|e| AppError::BadRequest(format!("ATS数据格式错误: {}", e)))?;

    // 生成文档
    let doc_bytes =
        DocumentGenerator::generate_optimized_resume(&optimized, &ats_analysis, &req.original_name)
            .await
            .map_err(|e| AppError::BadRequest(format!("生成文档失败: {}", e)))?;

    // 返回Word文档
    let file_name = format!(
        "优化后的简历_{}.docx",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );

    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{}\"", file_name),
            ),
        ],
        doc_bytes,
    )
        .into_response())
}

/// 生成ATS分析报告Word
pub async fn generate_ats_report_word(
    State(_state): State<AppState>,
    Extension(_claims): Extension<Claims>,
    Json(req): Json<serde_json::Value>,
) -> Result<Response> {
    tracing::info!("📊 收到生成ATS报告请求");

    // 反序列化
    let ats_analysis: super::super::services::ats_optimizer::ATSAnalysis =
        serde_json::from_value(req.clone())
            .map_err(|e| AppError::BadRequest(format!("ATS数据格式错误: {}", e)))?;

    let file_name = req
        .get("file_name")
        .and_then(|v| v.as_str())
        .unwrap_or("简历");

    // 生成报告
    let doc_bytes = DocumentGenerator::generate_ats_report(&ats_analysis, file_name)
        .await
        .map_err(|e| AppError::BadRequest(format!("生成报告失败: {}", e)))?;

    // 返回Word文档
    let report_name = format!(
        "ATS分析报告_{}.docx",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );

    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{}\"", report_name),
            ),
        ],
        doc_bytes,
    )
        .into_response())
}
