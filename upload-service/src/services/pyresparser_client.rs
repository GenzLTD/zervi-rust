use bytes::Bytes;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::utils::error::{AppError, Result};

/// pyresparser 客户端
#[derive(Clone)]
pub struct PyResParserClient {
    #[allow(dead_code)]
    client: reqwest::Client,
    #[allow(dead_code)]
    base_url: String,
}

/// pyresparser 解析响应
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyResParserResponse {
    pub code: i32,
    pub message: String,
    pub success: bool,
    pub data: Option<PyResParserData>,
    pub meta: Option<PyResParserMeta>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyResParserData {
    pub name: Option<String>,
    pub email: Option<String>,
    pub mobile_number: Option<String>,
    pub skills: Option<Vec<String>>,
    pub education: Option<Vec<serde_json::Value>>,
    pub text_length: Option<usize>,
    pub filename: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyResParserMeta {
    pub processing_time_ms: Option<i32>,
    pub parser: Option<String>,
    pub version: Option<String>,
    pub libraries: Option<Vec<String>>,
}

impl PyResParserClient {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10)) // 10秒超时
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url }
    }

    /// 解析简历（文件上传）
    #[allow(dead_code)]
    pub async fn parse_file(
        &self,
        file_data: Bytes,
        file_name: &str,
    ) -> Result<PyResParserResponse> {
        tracing::info!("📤 调用pyresparser解析: {}", file_name);

        let start = std::time::Instant::now();

        // 构建multipart表单
        let part = Part::bytes(file_data.to_vec()).file_name(file_name.to_string());

        let form = Form::new().part("file", part);

        // 发送请求
        let response = self
            .client
            .post(format!("{}/parse", self.base_url))
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("pyresparser请求失败: {}", e)))?;

        let duration = start.elapsed();

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::InternalServerError(format!(
                "pyresparser返回错误 {}: {}",
                status, error_text
            )));
        }

        let result: PyResParserResponse = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("pyresparser响应解析失败: {}", e))
        })?;

        tracing::info!("✅ pyresparser解析完成，耗时: {:?}", duration);

        Ok(result)
    }

    /// 解析简历（文本）
    #[allow(dead_code)]
    pub async fn parse_text(&self, text: &str) -> Result<PyResParserResponse> {
        tracing::info!("📝 调用pyresparser解析文本，长度: {}", text.len());

        let start = std::time::Instant::now();

        #[derive(Serialize)]
        struct TextRequest {
            text: String,
        }

        let response = self
            .client
            .post(format!("{}/parse", self.base_url))
            .json(&TextRequest {
                text: text.to_string(),
            })
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("pyresparser请求失败: {}", e)))?;

        let duration = start.elapsed();

        let result: PyResParserResponse = response.json().await.map_err(|e| {
            AppError::InternalServerError(format!("pyresparser响应解析失败: {}", e))
        })?;

        tracing::info!("✅ pyresparser文本解析完成，耗时: {:?}", duration);

        Ok(result)
    }

    /// 健康检查
    #[allow(dead_code)]
    pub async fn health_check(&self) -> Result<bool> {
        match self
            .client
            .get(format!("{}/health", self.base_url))
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}
