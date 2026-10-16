use super::auth::Principal;
use super::error::{fail, fail_extra};
use super::AppState;
use axum::{http::StatusCode, response::Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

const LABELS: &[(&str, &str)] = &[
    ("jobseeker_core", "求职核心处理"),
    ("application_submit", "投递简历到职位"),
    ("resume_parse", "简历结构化提取"),
    ("job_match", "职位智能匹配"),
];

fn label(scope: &str) -> &str {
    LABELS
        .iter()
        .find(|(k, _)| *k == scope)
        .map(|(_, v)| *v)
        .unwrap_or(scope)
}

fn parent(scope: &str) -> Option<&'static str> {
    match scope {
        "resume_upload" | "resume_parse" | "job_match" | "report_generate" | "application_submit" => {
            Some("jobseeker_core")
        }
        _ => None,
    }
}

pub async fn require_consent(
    state: &AppState,
    principal: &Principal,
    scope: &str,
) -> Result<(), Response> {
    let shown = parent(scope).unwrap_or(scope);
    if !has_granted(state, &principal.user_id, scope).await? {
        if let Some(pkg) = parent(scope) {
            if has_granted(state, &principal.user_id, pkg).await? {
                return Ok(());
            }
        }
        return Err(fail_extra(
            StatusCode::FORBIDDEN,
            "consent_required",
            format!("需要授权: {}", label(shown)),
            json!({
                "required_scope": shown,
                "action": "grant_consent",
            }),
        ));
    }
    Ok(())
}

async fn has_granted(state: &AppState, user_id: &str, scope: &str) -> Result<bool, Response> {
    use crate::entities::consents;
    let row = consents::Entity::find()
        .filter(consents::Column::UserId.eq(user_id))
        .filter(consents::Column::Scope.eq(scope))
        .filter(consents::Column::Status.eq("granted"))
        .one(&state.db)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?;
    Ok(row.is_some())
}
