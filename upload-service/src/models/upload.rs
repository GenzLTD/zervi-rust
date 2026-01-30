use chrono::NaiveDateTime;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "uploads")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "String(Some(36))")]
    pub id: String,

    #[sea_orm(column_type = "String(Some(36))")]
    pub user_id: String,

    #[sea_orm(column_type = "String(Some(255))")]
    pub original_name: String,

    #[sea_orm(column_type = "String(Some(255))")]
    pub stored_name: String,

    #[sea_orm(column_type = "String(Some(500))")]
    pub file_path: String,

    pub file_size: i64,

    #[sea_orm(column_type = "String(Some(100))")]
    pub mime_type: String,

    #[sea_orm(column_type = "String(Some(20))")]
    pub storage_type: StorageType,

    #[sea_orm(column_type = "String(Some(20))")]
    pub status: UploadStatus,

    pub upload_ip: Option<String>,

    #[sea_orm(column_type = "String(Some(64))")]
    pub file_hash: Option<String>,

    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(Some(20))")]
pub enum StorageType {
    #[sea_orm(string_value = "local")]
    Local,
    #[sea_orm(string_value = "oss")]
    Oss,
}

impl std::fmt::Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageType::Local => write!(f, "local"),
            StorageType::Oss => write!(f, "oss"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(Some(20))")]
pub enum UploadStatus {
    #[sea_orm(string_value = "uploaded")]
    Uploaded,
    #[sea_orm(string_value = "parsing")]
    Parsing,
    #[sea_orm(string_value = "completed")]
    Completed,
    #[sea_orm(string_value = "failed")]
    Failed,
}

impl std::fmt::Display for UploadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadStatus::Uploaded => write!(f, "uploaded"),
            UploadStatus::Parsing => write!(f, "parsing"),
            UploadStatus::Completed => write!(f, "completed"),
            UploadStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
