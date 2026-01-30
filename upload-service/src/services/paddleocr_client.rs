use crate::utils::error::{AppError, Result};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PaddleOCRResponse {
    pub success: bool,
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<OCRData>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OCRData {
    pub text: String,
    pub text_length: usize,
    pub line_count: usize,
    #[serde(default)]
    pub lines: Vec<String>,
}

/// PaddleOCR客户端
pub struct PaddleOCRClient {
    base_url: String,
    client: reqwest::Client,
}

impl PaddleOCRClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// OCR识别（支持PDF和图片）
    #[allow(dead_code)]
    pub async fn ocr_document(&self, file_data: &[u8], file_name: &str) -> Result<String> {
        tracing::info!(
            "📤 调用PaddleOCR识别: {}, 文件大小: {} bytes",
            file_name,
            file_data.len()
        );

        let start = std::time::Instant::now();

        // 构建multipart表单
        let part = Part::bytes(file_data.to_vec()).file_name(file_name.to_string());

        let form = Form::new().part("file", part);

        // 发送请求
        let response = self
            .client
            .post(format!("{}/ocr", self.base_url))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(120)) // OCR可能需要较长时间
            .send()
            .await
            .map_err(|e| {
                tracing::error!("PaddleOCR请求失败: {}", e);
                AppError::InternalServerError(format!("OCR服务请求失败: {}", e))
            })?;

        let ocr_response: PaddleOCRResponse = response.json().await.map_err(|e| {
            tracing::error!("解析PaddleOCR响应失败: {}", e);
            AppError::InternalServerError(format!("OCR响应解析失败: {}", e))
        })?;

        let duration = start.elapsed();

        if !ocr_response.success {
            tracing::error!("PaddleOCR识别失败: {}", ocr_response.message);
            return Err(AppError::InternalServerError(format!(
                "OCR识别失败: {}",
                ocr_response.message
            )));
        }

        let text = ocr_response
            .data
            .ok_or_else(|| AppError::InternalServerError("OCR响应无数据".to_string()))?
            .text;

        tracing::info!(
            "✅ PaddleOCR识别完成: {} 字符, 耗时: {:?}",
            text.len(),
            duration
        );

        Ok(text)
    }

    /// 快速OCR识别（仅提取关键信息用于分类，不完整OCR）
    /// 策略：只提取前N个字符或前几页，用于快速判断文档类型
    pub async fn quick_ocr_for_classification(
        &self,
        file_data: &[u8],
        file_name: &str,
    ) -> Result<String> {
        tracing::info!("⚡ 快速OCR识别（用于分类判断）: {}", file_name);

        let start = std::time::Instant::now();

        // 构建multipart表单（与完整OCR相同，但可以添加参数控制）
        let part = Part::bytes(file_data.to_vec()).file_name(file_name.to_string());

        let form = Form::new().part("file", part);

        // 发送请求（超时时间更短，因为只需要部分识别）
        let response = self
            .client
            .post(format!("{}/ocr", self.base_url))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(30)) // 更短的超时
            .send()
            .await
            .map_err(|e| {
                tracing::warn!("快速OCR请求失败: {}", e);
                AppError::InternalServerError(format!("快速OCR失败: {}", e))
            })?;

        let ocr_response: PaddleOCRResponse = response.json().await.map_err(|e| {
            tracing::warn!("解析快速OCR响应失败: {}", e);
            AppError::InternalServerError(format!("响应解析失败: {}", e))
        })?;

        let duration = start.elapsed();

        if !ocr_response.success {
            tracing::warn!("快速OCR识别失败: {}", ocr_response.message);
            return Err(AppError::InternalServerError(format!(
                "快速OCR失败: {}",
                ocr_response.message
            )));
        }

        let text = ocr_response
            .data
            .ok_or_else(|| AppError::InternalServerError("OCR响应无数据".to_string()))?
            .text;

        // 只取前1000字符用于分类（足够判断文档类型）
        let quick_text = if text.len() > 1000 {
            tracing::info!(
                "⚡ 快速OCR完成: 提取前1000字符（总{}字符），耗时: {:?}",
                text.len(),
                duration
            );
            text.chars().take(1000).collect()
        } else {
            tracing::info!("⚡ 快速OCR完成: {} 字符, 耗时: {:?}", text.len(), duration);
            text
        };

        Ok(quick_text)
    }

    /// 健康检查
    #[allow(dead_code)]
    pub async fn health_check(&self) -> Result<bool> {
        let response = self
            .client
            .get(format!("{}/health", self.base_url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("PaddleOCR健康检查失败: {}", e)))?;

        Ok(response.status().is_success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // 需要PaddleOCR服务运行
    async fn test_health_check() {
        let client = PaddleOCRClient::new("http://localhost:8080".to_string());
        let result = client.health_check().await;
        assert!(result.is_ok());
    }
}
