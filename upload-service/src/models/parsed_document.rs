use chrono::NaiveDateTime;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "parsed_documents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "String(Some(36))")]
    pub id: String,

    #[sea_orm(column_type = "String(Some(36))")]
    pub upload_id: String,

    #[sea_orm(column_type = "Text")]
    pub text_content: String,

    pub page_count: Option<i32>,
    pub tables_count: Option<i32>,
    pub images_count: Option<i32>,

    pub parsed_at: NaiveDateTime,
    pub parse_duration_ms: Option<i32>,

    #[sea_orm(column_type = "String(Some(32))")]
    pub classification_type: Option<String>,
    pub classification_confidence: Option<f64>,
    pub ocr_confidence: Option<f64>,
    pub ocr_text_length: Option<i32>,
    pub jd_confidence: Option<f64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::upload::Entity",
        from = "Column::UploadId",
        to = "super::upload::Column::Id"
    )]
    Upload,
}

impl Related<super::upload::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Upload.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
