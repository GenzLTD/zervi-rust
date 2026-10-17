use axum::{
    extract::{Multipart, State},
    response::{IntoResponse, Response, sse::{Event, Sse}},
    Extension, Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use uuid::Uuid;

use crate::{
    utils::{
        error::AppError,
        jwt::Claims,
        response::ApiResponse,
    },
    AppState,
};

// ==================== 请求/响应结构 ====================

#[derive(Debug, Serialize)]
pub struct BatchUploadResponse {
    pub session_id: String,
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<ProcessResult>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProcessResult {
    pub success: bool,
    pub file_name: String,
    pub resume_id: Option<String>,
    pub error: Option<String>,
    pub process_time_ms: Option<u64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProgressUpdate {
    pub session_id: String,
    pub current: usize,
    pub total: usize,
    pub status: String,  // "processing", "completed", "failed"
    pub file_name: String,
    pub progress_percent: f32,
}

// ==================== 批量上传（普通POST）====================

/// 批量上传简历（无实时进度）
pub async fn batch_upload_resumes(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    mut multipart: Multipart,
) -> Result<ApiResponse<BatchUploadResponse>> {
    tracing::info!("📦 收到批量上传请求，用户: {}", claims.sub);
    
    let session_id = Uuid::new_v4().to_string();
    let user_id = claims.sub.clone();
    
    // 1. 接收所有文件
    let mut uploaded_files = Vec::new();
    
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        AppError::BadRequest(format!("文件上传失败: {}", e))
    })? {
        if let Some(file_name) = field.file_name() {
            let file_name = file_name.to_string();
            let data = field.bytes().await.map_err(|e| {
                AppError::BadRequest(format!("读取文件失败: {}", e))
            })?;
            
            // 文件大小限制检查
            if data.len() > 10 * 1024 * 1024 {
                tracing::warn!("文件 {} 超过10MB限制", file_name);
                uploaded_files.push(Err((file_name, "文件大小超过10MB限制".to_string())));
                continue;
            }
            
            // 文件类型检查
            if !is_valid_file_type(&file_name) {
                tracing::warn!("文件 {} 类型不支持", file_name);
                uploaded_files.push(Err((file_name, "不支持的文件类型".to_string())));
                continue;
            }
            
            // 保存文件
            match save_upload_file(&user_id, &data, &file_name).await {
                Ok(file_path) => {
                    uploaded_files.push(Ok((file_name, file_path, data.len())));
                }
                Err(e) => {
                    uploaded_files.push(Err((file_name, format!("保存失败: {}", e))));
                }
            }
        }
    }
    
    let total_files = uploaded_files.len();
    
    if total_files == 0 {
        return Err(AppError::BadRequest("没有收到任何文件".to_string()));
    }
    
    // ⭐ 使用配置项，而不是硬编码
    if total_files > state.config.max_batch_size {
        return Err(AppError::BadRequest(
            format!("一次最多上传{}份简历", state.config.max_batch_size)
        ));
    }
    
    tracing::info!("📥 收到 {} 个文件，开始批量处理（限制: {}）", total_files, state.config.max_batch_size);
    
    // 2. 创建批量上传会话
    create_batch_session(&state, &session_id, &user_id, total_files).await?;
    
    // 3. 并发处理所有简历（使用Semaphore限流）
    let mut handles = Vec::new();
    
    for (index, file_result) in uploaded_files.into_iter().enumerate() {
        let state = state.clone();
        let session_id = session_id.clone();
        let user_id = user_id.clone();
        let semaphore = Arc::clone(&state.upload_semaphore);  // ✅ 并发控制
        
        let handle = tokio::spawn(async move {
            // ✅ 获取信号量许可（限制并发数）
            let _permit = semaphore.acquire().await.expect("Semaphore closed");
            match file_result {
                Ok((file_name, file_path, file_size)) => {
                    process_single_resume(
                        state,
                        session_id,
                        user_id,
                        file_name,
                        file_path,
                        file_size,
                        index,
                    ).await
                }
                Err((file_name, error)) => {
                    ProcessResult {
                        success: false,
                        file_name,
                        resume_id: None,
                        error: Some(error),
                        process_time_ms: None,
                    }
                }
            }
        });
        
        handles.push(handle);
    }
    
    // 4. 等待所有任务完成
    let mut results = Vec::new();
    let mut succeeded = 0;
    let mut failed = 0;
    
    for handle in handles {
        match handle.await {
            Ok(result) => {
                if result.success {
                    succeeded += 1;
                } else {
                    failed += 1;
                }
                results.push(result);
            }
            Err(e) => {
                failed += 1;
                results.push(ProcessResult {
                    success: false,
                    file_name: "unknown".to_string(),
                    resume_id: None,
                    error: Some(format!("任务失败: {}", e)),
                    process_time_ms: None,
                });
            }
        }
    }
    
    // 5. 更新会话状态
    update_batch_session(&state, &session_id, succeeded, failed).await?;
    
    tracing::info!(
        "✅ 批量处理完成！总计: {}, 成功: {}, 失败: {}",
        total_files,
        succeeded,
        failed
    );
    
    Ok(ApiResponse::success(BatchUploadResponse {
        session_id,
        total: total_files,
        succeeded,
        failed,
        results,
    }))
}

// ==================== 批量上传（SSE实时进度）====================

/// 批量上传简历（带实时进度推送）
pub async fn batch_upload_with_progress(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    mut multipart: Multipart,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel(100);
    let session_id = Uuid::new_v4().to_string();
    let user_id = claims.sub.clone();
    
    tokio::spawn(async move {
        // 1. 接收文件
        let mut uploaded_files = Vec::new();
        
        while let Some(field) = multipart.next_field().await.ok().flatten() {
            if let Some(file_name) = field.file_name() {
                let file_name = file_name.to_string();
                let data = match field.bytes().await {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx.send(create_error_event(&session_id, &file_name, &e.to_string())).await;
                        continue;
                    }
                };
                
                if data.len() > 10 * 1024 * 1024 {
                    let _ = tx.send(create_error_event(&session_id, &file_name, "文件超过10MB")).await;
                    continue;
                }
                
                match save_upload_file(&user_id, &data, &file_name).await {
                    Ok(file_path) => {
                        uploaded_files.push((file_name, file_path, data.len()));
                    }
                    Err(e) => {
                        let _ = tx.send(create_error_event(&session_id, &file_name, &e.to_string())).await;
                    }
                }
            }
        }
        
        let total_files = uploaded_files.len();
        
        if total_files == 0 {
            let _ = tx.send(create_error_event(&session_id, "unknown", "没有收到任何文件")).await;
            return;
        }
        
        // 2. 创建会话
        if let Err(e) = create_batch_session(&state, &session_id, &user_id, total_files).await {
            let _ = tx.send(create_error_event(&session_id, "session", &e.to_string())).await;
            return;
        }
        
        // 3. 并发处理
        let mut handles = Vec::new();
        
        for (index, (file_name, file_path, file_size)) in uploaded_files.into_iter().enumerate() {
            let state = state.clone();
            let session_id = session_id.clone();
            let user_id = user_id.clone();
            let tx = tx.clone();
            let total = total_files;
            
            let handle = tokio::spawn(async move {
                // 发送开始处理事件
                let _ = tx.send(create_progress_event(
                    &session_id,
                    index + 1,
                    total,
                    "processing",
                    &file_name,
                )).await;
                
                // 处理文件
                let result = process_single_resume(
                    state.clone(),
                    session_id.clone(),
                    user_id,
                    file_name.clone(),
                    file_path,
                    file_size,
                    index,
                ).await;
                
                // 发送完成事件
                let status = if result.success { "completed" } else { "failed" };
                let _ = tx.send(create_progress_event(
                    &session_id,
                    index + 1,
                    total,
                    status,
                    &file_name,
                )).await;
                
                result
            });
            
            handles.push(handle);
        }
        
        // 4. 等待完成并统计
        let mut succeeded = 0;
        let mut failed = 0;
        
        for handle in handles {
            if let Ok(result) = handle.await {
                if result.success {
                    succeeded += 1;
                } else {
                    failed += 1;
                }
            } else {
                failed += 1;
            }
        }
        
        // 5. 更新会话
        let _ = update_batch_session(&state, &session_id, succeeded, failed).await;
        
        // 6. 发送最终完成事件
        let _ = tx.send(create_complete_event(&session_id, total_files, succeeded, failed)).await;
    });
    
    Sse::new(ReceiverStream::new(rx))
}

// ==================== 辅助函数 ====================

/// 处理单个简历
async fn process_single_resume(
    state: AppState,
    session_id: String,
    user_id: String,
    file_name: String,
    file_path: PathBuf,
    file_size: usize,
    _index: usize,
) -> ProcessResult {
    let start_time = std::time::Instant::now();
    let resume_id = Uuid::new_v4().to_string();
    
    // 创建批量上传详情记录
    if let Err(e) = create_batch_detail(&state, &session_id, &resume_id, &file_name, file_size).await {
        return ProcessResult {
            success: false,
            file_name,
            resume_id: None,
            error: Some(format!("创建记录失败: {}", e)),
            process_time_ms: None,
        };
    }
    
    // 1. 文件已保存，标记为success（简化版本，不进行解析）
    let parse_status = "pending"; // 后续可以异步解析
    
    // 2. AI提取信息
    // 走 config 的单一事实源：config 已统一读 env 并打启动日志（默认 qwen2.5:7b）。
    // 这里不再二次读 env —— 否则默认值会再次分叉（历史问题：4 处各自兜底，
    // 终端只装 qwen 时会去打一个不存在的模型名）。
    let deepseek = DeepSeekClient::new(
        state.config.deepseek_url.clone(),
        state.config.deepseek_model.clone(),
    );
    
    let resume_info = match deepseek.extract_resume_info(&text).await {
        Ok(info) => info,
        Err(e) => {
            let _ = update_batch_detail_status(&state, &resume_id, "failed", Some(&e.to_string())).await;
            return ProcessResult {
                success: false,
                file_name,
                resume_id: Some(resume_id),
                error: Some(format!("AI提取失败: {}", e)),
                process_time_ms: Some(start_time.elapsed().as_millis() as u64),
            };
        }
    };
    
    // 3. 保存到数据库
    if let Err(e) = save_resume_to_db(
        &state,
        &resume_id,
        &user_id,
        &file_name,
        file_path.to_str().unwrap(),
        file_size,
        &resume_info,
    ).await {
        let _ = update_batch_detail_status(&state, &resume_id, "failed", Some(&e.to_string())).await;
        return ProcessResult {
            success: false,
            file_name,
            resume_id: Some(resume_id),
            error: Some(format!("保存失败: {}", e)),
            process_time_ms: Some(start_time.elapsed().as_millis() as u64),
        };
    }
    
    let process_time = start_time.elapsed().as_millis() as u64;
    
    // 4. 更新详情状态
    let _ = update_batch_detail_completed(&state, &resume_id, process_time).await;
    
    ProcessResult {
        success: true,
        file_name,
        resume_id: Some(resume_id),
        error: None,
        process_time_ms: Some(process_time),
    }
}

/// 检查文件类型
fn is_valid_file_type(file_name: &str) -> bool {
    file_name.ends_with(".pdf") || 
    file_name.ends_with(".docx") || 
    file_name.ends_with(".txt")
}

/// 保存上传的文件
async fn save_upload_file(
    user_id: &str,
    data: &[u8],
    file_name: &str,
) -> Result<PathBuf> {
    use tokio::fs;
    
    let upload_dir = PathBuf::from("uploads").join(user_id);
    fs::create_dir_all(&upload_dir).await?;
    
    let file_path = upload_dir.join(file_name);
    fs::write(&file_path, data).await?;
    
    Ok(file_path)
}

// ==================== SSE事件创建函数 ====================

fn create_progress_event(
    session_id: &str,
    current: usize,
    total: usize,
    status: &str,
    file_name: &str,
) -> Result<Event, Infallible> {
    let update = ProgressUpdate {
        session_id: session_id.to_string(),
        current,
        total,
        status: status.to_string(),
        file_name: file_name.to_string(),
        progress_percent: (current as f32 / total as f32) * 100.0,
    };
    
    Ok(Event::default().json_data(update).unwrap())
}

fn create_error_event(session_id: &str, file_name: &str, error: &str) -> Result<Event, Infallible> {
    let update = ProgressUpdate {
        session_id: session_id.to_string(),
        current: 0,
        total: 0,
        status: "error".to_string(),
        file_name: file_name.to_string(),
        progress_percent: 0.0,
    };
    
    Ok(Event::default().json_data(update).unwrap())
}

fn create_complete_event(
    session_id: &str,
    total: usize,
    succeeded: usize,
    failed: usize,
) -> Result<Event, Infallible> {
    #[derive(Serialize)]
    struct CompleteEvent {
        session_id: String,
        status: String,
        total: usize,
        succeeded: usize,
        failed: usize,
    }
    
    let event = CompleteEvent {
        session_id: session_id.to_string(),
        status: "complete".to_string(),
        total,
        succeeded,
        failed,
    };
    
    Ok(Event::default().json_data(event).unwrap())
}

// ==================== 数据库操作函数 ====================

async fn create_batch_session(
    state: &AppState,
    session_id: &str,
    user_id: &str,
    total_files: usize,
) -> Result<()> {
    // TODO: 实际的数据库插入
    tracing::info!("创建批量上传会话: {}", session_id);
    Ok(())
}

async fn update_batch_session(
    state: &AppState,
    session_id: &str,
    succeeded: usize,
    failed: usize,
) -> Result<()> {
    // TODO: 实际的数据库更新
    tracing::info!("更新会话状态: {} (成功: {}, 失败: {})", session_id, succeeded, failed);
    Ok(())
}

async fn create_batch_detail(
    state: &AppState,
    session_id: &str,
    resume_id: &str,
    file_name: &str,
    file_size: usize,
) -> Result<()> {
    // TODO: 实际的数据库插入
    tracing::info!("创建批量详情: {}", file_name);
    Ok(())
}

async fn update_batch_detail_status(
    state: &AppState,
    resume_id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    // TODO: 实际的数据库更新
    tracing::info!("更新详情状态: {} -> {}", resume_id, status);
    Ok(())
}

async fn update_batch_detail_completed(
    state: &AppState,
    resume_id: &str,
    process_time_ms: u64,
) -> Result<()> {
    // TODO: 实际的数据库更新
    tracing::info!("完成处理: {} (耗时: {}ms)", resume_id, process_time_ms);
    Ok(())
}

async fn save_resume_to_db(
    state: &AppState,
    resume_id: &str,
    user_id: &str,
    file_name: &str,
    file_path: &str,
    file_size: usize,
    resume_info: &crate::services::deepseek_client::ExtractedResumeInfo,
) -> Result<()> {
    // TODO: 实际的数据库插入
    tracing::info!("保存简历到数据库: {}", resume_id);
    Ok(())
}

