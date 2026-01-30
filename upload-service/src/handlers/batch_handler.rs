use axum::extract::Multipart;
use axum::{extract::State, Extension};

use crate::{
    dto::BatchUploadResult,
    services::UploadService,
    utils::{
        error::{AppError, Result},
        jwt::Claims,
        response::ApiResponse,
    },
    AppState,
};

/// 批量上传并解析
pub async fn batch_upload_and_parse(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    mut multipart: Multipart,
) -> Result<ApiResponse<BatchUploadResult>> {
    tracing::info!("📦 收到批量上传请求，用户: {}", claims.sub);

    let mut files = Vec::new();

    // 收集所有文件
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("解析上传数据失败: {}", e)))?
    {
        let file_name = field
            .file_name()
            .ok_or_else(|| AppError::BadRequest("缺少文件名".to_string()))?
            .to_string();

        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("读取文件数据失败: {}", e)))?;

        files.push((file_name, data, content_type));
    }

    if files.is_empty() {
        return Err(AppError::BadRequest("未找到上传文件".to_string()));
    }

    tracing::info!("📥 收到 {} 个文件", files.len());

    // 批量处理
    let result = UploadService::batch_upload_with_processing(
        &state.db,
        &*state.storage,
        &*state.processor,
        claims.sub,
        files,
    )
    .await?;

    let success_count = result.success;
    let total_count = result.total;

    Ok(ApiResponse::success_with_message(
        result,
        format!("批量上传完成，成功: {}/{}", success_count, total_count),
    ))
}

/// 获取解析结果
pub async fn get_parsed_result(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(upload_id): axum::extract::Path<String>,
) -> Result<ApiResponse<crate::dto::ParsedDocumentDto>> {
    let result = UploadService::get_parsed_result(&state.db, &upload_id, &claims.sub).await?;
    Ok(ApiResponse::success(result))
}

/// 获取提取的简历信息
pub async fn get_resume_info(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(upload_id): axum::extract::Path<String>,
) -> Result<ApiResponse<crate::dto::ResumeInfoDto>> {
    let result = UploadService::get_resume_info(&state.db, &upload_id, &claims.sub).await?;
    Ok(ApiResponse::success(result))
}
