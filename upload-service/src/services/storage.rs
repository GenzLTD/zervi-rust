use anyhow::Result;
use bytes::Bytes;
use std::path::PathBuf;

/// 本地存储实现
pub struct LocalStorage {
    base_dir: PathBuf,
    base_url: String,
}

impl LocalStorage {
    pub fn new(base_dir: String, base_url: String) -> Self {
        let path = PathBuf::from(&base_dir);

        // 确保上传目录存在
        if !path.exists() {
            std::fs::create_dir_all(&path).expect("Failed to create upload directory");
        }

        Self {
            base_dir: path,
            base_url,
        }
    }

    /// 上传文件
    pub async fn upload(&self, file_name: &str, data: Bytes, _mime_type: &str) -> Result<String> {
        let file_path = self.base_dir.join(file_name);

        // 确保父目录存在
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 写入文件
        tokio::fs::write(&file_path, data).await?;

        Ok(file_name.to_string())
    }

    /// 删除文件
    pub async fn delete(&self, file_path: &str) -> Result<()> {
        let full_path = self.base_dir.join(file_path);

        if full_path.exists() {
            tokio::fs::remove_file(full_path).await?;
        }

        Ok(())
    }

    /// 获取访问URL
    pub async fn get_url(&self, file_path: &str) -> Result<String> {
        Ok(format!("{}/{}", self.base_url, file_path))
    }
}
