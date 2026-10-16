use super::auth::Principal;
use super::error::fail_extra;
use super::now_tz;
use super::AppState;
use axum::{http::StatusCode, response::Response};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;

#[derive(Clone, Copy)]
pub enum Resource {
    ResumeParse,
    JobMatch,
}

impl Resource {
    fn key(self) -> &'static str {
        match self {
            Self::ResumeParse => "resume_parse",
            Self::JobMatch => "job_match",
        }
    }

    fn limit(self, tier: &str) -> i32 {
        let unlimited = matches!(tier, "supporter" | "pro" | "enterprise");
        if unlimited {
            return 99999;
        }
        match (tier, self) {
            ("guest", Self::ResumeParse) => 1,
            ("guest", Self::JobMatch) => 1,
            (_, Self::ResumeParse) => 3,
            (_, Self::JobMatch) => 5,
        }
    }
}

/// 日配额。不含 boost 包；超限返回 429，错误码与 looma 简历解析一致。
pub async fn consume_daily(
    state: &AppState,
    principal: &Principal,
    resource: Resource,
) -> Result<(), Response> {
    use crate::entities::quota_records;
    let date = now_tz().date_naive().format("%Y-%m-%d").to_string();
    let limit = resource.limit(&principal.tier);
    let existing = quota_records::Entity::find()
        .filter(quota_records::Column::UserId.eq(principal.user_id.clone()))
        .filter(quota_records::Column::Resource.eq(resource.key()))
        .filter(quota_records::Column::Date.eq(date.clone()))
        .one(&state.db)
        .await
        .map_err(|e| {
            fail_extra(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                e.to_string(),
                json!({}),
            )
        })?;

    if let Some(row) = existing {
        let used = row.used.unwrap_or(0);
        if used >= limit {
            return Err(quota_response(&principal.tier, resource));
        }
        let mut active: quota_records::ActiveModel = row.into();
        active.used = Set(Some(used + 1));
        active.update(&state.db).await.map_err(|e| {
            fail_extra(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                e.to_string(),
                json!({}),
            )
        })?;
        return Ok(());
    }

    let created = quota_records::ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        user_id: Set(principal.user_id.clone()),
        resource: Set(resource.key().into()),
        date: Set(date),
        used: Set(Some(1)),
        daily_limit: Set(limit),
    };
    created.insert(&state.db).await.map_err(|e| {
        fail_extra(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            e.to_string(),
            json!({}),
        )
    })?;
    Ok(())
}

fn quota_response(tier: &str, resource: Resource) -> Response {
    let message = match resource {
        Resource::ResumeParse => "当日简历解析配额已用尽",
        Resource::JobMatch => "当日职位匹配配额已用尽",
    };
    fail_extra(
        StatusCode::TOO_MANY_REQUESTS,
        "quota_exceeded",
        message,
        json!({ "upgrade": { "tier": tier, "reason": "quota_exhausted" } }),
    )
}
