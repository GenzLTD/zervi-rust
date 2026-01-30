use bytes::Bytes;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::utils::error::{AppError, Result};

/// MinerU 客户端
#[derive(Clone)]
pub struct MinerUClient {
    #[allow(dead_code)]
    client: reqwest::Client,
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    enabled: bool,
}

/// MinerU 解析响应
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerUResponse {
    pub success: bool,
    pub data: Option<MinerUData>,
    pub error: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinerUData {
    pub text: String,
    pub tables: Option<Vec<serde_json::Value>>,
    pub images: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

impl MinerUClient {
    pub fn new(base_url: Option<String>) -> Self {
        let (url, enabled) = match base_url {
            Some(u) => (u, true),
            None => ("http://localhost:8080".to_string(), false),
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30)) // MinerU可能需要更长时间
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url: url,
            enabled,
        }
    }

    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 解析文档
    #[allow(dead_code)]
    pub async fn parse_document(
        &self,
        file_data: Bytes,
        file_name: &str,
    ) -> Result<MinerUResponse> {
        if !self.enabled {
            return Err(AppError::InternalServerError("MinerU未启用".to_string()));
        }

        tracing::info!("📤 调用MinerU解析: {}", file_name);

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
            .map_err(|e| AppError::InternalServerError(format!("MinerU请求失败: {}", e)))?;

        let duration = start.elapsed();

        let result: MinerUResponse = response
            .json()
            .await
            .map_err(|e| AppError::InternalServerError(format!("MinerU响应解析失败: {}", e)))?;

        tracing::info!("✅ MinerU解析完成，耗时: {:?}", duration);

        Ok(result)
    }

    /// 健康检查
    #[allow(dead_code)]
    pub async fn health_check(&self) -> Result<bool> {
        if !self.enabled {
            return Ok(false);
        }

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
