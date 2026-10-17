mod batch_handler;
mod optimize_handler;
// mod batch_upload_handler; // 暂时注释 - 编译错误

use axum::{
    extract::{Multipart, Path, State},
    http::HeaderMap,
    Extension,
};

use crate::{
    dto::{
        ExtractionDigest, QualityDigest, UploadDetail, UploadListItem, UploadRecord,
        UploadRecordWithMemory,
    },
    services::{memory_prompt, UploadService},
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
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<ApiResponse<UploadRecordWithMemory>> {
    use crate::dto::ClassificationDto;

    // ── A/B 测量面开关（默认关，`UPLOAD_EXTRACTION_DIGEST=false`）──
    //
    // 开启时：① 响应体多 `extraction_digest`；② 允许请求头 `X-Memory-Mode: off`
    // 单请求跳过记忆。用请求头而不是「重启换配置」，是因为两组必须共享同一个
    // 进程和同一份预热的 Ollama —— 否则测到的是冷启动差异，不是记忆的收益。
    let ab_enabled = state.config.upload_extraction_digest;
    let ab_memory_off = ab_enabled
        && headers
            .get("x-memory-mode")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().eq_ignore_ascii_case("off"))
            .unwrap_or(false);

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

        // 【钩子 1】优化前 recall：该用户历史简历版本 / 历次反馈 / 目标岗位
        //
        // 失败语义（已拍定）：**跳过记忆继续**，不阻断主链路。用户看不到服务端
        // 日志，所以跳过原因必须进响应体（memory_skip_reason）。
        // 超时上限由 MCP_RECALL_TIMEOUT_MS 控制（默认 2s，真机标定：最坏 ≈0.7s），
        // 保证记忆层不会把 /upload 拖得更久。
        let recall_query = format!("{} 简历 优化 历史反馈 目标岗位", file_name);
        let mut memory_skip_reason: Option<String> = None;
        let mut memory_used = true;
        // recall 结果渲染成的注入块。开关关闭 / 未命中 ⇒ None ⇒ prompt 与
        // 「没有记忆层」逐字节一致（A/B 对照组因此才有意义）。
        let mut memory_block: Option<String> = None;
        if ab_memory_off {
            // 对照组：跳过记忆，其余流程完全一致（同一进程、同一份预热模型）
            memory_used = false;
            memory_skip_reason =
                Some("A/B 对照组：X-Memory-Mode: off（仅测量面开启时生效）".to_string());
            tracing::info!("🧪 A/B 对照组：本请求跳过记忆（recall / remember 均不调）");
        } else {
            match state.memory.recall(&recall_query, Some("episodic"), 5).await {
                Ok(hits) => {
                    tracing::info!("🧠 记忆命中 {} 条（query={}）", hits.len(), recall_query);
                    if state.config.memory_prompt_injection {
                        let lines = memory_prompt::prepare_lines(
                            &hits,
                            state.config.memory_inject_max_hits,
                            state.config.memory_inject_hit_chars,
                        );
                        memory_block = memory_prompt::render_block(&lines);
                        tracing::info!(
                            "🧠 记忆注入: {}（可用 {} 条 / {} 字符）",
                            if memory_block.is_some() {
                                "已注入"
                            } else {
                                "无可用条目"
                            },
                            lines.len(),
                            memory_block
                                .as_ref()
                                .map(|b| b.chars().count())
                                .unwrap_or(0)
                        );
                    } else {
                        tracing::debug!("🧠 记忆注入关闭（MEMORY_PROMPT_INJECTION=false）");
                    }
                }
                Err(reason) => {
                    tracing::warn!("⚠️  记忆层跳过（recall）: {}", reason);
                    memory_used = false;
                    memory_skip_reason = Some(reason);
                }
            }
        }

        // 2. 同步执行文档处理（获取分类结果）
        let processed = state
            .processor
            .process_document(data_clone, &file_name, memory_block.as_deref())
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

        // 【钩子 2】优化后 remember：把这次提取的结果写进终端本地记忆
        //
        // 只在 recall 成功时才写（记忆层已坏就不必再加一次等待）。
        // 内容刻意不含手机/邮箱等联系方式 —— 那些已在上传库里；记忆体只留
        // 「可复用的画像」，metadata 标 privacy_tier=L0_PII 让合规层识别。
        if memory_used {
            let skills = processed.extracted_info.skills.iter().take(10).cloned().collect::<Vec<_>>();
            let content = format!(
                "上传简历「{}」（分类={:?}）：整体质量 {}/100（完整度 {}，清晰度 {}，专业度 {}）；\
                 教育 {} 项、工作 {} 项；技能：{}",
                file_name,
                processed.category,
                processed.quality_score.overall_score,
                processed.quality_score.completeness,
                processed.quality_score.clarity,
                processed.quality_score.professionalism,
                processed.extracted_info.education.len(),
                processed.extracted_info.work_experience.len(),
                if skills.is_empty() { "（未识别）".to_string() } else { skills.join("、") },
            );

            let metadata = serde_json::json!({
                "source": "zervi-rust/upload-service",
                "upload_id": upload_record.id,
                "file_name": file_name,
                // 跨境合规保留键：简历属个人信息，出网时必须过 M3 判定
                "privacy_tier": "L0_PII",
                "redacted": true,
            });

            match state
                .memory
                .remember(&content, Some("episodic"), Some(metadata), Some(0.6))
                .await
            {
                Ok(id) => tracing::info!("🧠 记忆已写入: {}", id),
                Err(reason) => {
                    tracing::warn!("⚠️  记忆层跳过（remember）: {}", reason);
                    memory_used = false;
                    memory_skip_reason = Some(reason);
                }
            }
        }

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

        // 【A/B 测量面】只在 UPLOAD_EXTRACTION_DIGEST=true 时构造，默认不出现。
        // 不含姓名/手机/邮箱：测量面不应扩大 PII 暴露。
        let extraction_digest = if ab_enabled {
            Some(ExtractionDigest {
                memory_used,
                memory_skip_reason: memory_skip_reason.clone(),
                // 「注入块非空」才是真注入。开关关着 / 命中 0 条 ⇒ false，
                // 此时 prompt 与对照组逐字节一致，Δ 必须为 0。
                memory_injected: memory_block.is_some(),
                memory_injected_chars: memory_block
                    .as_ref()
                    .map(|b| b.chars().count())
                    .unwrap_or(0),
                doc_type: processed
                    .classification
                    .as_ref()
                    .map(|c| format!("{:?}", c.doc_type))
                    .unwrap_or_else(|| "unknown".to_string()),
                category: format!("{:?}", processed.category),
                classification_confidence: processed
                    .classification
                    .as_ref()
                    .map(|c| c.confidence)
                    .unwrap_or(0.0),
                extraction_duration_ms: processed.extraction_duration_ms,
                scoring_duration_ms: processed.scoring_duration_ms,
                processing_time_ms: processed.processing_time_ms,
                quality: QualityDigest {
                    overall_score: processed.quality_score.overall_score,
                    completeness: processed.quality_score.completeness,
                    clarity: processed.quality_score.clarity,
                    professionalism: processed.quality_score.professionalism,
                },
                skills: processed
                    .extracted_info
                    .skills
                    .iter()
                    .take(10)
                    .cloned()
                    .collect(),
                skills_count: processed.extracted_info.skills.len(),
                education_count: processed.extracted_info.education.len(),
                work_count: processed.extracted_info.work_experience.len(),
                extraction_confidence: processed.extraction_confidence,
                quality_confidence: processed.quality_confidence,
            })
        } else {
            None
        };

        return Ok(ApiResponse::success_with_message(
            UploadRecordWithMemory {
                record,
                memory_used,
                memory_skip_reason,
                extraction_digest,
            },
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
