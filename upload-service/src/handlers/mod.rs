mod batch_handler;
mod optimize_handler;
// mod batch_upload_handler; // 暂时注释 - 编译错误

use axum::{
    extract::{Multipart, Path, State},
    Extension,
};

use crate::{
    dto::{UploadDetail, UploadListItem, UploadRecord},
    services::UploadService,
    utils::{
        error::{AppError, Result},
        jwt::Claims,
        response::ApiResponse,
    },
    AppState,
};

pub use batch_handler::{batch_upload_and_parse, get_parsed_result, get_resume_info};
pub use optimize_handler::{
    analyze_ats, generate_ats_report_word, generate_optimized_word, optimize_resume,
};
// pub use batch_upload_handler::{
//     batch_upload_resumes,
//     batch_upload_with_progress,
// };

/// 单文件上传（Phase 2增强：同步分类和解析）
pub async fn upload_single(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    mut multipart: Multipart,
) -> Result<ApiResponse<UploadRecord>> {
    use crate::dto::ClassificationDto;

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

        let file_size = data.len() as i64;
        let mime_type_str = content_type.clone();

        // ✅ Phase 2：同步处理文档并返回分类结果
        let data_clone = data.clone();

        tracing::info!("📤 开始上传文件: {}, 大小: {} bytes", file_name, data.len());

        // 1. 先上传文件
        let upload_record = UploadService::upload_file(
            &state.db,
            &*state.storage,
            claims.sub.clone(),
            file_name.clone(),
            data,
            content_type,
            None,
        )
        .await?;

        tracing::info!("✅ 文件上传成功: {}, 开始文档处理...", upload_record.id);

        // 2. 同步执行文档处理（获取分类结果）
        let processed = state
            .processor
            .process_document(data_clone, &file_name)
            .await
            .map_err(|e| {
                tracing::error!("❌ 文档处理失败: {}", e);
                AppError::InternalServerError(format!("文档处理失败: {}", e))
            })?;

        tracing::info!(
            "✅ 文档处理完成，classification: {:?}",
            processed.classification.is_some()
        );

        // 3. 构建带分类结果的返回
        let classification = processed
            .classification
            .as_ref()
            .map(|c| ClassificationDto {
                doc_type: format!("{:?}", c.doc_type),
                confidence: c.confidence,
                matched_features: c.matched_features.clone(),
            });

        let jd_info = processed.jd_info.as_ref().map(|jd| {
            serde_json::json!({
                "title": jd.title,
                "responsibilities": jd.responsibilities,
                "requirements": jd.requirements,
                "location": jd.location,
                "salary_range": jd.salary_range,
                "skills": jd.skills,
            })
        });

        let record = UploadRecord {
            id: upload_record.id.clone(),
            original_name: file_name.clone(),
            file_size,
            mime_type: mime_type_str,
            access_url: format!("/uploads/{}", upload_record.id),
            created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            classification,
            jd_info,
        };

        // 4. 记录分类结果
        if let Some(ref c) = record.classification {
            tracing::info!("✅ 文档分类完成: {} ({}%)", c.doc_type, c.confidence);
        }

        return Ok(ApiResponse::success_with_message(
            record,
            "上传成功并完成智能分类",
        ));
    }

    Err(AppError::BadRequest("未找到上传文件".to_string()))
}

/// 获取文件列表
pub async fn list_uploads(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<ApiResponse<Vec<UploadListItem>>> {
    let uploads = UploadService::list_user_uploads(&state.db, &*state.storage, &claims.sub).await?;

    Ok(ApiResponse::success(uploads))
}

/// 获取文件详情
pub async fn get_upload(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(file_id): Path<String>,
) -> Result<ApiResponse<UploadDetail>> {
    let detail =
        UploadService::get_upload_detail(&state.db, &*state.storage, &file_id, &claims.sub).await?;

    Ok(ApiResponse::success(detail))
}

/// 删除文件
pub async fn delete_upload(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(file_id): Path<String>,
) -> Result<ApiResponse<String>> {
    UploadService::delete_file(&state.db, &*state.storage, &file_id, &claims.sub).await?;

    Ok(ApiResponse::success_with_message(
        "删除成功".to_string(),
        "文件已删除",
    ))
}
