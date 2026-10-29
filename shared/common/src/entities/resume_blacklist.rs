//! `resume_blacklist` —— 用户维屏蔽企业（手写，非 codegen）。
//!
//! 契约：looma-zervi backend/contracts/resume.v1.json v0.1.0 · blacklist_list / blacklist_add / blacklist_remove
//! DDL：shared/resume_blacklist_v0.1.sql（含分情况必填 CHECK + 三条部分唯一索引 + 随 users 级联）
//!
//! 两处刻意与 codegen 产物不同：
//! · 列名 `match_kind` 而非 `match` —— match 是多方言关键字，避开可省掉一整类引号问题；
//!   JSON 面仍是 `match`，序列化在 routes.rs 显式拼装，不靠 Serialize 直出。
//! · 无 auto_increment —— 主键是 uuid 文本（同 consents / report_sharing），由 handler 生成，
//!   便于客户端把 id 当不透明字符串用（契约 type: string）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "resume_blacklist")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_type = "Text")]
    pub user_id: String,
    #[sea_orm(column_type = "Text")]
    pub match_kind: String,
    #[sea_orm(column_type = "Text")]
    pub company_name: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub credit_code: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub enterprise_id: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub note: Option<String>,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::UserId",
        to = "super::users::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Users,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Users.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
