//! `/v1/application*` 五个端点，外加跨前缀 `GET /v1/jobs/:job_id/applications`。
//! 职位存在性先查 documents（doc_type=job），再查 job_posts。本轮不收口双轨。

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use common::entities::{applications, documents, job_posts};
use common::looma::{fail, now_tz, require_consent, resolve_principal, AppState, AuthMode};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/application",
            axum::routing::post(create_application).get(list_applications),
        )
        .route(
            "/v1/application/:application_id",
            get(get_application).delete(delete_application),
        )
        .route("/v1/application/:application_id/report", get(application_report))
        .route("/v1/jobs/:job_id/applications", get(job_applications))
        .with_state(state)
}

#[derive(Deserialize, Default)]
struct CreateBody {
    #[serde(default)]
    resume_id: String,
    #[serde(default)]
    job_id: String,
    #[serde(default)]
    enterprise_id: String,
    #[serde(default)]
    source: String,
}

async fn create_application(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(r) = require_consent(&state, &principal, "application_submit").await {
        return r;
    }
    if body.resume_id.trim().is_empty() || body.job_id.trim().is_empty() {
        return fail(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "resume_id and job_id are required",
        );
    }
    let Ok(resume_id) = body.resume_id.parse::<i64>() else {
        return fail(StatusCode::NOT_FOUND, "not_found", "简历不存在或不属于当前用户");
    };
    let owned = documents::Entity::find_by_id(resume_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .filter(|d| d.doc_type == "resume" && meta_user(d.metadata.as_deref()) == principal.user_id);
    let Some(resume) = owned else {
        return fail(StatusCode::NOT_FOUND, "not_found", "简历不存在或不属于当前用户");
    };
    if !job_exists(&state, &body.job_id).await {
        return fail(StatusCode::NOT_FOUND, "not_found", "职位不存在");
    }
    if let Ok(Some(existing)) = applications::Entity::find()
        .filter(applications::Column::SeekerUserId.eq(principal.user_id.clone()))
        .filter(applications::Column::ResumeId.eq(resume_id))
        .filter(applications::Column::JobId.eq(body.job_id.clone()))
        .filter(applications::Column::Status.eq("submitted"))
        .one(&state.db)
        .await
    {
        return Json(json!({
            "application": app_json(&existing),
            "already_exists": true,
        }))
        .into_response();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_tz();
    let model = applications::ActiveModel {
        id: Set(id),
        seeker_user_id: Set(principal.user_id.clone()),
        resume_id: Set(resume_id),
        job_id: Set(body.job_id.clone()),
        enterprise_id: Set(if body.enterprise_id.is_empty() {
            None
        } else {
            Some(body.enterprise_id)
        }),
        status: Set(Some("submitted".into())),
        metadata: Set(Some(
            json!({
                "source": if body.source.is_empty() { "api" } else { body.source.as_str() },
                "resume_title": resume.title,
            })
            .to_string(),
        )),
        created_at: Set(now),
        updated_at: Set(now),
        withdrawn_at: Set(None),
    };
    match model.insert(&state.db).await {
        Ok(row) => (
            StatusCode::CREATED,
            Json(json!({ "application": app_json(&row), "already_exists": false })),
        )
            .into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    status: String,
}

async fn list_applications(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let mut finder = applications::Entity::find()
        .filter(applications::Column::SeekerUserId.eq(principal.user_id));
    if !q.status.trim().is_empty() {
        finder = finder.filter(applications::Column::Status.eq(q.status));
    }
    let rows = finder
        .order_by_desc(applications::Column::CreatedAt)
        .limit(100)
        .all(&state.db)
        .await
        .unwrap_or_default();
    let items: Vec<Value> = rows.iter().map(app_json).collect();
    Json(json!({ "applications": items, "total": items.len() })).into_response()
}

async fn get_application(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(application_id): Path<String>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match owned(&state, &application_id, &principal.user_id).await {
        Some(row) => Json(json!({ "application": app_json(&row) })).into_response(),
        None => fail(StatusCode::NOT_FOUND, "not_found", "投递不存在"),
    }
}

async fn delete_application(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(application_id): Path<String>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(row) = owned(&state, &application_id, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "投递不存在");
    };
    let mut active: applications::ActiveModel = row.into();
    active.status = Set(Some("withdrawn".into()));
    active.withdrawn_at = Set(Some(now_tz()));
    active.updated_at = Set(now_tz());
    match active.update(&state.db).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "delete_failed", e.to_string()),
    }
}

async fn application_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(application_id): Path<String>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(app) = owned(&state, &application_id, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "投递不存在");
    };
    let resume = documents::Entity::find_by_id(app.resume_id)
        .one(&state.db)
        .await
        .ok()
        .flatten();
    let markdown = resume
        .as_ref()
        .and_then(|d| d.metadata.as_deref())
        .and_then(|m| serde_json::from_str::<Value>(m).ok())
        .and_then(|v| v.get("markdown").and_then(|s| s.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    if markdown.trim().len() < 20 {
        return fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unprocessable",
            "resume/job 内容不足以计算匹配",
        );
    }
    let Some(raw) = common::looma::call_llm(
        &state.settings,
        &format!("生成匹配报告 JSON。简历：\n{markdown}"),
        0.2,
    )
    .await
    else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "match_failed",
            "AI 服务暂不可用（区域过滤后无可用模型，或上游失败）",
        );
    };
    Json(json!({
        "application_id": app.id,
        "resume_id": app.resume_id.to_string(),
        "job_id": app.job_id,
        "match_report": json_block(&raw).unwrap_or(json!({"raw": raw})),
    }))
    .into_response()
}

async fn job_applications(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !job_owned_by(&state, &job_id, &principal.user_id).await
        && !matches!(principal.role.as_str(), "admin" | "hr" | "enterprise")
    {
        return fail(StatusCode::FORBIDDEN, "forbidden", "无权查看该职位的投递");
    }
    let rows = applications::Entity::find()
        .filter(applications::Column::JobId.eq(job_id))
        .order_by_desc(applications::Column::CreatedAt)
        .all(&state.db)
        .await
        .unwrap_or_default();
    let items: Vec<Value> = rows.iter().map(app_json).collect();
    Json(json!({ "applications": items, "total": items.len() })).into_response()
}

fn app_json(row: &applications::Model) -> Value {
    json!({
        "id": row.id,
        "seeker_user_id": row.seeker_user_id,
        "resume_id": row.resume_id.to_string(),
        "job_id": row.job_id,
        "enterprise_id": row.enterprise_id,
        "status": row.status,
        "metadata": row.metadata,
        "created_at": row.created_at.to_rfc3339(),
        "updated_at": row.updated_at.to_rfc3339(),
    })
}

fn meta_user(raw: Option<&str>) -> String {
    raw.and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.get("user_id").and_then(|u| u.as_str()).map(|s| s.to_string()))
        .unwrap_or_default()
}

async fn owned(state: &AppState, id: &str, user_id: &str) -> Option<applications::Model> {
    applications::Entity::find_by_id(id.to_string())
        .filter(applications::Column::SeekerUserId.eq(user_id))
        .one(&state.db)
        .await
        .ok()
        .flatten()
}

async fn job_exists(state: &AppState, job_id: &str) -> bool {
    if job_posts::Entity::find_by_id(job_id.to_string())
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .is_some()
    {
        return true;
    }
    documents::Entity::find()
        .filter(documents::Column::DocType.eq("job"))
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .any(|d| {
            d.metadata
                .as_deref()
                .and_then(|m| serde_json::from_str::<Value>(m).ok())
                .map(|meta| {
                    meta.get("job_id").and_then(|v| v.as_str()) == Some(job_id)
                        || meta.pointer("/parsed/id").and_then(|v| v.as_str()) == Some(job_id)
                })
                .unwrap_or(false)
        })
}

async fn job_owned_by(state: &AppState, job_id: &str, user_id: &str) -> bool {
    if let Ok(Some(post)) = job_posts::Entity::find_by_id(job_id.to_string()).one(&state.db).await {
        if post.user_id == user_id {
            return true;
        }
    }
    documents::Entity::find()
        .filter(documents::Column::DocType.eq("job"))
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .any(|d| {
            let meta: Value = serde_json::from_str(d.metadata.as_deref().unwrap_or("{}")).unwrap_or(json!({}));
            let id_ok = meta.get("job_id").and_then(|v| v.as_str()) == Some(job_id)
                || meta.pointer("/parsed/id").and_then(|v| v.as_str()) == Some(job_id);
            let owner_ok = meta.get("user_id").and_then(|v| v.as_str()) == Some(user_id);
            id_ok && owner_ok
        })
}

fn json_block(raw: &str) -> Option<Value> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    serde_json::from_str(&raw[start..=end]).ok()
}
