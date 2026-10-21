//! Looma 契约层：错误体、双轨身份、同意、配额、X1 出境守卫。
//!
//! 四服务（resume / job / application / company）共用。会员与账户导出不在本轮。

mod auth;
mod consent;
mod error;
mod quota;
mod x1;

pub use auth::{require_tier, resolve_principal, AuthMode, Principal};
pub use consent::require_consent;
pub use error::{fail, fail_extra, LoomaError};
pub use quota::{consume_daily, Resource};
pub use x1::{
    call_llm, is_overseas_deploy, qcc_disabled_reason, safe_llm_provider_order, Settings,
};

use sea_orm::DatabaseConnection;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub settings: Arc<Settings>,
}

impl AppState {
    pub async fn connect() -> Result<Self, sea_orm::DbErr> {
        let settings = Arc::new(Settings::from_env());
        let db = sea_orm::Database::connect(&settings.database_url).await?;
        Ok(Self { db, settings })
    }
}

pub fn now_tz() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Utc::now().fixed_offset()
}
