use chrono::NaiveDateTime;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "resume_extracted_info")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "String(Some(36))")]
    pub id: String,

    #[sea_orm(column_type = "String(Some(36))")]
    pub parsed_doc_id: String,

    #[sea_orm(column_type = "String(Some(36))")]
    pub upload_id: String,

    pub name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub location: Option<String>,
    pub summary: Option<String>,

    #[sea_orm(column_type = "Json")]
    pub education: Option<serde_json::Value>,

    #[sea_orm(column_type = "Json")]
    pub work_experience: Option<serde_json::Value>,

    #[sea_orm(column_type = "Json")]
    pub skills: Option<serde_json::Value>,

    pub quality_score: Option<i32>,
    pub completeness_score: Option<i32>,
    pub clarity_score: Option<i32>,
    pub professionalism_score: Option<i32>,
    pub quality_confidence: Option<f64>,
    pub extraction_confidence: Option<f64>,

    #[sea_orm(column_type = "String(Some(20))")]
    pub category: Option<String>,

    pub extracted_at: NaiveDateTime,
    pub extraction_duration_ms: Option<i32>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::parsed_document::Entity",
        from = "Column::ParsedDocId",
        to = "super::parsed_document::Column::Id"
    )]
    ParsedDocument,

    #[sea_orm(
        belongs_to = "super::upload::Entity",
        from = "Column::UploadId",
        to = "super::upload::Column::Id"
    )]
    Upload,
}

impl Related<super::parsed_document::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ParsedDocument.def()
    }
}

impl Related<super::upload::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Upload.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
