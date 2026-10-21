//! 对外路径 `/v1/enterprise/*`。crate 名保持 company-service。

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use common::entities::{candidates, enterprise_users, enterprises, invite_codes, sales_inquiries, users};
use common::looma::{fail, now_tz, require_tier, resolve_principal, AppState, AuthMode, Principal};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/enterprise/create", axum::routing::post(create_enterprise))
        .route("/v1/enterprise/join", axum::routing::post(join_enterprise))
        .route("/v1/enterprise/profile", get(profile))
        .route(
            "/v1/enterprise/candidates",
            get(list_candidates).post(add_candidate),
        )
        .route("/v1/enterprise/candidates/add", axum::routing::post(add_candidate))
        .route(
            "/v1/enterprise/candidates/import-share",
            axum::routing::post(import_share),
        )
        .route("/v1/enterprise/candidate/:candidate_id", get(get_candidate))
        .route("/v1/enterprise/contact-sales", axum::routing::post(contact_sales))
        .with_state(state)
}

async fn required(state: &AppState, headers: &HeaderMap) -> Result<Principal, Response> {
    resolve_principal(state, headers, AuthMode::Required).await
}

#[derive(Deserialize, Default)]
struct CreateBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: String,
}

async fn create_enterprise(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Response {
    let principal = match required(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if body.name.trim().is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "enterprise name required");
    }
    let enterprise_id = uuid::Uuid::new_v4().to_string();
    let now = Some(now_tz());
    let ent = enterprises::ActiveModel {
        id: Set(enterprise_id.clone()),
        name: Set(body.name.clone()),
        domain: Set(Some(body.domain.clone())),
        plan: Set(None),
        created_at: Set(now),
    };
    if let Err(e) = ent.insert(&state.db).await {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "create_failed", e.to_string());
    }
    let link = enterprise_users::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        enterprise_id: Set(enterprise_id.clone()),
        user_id: Set(principal.user_id),
        role: Set(Some("admin".into())),
        created_at: Set(now),
    };
    if let Err(e) = link.insert(&state.db).await {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "create_failed", e.to_string());
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "id": enterprise_id,
            "name": body.name,
            "domain": body.domain,
            "role": "admin",
        })),
    )
        .into_response()
}

#[derive(Deserialize, Default)]
struct JoinBody {
    #[serde(default)]
    enterprise_id: String,
}

async fn join_enterprise(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<JoinBody>,
) -> Response {
    let principal = match required(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if body.enterprise_id.trim().is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "enterprise_id required");
    }
    if enterprises::Entity::find_by_id(body.enterprise_id.clone())
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return fail(StatusCode::NOT_FOUND, "not_found", "企业不存在");
    }
    let existing = enterprise_users::Entity::find()
        .filter(enterprise_users::Column::EnterpriseId.eq(body.enterprise_id.clone()))
        .filter(enterprise_users::Column::UserId.eq(principal.user_id.clone()))
        .one(&state.db)
        .await
        .ok()
        .flatten();
    if existing.is_some() {
        return fail(StatusCode::CONFLICT, "conflict", "已经是该企业的成员");
    }
    let link = enterprise_users::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        enterprise_id: Set(body.enterprise_id.clone()),
        user_id: Set(principal.user_id),
        role: Set(Some("member".into())),
        created_at: Set(Some(now_tz())),
    };
    match link.insert(&state.db).await {
        Ok(_) => Json(json!({ "enterprise_id": body.enterprise_id, "role": "member" })).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "join_failed", e.to_string()),
    }
}

async fn profile(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match required(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(link) = membership(&state, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "不属于任何企业");
    };
    let Some(ent) = enterprises::Entity::find_by_id(link.enterprise_id.clone())
        .one(&state.db)
        .await
        .ok()
        .flatten()
    else {
        return fail(StatusCode::NOT_FOUND, "not_found", "不属于任何企业");
    };
    Json(json!({
        "id": ent.id,
        "name": ent.name,
        "domain": ent.domain,
        "plan": ent.plan,
        "role": link.role,
    }))
    .into_response()
}

async fn list_candidates(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(link) = membership(&state, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "请先创建或加入企业");
    };
    let rows = candidates::Entity::find()
        .filter(candidates::Column::EnterpriseId.eq(link.enterprise_id))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let items: Vec<Value> = rows.iter().map(candidate_json).collect();
    Json(json!({ "candidates": items, "total": items.len() })).into_response()
}

async fn get_candidate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(candidate_id): Path<String>,
) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(link) = membership(&state, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "请先创建或加入企业");
    };
    match candidates::Entity::find_by_id(candidate_id)
        .filter(candidates::Column::EnterpriseId.eq(link.enterprise_id))
        .one(&state.db)
        .await
        .ok()
        .flatten()
    {
        Some(row) => Json(candidate_json(&row)).into_response(),
        None => fail(StatusCode::NOT_FOUND, "not_found", "候选人不存在"),
    }
}

#[derive(Deserialize, Default)]
struct AddBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    user_id: String,
}

async fn add_candidate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AddBody>,
) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(link) = membership(&state, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "请先创建或加入企业");
    };
    if body.name.trim().is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "name required");
    }
    let row = candidates::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        enterprise_id: Set(link.enterprise_id),
        user_id: Set(if body.user_id.is_empty() { None } else { Some(body.user_id) }),
        name: Set(body.name),
        email: Set(if body.email.is_empty() { None } else { Some(body.email) }),
        phone: Set(if body.phone.is_empty() { None } else { Some(body.phone) }),
        status: Set(Some("new".into())),
        profile_data: Set(None),
        created_at: Set(Some(now_tz())),
    };
    match row.insert(&state.db).await {
        Ok(saved) => (StatusCode::CREATED, Json(candidate_json(&saved))).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "create_failed", e.to_string()),
    }
}

#[derive(Deserialize, Default)]
struct ShareBody {
    #[serde(default)]
    share_code: String,
}

async fn import_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ShareBody>,
) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let code = body.share_code.trim().to_uppercase();
    if code.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "share_code required");
    }
    let Some(link) = membership(&state, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "请先创建或加入企业");
    };
    let Some(invite) = invite_codes::Entity::find()
        .filter(invite_codes::Column::Code.eq(code))
        .one(&state.db)
        .await
        .ok()
        .flatten()
    else {
        return fail(StatusCode::NOT_FOUND, "not_found", "分享码无效");
    };
    let Some(seeker_id) = invite.created_by else {
        return fail(StatusCode::NOT_FOUND, "not_found", "分享码无效");
    };
    if let Some(existing) = candidates::Entity::find()
        .filter(candidates::Column::EnterpriseId.eq(link.enterprise_id.clone()))
        .filter(candidates::Column::UserId.eq(seeker_id.clone()))
        .one(&state.db)
        .await
        .ok()
        .flatten()
    {
        return Json(candidate_json(&existing)).into_response();
    }
    let seeker = users::Entity::find_by_id(seeker_id.clone())
        .one(&state.db)
        .await
        .ok()
        .flatten();
    let name = seeker
        .as_ref()
        .and_then(|u| u.name.clone())
        .unwrap_or_else(|| "未命名候选人".into());
    let row = candidates::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        enterprise_id: Set(link.enterprise_id),
        user_id: Set(Some(seeker_id)),
        name: Set(name),
        email: Set(seeker.and_then(|u| u.email)),
        phone: Set(None),
        status: Set(Some("imported".into())),
        profile_data: Set(None),
        created_at: Set(Some(now_tz())),
    };
    match row.insert(&state.db).await {
        Ok(saved) => (StatusCode::CREATED, Json(candidate_json(&saved))).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "import_failed", e.to_string()),
    }
}

#[derive(Deserialize, Default)]
struct SalesBody {
    #[serde(default)]
    company_name: String,
    #[serde(default)]
    contact_name: String,
    #[serde(default)]
    contact_email: String,
    #[serde(default)]
    contact_phone: String,
    #[serde(default)]
    message: String,
}

async fn contact_sales(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SalesBody>,
) -> Response {
    let principal = match required(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if body.company_name.trim().is_empty() || body.contact_email.trim().is_empty() {
        return fail(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "company_name and contact_email required",
        );
    }
    let row = sales_inquiries::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        user_id: Set(Some(principal.user_id)),
        company_name: Set(body.company_name),
        contact_name: Set(body.contact_name),
        contact_email: Set(body.contact_email),
        contact_phone: Set(if body.contact_phone.is_empty() {
            None
        } else {
            Some(body.contact_phone)
        }),
        scale: Set(None),
        message: Set(if body.message.is_empty() { None } else { Some(body.message) }),
        status: Set(Some("new".into())),
        created_at: Set(Some(now_tz())),
    };
    match row.insert(&state.db).await {
        Ok(saved) => (StatusCode::CREATED, Json(json!({ "id": saved.id, "status": saved.status }))).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "create_failed", e.to_string()),
    }
}

async fn supporter(state: &AppState, headers: &HeaderMap) -> Result<Principal, Response> {
    let principal = required(state, headers).await?;
    require_tier(&principal, "supporter")?;
    Ok(principal)
}

async fn membership(state: &AppState, user_id: &str) -> Option<enterprise_users::Model> {
    enterprise_users::Entity::find()
        .filter(enterprise_users::Column::UserId.eq(user_id))
        .one(&state.db)
        .await
        .ok()
        .flatten()
}

fn candidate_json(row: &candidates::Model) -> Value {
    json!({
        "id": row.id,
        "enterprise_id": row.enterprise_id,
        "user_id": row.user_id,
        "name": row.name,
        "email": row.email,
        "phone": row.phone,
        "status": row.status,
    })
}
