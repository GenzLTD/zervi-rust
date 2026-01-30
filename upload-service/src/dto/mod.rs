use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadRecord {
    pub id: String,
    pub original_name: String,
    pub file_size: i64,
    pub mime_type: String,
    pub access_url: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<ClassificationDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_info: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationDto {
    pub doc_type: String,
    pub confidence: f64,
    pub matched_features: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadListItem {
    pub id: String,
    pub original_name: String,
    pub file_size: i64,
    pub mime_type: String,
    pub storage_type: String,
    pub status: String,
    pub access_url: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadDetail {
    pub id: String,
    pub user_id: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_path: String,
    pub file_size: i64,
    pub mime_type: String,
    pub storage_type: String,
    pub status: String,
    pub access_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUploadResult {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub processing: usize,
    pub results: Vec<BatchUploadItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchUploadItem {
    pub file_name: String,
    pub status: String,
    pub upload_id: Option<String>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<ClassificationDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_info: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParsedDocumentDto {
    pub id: String,
    pub upload_id: String,
    pub text_content: String,
    pub page_count: i32,
    pub tables_count: i32,
    pub parsed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_text_length: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jd_confidence: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumeInfoDto {
    pub id: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub location: Option<String>,
    pub summary: Option<String>,
    pub education: Vec<serde_json::Value>,
    pub work_experience: Vec<serde_json::Value>,
    pub skills: Vec<String>,
    pub quality_score: i32,
    pub completeness_score: i32,
    pub clarity_score: i32,
    pub professionalism_score: i32,
    pub quality_confidence: f64,
    pub extraction_confidence: f64,
    pub category: String,
    pub extracted_at: String,
}
