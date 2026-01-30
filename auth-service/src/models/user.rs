use chrono::NaiveDateTime;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "String(Some(36))")]
    pub id: String,

    #[sea_orm(unique)]
    pub phone: String,

    pub email: Option<String>,
    pub password_hash: String,
    pub name: String,
    pub avatar: Option<String>,

    #[sea_orm(column_type = "String(Some(20))")]
    pub role: UserRole,

    #[sea_orm(column_type = "String(Some(20))")]
    pub status: UserStatus,

    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// auth-service中移除与其他服务的关联
// 这些关联在各自的服务中独立管理

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(Some(20))")]
pub enum UserRole {
    #[sea_orm(string_value = "candidate")]
    Candidate,
    #[sea_orm(string_value = "recruiter")]
    Recruiter,
    #[sea_orm(string_value = "admin")]
    Admin,
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::Candidate => write!(f, "candidate"),
            UserRole::Recruiter => write!(f, "recruiter"),
            UserRole::Admin => write!(f, "admin"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(Some(20))")]
pub enum UserStatus {
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "disabled")]
    Disabled,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // auth-service只管理用户认证相关数据
    // 简历、投递等关联在各自的服务中管理
}

impl ActiveModelBehavior for ActiveModel {}
