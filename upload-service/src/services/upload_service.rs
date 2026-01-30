use bytes::Bytes;
use chrono::Utc;
use sea_orm::*;
use std::path::Path;
use uuid::Uuid;

use crate::{
    dto::{UploadDetail, UploadListItem, UploadRecord},
    models::upload::{self, Entity as Upload, StorageType, UploadStatus},
    services::storage::LocalStorage,
    utils::error::{AppError, Result},
    utils::hash::calculate_file_hash,
};

pub struct UploadService;

impl UploadService {
    /// 单文件上传
    pub async fn upload_file(
        db: &DatabaseConnection,
        storage: &LocalStorage,
        user_id: String,
        original_name: String,
        data: Bytes,
        mime_type: String,
        upload_ip: Option<String>,
    ) -> Result<UploadRecord> {
        // 1. 验证文件类型
        Self::validate_file_type(&mime_type)?;

        // 2. 验证文件大小
        Self::validate_file_size(data.len() as u64)?;

        // 3. 计算文件Hash
        let file_hash = calculate_file_hash(&data);
        tracing::info!("📋 文件Hash: {}, 文件名: {}", file_hash, original_name);

        // 4. 检查是否已存在相同Hash的文件
        if let Some(existing) = Upload::find()
            .filter(upload::Column::FileHash.eq(&file_hash))
            .filter(upload::Column::UserId.eq(&user_id))
            .one(db)
            .await?
        {
            tracing::info!(
                "⚡ 发现重复文件，直接返回已有记录: {} (原文件: {})",
                existing.id,
                existing.original_name
            );

            // 获取访问URL
            let access_url = storage
                .get_url(&existing.file_path)
                .await
                .map_err(|e| AppError::InternalServerError(format!("获取URL失败: {}", e)))?;

            return Ok(UploadRecord {
                id: existing.id,
                original_name: existing.original_name,
                file_size: existing.file_size,
                mime_type: existing.mime_type,
                access_url,
                created_at: existing.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                classification: None,
                jd_info: None,
            });
        }

        tracing::info!("🆕 新文件，开始上传...");

        // 5. 生成存储文件名
        let stored_name = Self::generate_file_name(&original_name);

        // 6. 上传到存储
        let file_path = storage
            .upload(&stored_name, data.clone(), &mime_type)
            .await
            .map_err(|e| AppError::InternalServerError(format!("上传失败: {}", e)))?;

        // 7. 保存到数据库
        let upload = upload::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            user_id: Set(user_id),
            original_name: Set(original_name.clone()),
            stored_name: Set(stored_name),
            file_path: Set(file_path.clone()),
            file_size: Set(data.len() as i64),
            mime_type: Set(mime_type.clone()),
            storage_type: Set(StorageType::Local),
            status: Set(UploadStatus::Uploaded),
            upload_ip: Set(upload_ip),
            file_hash: Set(Some(file_hash)),
            created_at: Set(Utc::now().naive_utc()),
            updated_at: Set(Utc::now().naive_utc()),
        };

        let result = upload.insert(db).await?;

        // 6. 生成访问URL
        let access_url = storage
            .get_url(&file_path)
            .await
            .map_err(|e| AppError::InternalServerError(format!("获取URL失败: {}", e)))?;

        Ok(UploadRecord {
            id: result.id,
            original_name: result.original_name,
            file_size: result.file_size,
            mime_type: result.mime_type,
            access_url,
            created_at: result.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            classification: None,
            jd_info: None,
        })
    }

    /// 获取用户的上传列表
    pub async fn list_user_uploads(
        db: &DatabaseConnection,
        storage: &LocalStorage,
        user_id: &str,
    ) -> Result<Vec<UploadListItem>> {
        let uploads = Upload::find()
            .filter(upload::Column::UserId.eq(user_id))
            .order_by_desc(upload::Column::CreatedAt)
            .all(db)
            .await?;

        let mut result = Vec::new();
        for upload in uploads {
            let access_url = storage.get_url(&upload.file_path).await.unwrap_or_default();

            result.push(UploadListItem {
                id: upload.id,
                original_name: upload.original_name,
                file_size: upload.file_size,
                mime_type: upload.mime_type,
                storage_type: upload.storage_type.to_string(),
                status: upload.status.to_string(),
                access_url,
                created_at: upload.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            });
        }

        Ok(result)
    }

    /// 获取上传详情
    pub async fn get_upload_detail(
        db: &DatabaseConnection,
        storage: &LocalStorage,
        file_id: &str,
        user_id: &str,
    ) -> Result<UploadDetail> {
        let upload = Upload::find_by_id(file_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("文件不存在".to_string()))?;

        // 验证所有权
        if upload.user_id != user_id {
            return Err(AppError::Forbidden("无权访问此文件".to_string()));
        }

        let access_url = storage.get_url(&upload.file_path).await.unwrap_or_default();

        Ok(UploadDetail {
            id: upload.id,
            user_id: upload.user_id,
            original_name: upload.original_name,
            stored_name: upload.stored_name,
            file_path: upload.file_path,
            file_size: upload.file_size,
            mime_type: upload.mime_type,
            storage_type: upload.storage_type.to_string(),
            status: upload.status.to_string(),
            access_url,
            created_at: upload.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            updated_at: upload.updated_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    }

    /// 删除文件
    pub async fn delete_file(
        db: &DatabaseConnection,
        storage: &LocalStorage,
        file_id: &str,
        user_id: &str,
    ) -> Result<()> {
        let upload = Upload::find_by_id(file_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("文件不存在".to_string()))?;

        // 验证所有权
        if upload.user_id != user_id {
            return Err(AppError::Forbidden("无权删除此文件".to_string()));
        }

        // 删除物理文件
        storage
            .delete(&upload.file_path)
            .await
            .map_err(|e| AppError::InternalServerError(format!("删除文件失败: {}", e)))?;

        // 删除数据库记录
        upload::Entity::delete_by_id(file_id).exec(db).await?;

        Ok(())
    }

    /// 生成唯一文件名
    fn generate_file_name(original_name: &str) -> String {
        let ext = Path::new(original_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let uuid = Uuid::new_v4();
        let date = Utc::now().format("%Y%m%d");

        if ext.is_empty() {
            format!("{}/{}", date, uuid)
        } else {
            format!("{}/{}.{}", date, uuid, ext)
        }
    }

    /// 验证文件类型
    fn validate_file_type(mime_type: &str) -> Result<()> {
        const ALLOWED_TYPES: &[&str] = &[
            // 图片
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/svg+xml",
            // 文档
            "application/pdf",
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            // HTML
            "text/html",
            "application/xhtml+xml",
            // 压缩包
            "application/zip",
            "application/x-rar-compressed",
            // 文本
            "text/plain",
            "text/markdown",
            // 通用二进制（允许，稍后基于文件扩展名判断）
            "application/octet-stream",
        ];

        if !ALLOWED_TYPES.contains(&mime_type) {
            return Err(AppError::BadRequest(format!(
                "不支持的文件类型: {}",
                mime_type
            )));
        }

        Ok(())
    }

    /// 验证文件大小
    fn validate_file_size(size: u64) -> Result<()> {
        const MAX_SIZE: u64 = 100 * 1024 * 1024; // 100MB

        if size > MAX_SIZE {
            return Err(AppError::BadRequest(format!(
                "文件大小超过限制: {} bytes (最大: {} bytes)",
                size, MAX_SIZE
            )));
        }

        if size == 0 {
            return Err(AppError::BadRequest("文件不能为空".to_string()));
        }

        Ok(())
    }

    /// 批量上传并处理
    pub async fn batch_upload_with_processing(
        db: &DatabaseConnection,
        storage: &LocalStorage,
        processor: &crate::services::document_processor::DocumentProcessor,
        user_id: String,
        files: Vec<(String, Bytes, String)>,
    ) -> Result<crate::dto::BatchUploadResult> {
        use crate::dto::{BatchUploadItem, BatchUploadResult};

        let total = files.len();
        let mut results = Vec::new();
        let mut success = 0;
        let processing = total;

        tracing::info!("📦 开始批量上传 {} 个文件", total);

        for (name, data, mime) in files {
            match Self::upload_file(
                db,
                storage,
                user_id.clone(),
                name.clone(),
                data.clone(),
                mime,
                None,
            )
            .await
            {
                Ok(record) => {
                    success += 1;
                    let upload_id = record.id.clone();

                    results.push(BatchUploadItem {
                        file_name: name.clone(),
                        status: "uploaded".to_string(),
                        upload_id: Some(upload_id.clone()),
                        error: None,
                        classification: None,
                        jd_info: None,
                    });

                    // 异步处理文档
                    let db_clone = db.clone();
                    let processor_clone = processor.clone();
                    let file_name = name.clone();
                    let file_data = data.clone();
                    let upload_id_clone = upload_id.clone();

                    tokio::spawn(async move {
                        tracing::info!("🔄 开始异步处理文档: {}", file_name);

                        match Self::async_process_document(
                            &db_clone,
                            &processor_clone,
                            &upload_id_clone,
                            file_data,
                            &file_name,
                        )
                        .await
                        {
                            Ok(_) => {
                                tracing::info!("✅ 文档处理完成: {}", file_name);
                            }
                            Err(e) => {
                                tracing::error!("❌ 文档处理失败: {}, 错误: {}", file_name, e);
                            }
                        }
                    });
                }
                Err(e) => {
                    results.push(BatchUploadItem {
                        file_name: name,
                        status: "failed".to_string(),
                        upload_id: None,
                        error: Some(e.to_string()),
                        classification: None,
                        jd_info: None,
                    });
                }
            }
        }

        Ok(BatchUploadResult {
            total,
            success,
            failed: total - success,
            processing,
            results,
        })
    }

    /// 获取解析结果
    pub async fn get_parsed_result(
        db: &DatabaseConnection,
        upload_id: &str,
        user_id: &str,
    ) -> Result<crate::dto::ParsedDocumentDto> {
        use crate::dto::ParsedDocumentDto;
        use crate::models::{parsed_document, ParsedDocument, Upload};

        // 验证权限
        let upload = Upload::find_by_id(upload_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("文件不存在".to_string()))?;

        if upload.user_id != user_id {
            return Err(AppError::Forbidden("无权访问此文件".to_string()));
        }

        // 获取解析结果
        let parsed = ParsedDocument::find()
            .filter(parsed_document::Column::UploadId.eq(upload_id))
            .one(db)
            .await?
            .ok_or(AppError::NotFound(
                "解析结果不存在，可能尚未处理完成".to_string(),
            ))?;

        Ok(ParsedDocumentDto {
            id: parsed.id,
            upload_id: parsed.upload_id,
            text_content: parsed.text_content,
            page_count: parsed.page_count.unwrap_or(0),
            tables_count: parsed.tables_count.unwrap_or(0),
            parsed_at: parsed.parsed_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            classification_type: parsed.classification_type.clone(),
            classification_confidence: parsed.classification_confidence,
            ocr_confidence: parsed.ocr_confidence,
            ocr_text_length: parsed.ocr_text_length,
            jd_confidence: parsed.jd_confidence,
        })
    }

    /// 获取简历提取信息
    pub async fn get_resume_info(
        db: &DatabaseConnection,
        upload_id: &str,
        user_id: &str,
    ) -> Result<crate::dto::ResumeInfoDto> {
        use crate::dto::ResumeInfoDto;
        use crate::models::{resume_info, ResumeInfo, Upload};

        // 验证权限
        let upload = Upload::find_by_id(upload_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("文件不存在".to_string()))?;

        if upload.user_id != user_id {
            return Err(AppError::Forbidden("无权访问此文件".to_string()));
        }

        // 获取简历信息
        let info = ResumeInfo::find()
            .filter(resume_info::Column::UploadId.eq(upload_id))
            .one(db)
            .await?
            .ok_or(AppError::NotFound(
                "简历信息不存在，可能尚未提取完成".to_string(),
            ))?;

        // 解析JSON字段
        let education: Vec<serde_json::Value> = info
            .education
            .as_ref()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();

        let work_experience: Vec<serde_json::Value> = info
            .work_experience
            .as_ref()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();

        let skills: Vec<String> = info
            .skills
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(ResumeInfoDto {
            id: info.id,
            name: info.name,
            phone: info.phone,
            email: info.email,
            location: info.location,
            summary: info.summary,
            education,
            work_experience,
            skills,
            quality_score: info.quality_score.unwrap_or(0),
            completeness_score: info.completeness_score.unwrap_or(0),
            clarity_score: info.clarity_score.unwrap_or(0),
            professionalism_score: info.professionalism_score.unwrap_or(0),
            quality_confidence: info.quality_confidence.unwrap_or(0.0),
            extraction_confidence: info.extraction_confidence.unwrap_or(0.0),
            category: info.category.unwrap_or_else(|| "other".to_string()),
            extracted_at: info.extracted_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    }

    /// 异步处理文档：解析+提取+保存
    async fn async_process_document(
        db: &DatabaseConnection,
        processor: &crate::services::document_processor::DocumentProcessor,
        upload_id: &str,
        file_data: Bytes,
        file_name: &str,
    ) -> Result<()> {
        use crate::models::{parsed_document, resume_info};
        use crate::services::document_classifier::DocumentType;
        use sea_orm::*;

        // 1. 处理文档（PDF解析 + DeepSeek提取）
        let processed = processor
            .process_document(file_data, file_name)
            .await
            .map_err(|e| AppError::InternalServerError(format!("文档处理失败: {}", e)))?;

        tracing::info!(
            "📄 解析完成: {}, {} 字符, {} 页",
            file_name,
            processed.text.len(),
            processed.page_count
        );

        // 2. 保存解析文本到 parsed_documents
        let parsed_doc_id = Uuid::new_v4().to_string();
        let classification_label = processed.classification.as_ref().map(|c| match c.doc_type {
            DocumentType::Resume => "resume".to_string(),
            DocumentType::JobDescription => "job_description".to_string(),
            DocumentType::PoetryProfile => "poetry_profile".to_string(),
            DocumentType::Other => "other".to_string(),
        });
        let classification_confidence = processed.classification.as_ref().map(|c| c.confidence);

        let parsed_model = parsed_document::ActiveModel {
            id: Set(parsed_doc_id.clone()),
            upload_id: Set(upload_id.to_string()),
            text_content: Set(processed.text.clone()),
            page_count: Set(Some(processed.page_count as i32)),
            tables_count: Set(Some(0)),
            images_count: Set(Some(0)),
            parsed_at: Set(chrono::Utc::now().naive_utc()),
            parse_duration_ms: Set(Some(processed.processing_time_ms as i32)),
            classification_type: Set(classification_label),
            classification_confidence: Set(classification_confidence),
            ocr_confidence: Set(processed.ocr_confidence),
            ocr_text_length: Set(processed.ocr_text_length),
            jd_confidence: Set(processed.jd_confidence),
        };

        parsed_document::Entity::insert(parsed_model)
            .exec(db)
            .await
            .map_err(|e| AppError::DatabaseError(e))?;

        tracing::info!("✅ 解析文本已保存");

        // 3. 保存提取信息到 resume_extracted_info（复用表结构，支持简历和诗人档案）
        if let Some(ref poetry_profile) = processed.poetry_profile {
            // 保存诗人档案信息
            let poetry_json = serde_json::to_value(poetry_profile).unwrap_or_default();
            let resume_info_model = resume_info::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                parsed_doc_id: Set(parsed_doc_id),
                upload_id: Set(upload_id.to_string()),
                name: Set(poetry_profile.name.clone()),
                phone: Set(None),
                email: Set(None),
                location: Set(poetry_profile.birthplace.clone()),
                summary: Set(poetry_profile.biography.clone()),
                // 将诗人档案完整信息存储在 education 字段（JSON）
                education: Set(Some(poetry_json)),
                work_experience: Set(None),
                skills: Set(None),
                quality_score: Set(Some((processed.quality_confidence * 100.0) as i32)),
                completeness_score: Set(Some((processed.quality_confidence * 100.0) as i32)),
                clarity_score: Set(Some((processed.quality_confidence * 100.0) as i32)),
                professionalism_score: Set(Some((processed.quality_confidence * 100.0) as i32)),
                quality_confidence: Set(Some(processed.quality_confidence)),
                extraction_confidence: Set(Some(processed.extraction_confidence)),
                category: Set(Some("poetry_profile".to_string())),
                extracted_at: Set(chrono::Utc::now().naive_utc()),
                extraction_duration_ms: Set(Some(processed.processing_time_ms as i32)),
            };

            resume_info::Entity::insert(resume_info_model)
                .exec(db)
                .await
                .map_err(|e| AppError::DatabaseError(e))?;

            tracing::info!(
                "✅ 诗人档案信息已保存: 姓名={:?}, 朝代={:?}, 作品数={}",
                poetry_profile.name,
                poetry_profile.dynasty,
                poetry_profile.works.len()
            );
        } else {
            // 保存简历信息（原有逻辑）
            let info = &processed.extracted_info;
            let resume_info_model = resume_info::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                parsed_doc_id: Set(parsed_doc_id),
                upload_id: Set(upload_id.to_string()),
                name: Set(info.name.clone()),
                phone: Set(info.phone.clone()),
                email: Set(info.email.clone()),
                location: Set(info.location.clone()),
                summary: Set(info.summary.clone()),
                education: Set(serde_json::to_value(&info.education).ok()),
                work_experience: Set(serde_json::to_value(&info.work_experience).ok()),
                skills: Set(serde_json::to_value(&info.skills).ok()),
                quality_score: Set(Some(processed.quality_score.overall_score)),
                completeness_score: Set(Some(processed.quality_score.completeness)),
                clarity_score: Set(Some(processed.quality_score.clarity)),
                professionalism_score: Set(Some(processed.quality_score.professionalism)),
                quality_confidence: Set(Some(processed.quality_confidence)),
                extraction_confidence: Set(Some(processed.extraction_confidence)),
                category: Set(Some(processed.category.to_string())),
                extracted_at: Set(chrono::Utc::now().naive_utc()),
                extraction_duration_ms: Set(Some(processed.processing_time_ms as i32)),
            };

            resume_info::Entity::insert(resume_info_model)
                .exec(db)
                .await
                .map_err(|e| AppError::DatabaseError(e))?;

            tracing::info!(
                "✅ 简历信息已保存: 姓名={:?}, 技能数={}",
                info.name,
                info.skills.len()
            );
        }

        Ok(())
    }
}
