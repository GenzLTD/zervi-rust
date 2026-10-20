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

impl UserRole {
    /// 把各路来源的 role 词汇归一成本服务的三值枚举。
    ///
    /// 背景：平台侧（genzltd）的 `users.role` CHECK 只允许
    /// `user / enterprise / admin / driver`，而本服务（adirp_user）用
    /// `candidate / recruiter / admin`。网关签发的 JWT 可能带任一套词汇，
    /// 直接 match 会让平台词汇在建档时 400。
    ///
    /// 归一规则（唯一一处，避免各调用点各写一份 match）：
    /// - candidate / user / driver → Candidate（司机在求职侧也是供给方）
    /// - recruiter / enterprise     → Recruiter
    /// - admin                      → Admin
    ///
    /// 返回 `None` 表示未知词汇（如 observer）——调用方**必须拒绝**，
    /// 不得兜底成 candidate，否则无权限用户会被静默建档。
    ///
    /// # 使用边界（重要）
    ///
    /// 本函数只做**市场轴**（求职侧 / 企业侧）的档案词汇归一，
    /// 产出的是"这个人以什么身份出现在市场上"。
    /// 它**不得**用于任何授权判定：
    /// - 不等于记忆库 participant（记忆库仍是全局库，recall 不按 actor_sub 过滤）
    /// - 不写入 GFCC 八角梯 `roles[]`
    /// - 不改变任何门禁结果（例如上传闸门对 candidate 的拒绝照旧）
    ///
    /// 否则 `driver → Candidate` 这条归一会被当成提权路径。
    pub fn from_any(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "candidate" | "user" | "driver" => Some(UserRole::Candidate),
            "recruiter" | "enterprise" => Some(UserRole::Recruiter),
            "admin" => Some(UserRole::Admin),
            _ => None,
        }
    }
}

#[cfg(test)]
mod role_tests {
    use super::UserRole;

    #[test]
    fn native_vocabulary_is_accepted() {
        assert_eq!(UserRole::from_any("candidate"), Some(UserRole::Candidate));
        assert_eq!(UserRole::from_any("recruiter"), Some(UserRole::Recruiter));
        assert_eq!(UserRole::from_any("admin"), Some(UserRole::Admin));
    }

    #[test]
    fn platform_vocabulary_maps_to_native() {
        assert_eq!(UserRole::from_any("user"), Some(UserRole::Candidate));
        assert_eq!(UserRole::from_any("driver"), Some(UserRole::Candidate));
        assert_eq!(UserRole::from_any("enterprise"), Some(UserRole::Recruiter));
    }

    #[test]
    fn unknown_role_stays_unknown() {
        // 0c40b6f 的既定语义：observer 等未知值必须拒绝，不得兜底 candidate
        assert_eq!(UserRole::from_any("observer"), None);
        assert_eq!(UserRole::from_any(""), None);
        assert_eq!(UserRole::from_any("   "), None);
    }

    #[test]
    fn case_and_whitespace_are_normalized() {
        assert_eq!(UserRole::from_any(" ADMIN "), Some(UserRole::Admin));
        assert_eq!(UserRole::from_any("Enterprise"), Some(UserRole::Recruiter));
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

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserStatus::Active => write!(f, "active"),
            UserStatus::Disabled => write!(f, "disabled"),
        }
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // auth-service只管理用户认证相关数据
    // 简历、投递等关联在各自的服务中管理
}

impl ActiveModelBehavior for ActiveModel {}
