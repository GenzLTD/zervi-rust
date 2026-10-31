//! `/v1/jobs/*`（9）与 `/v1/job-posts*`（5）。B 轨 job_posts 不收口。
//! `GET /v1/job-posts/:id/matches` 是关键词计分，不进 LLM。
//! `POST /v1/jobs/match` 对库内职位检索；理由可走 X1，对不上 job_id 的项丢弃。
//! `POST /v1/jobs/parse|upload` 的模型调用走 X1。

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use common::entities::{documents, job_posts};
use common::looma::{
    consume_daily, fail, now_tz, require_tier, resolve_principal, AppState, AuthMode, Principal,
    Resource,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/jobs/list", get(list_jobs))
        .route("/v1/jobs/upload", axum::routing::post(upload_job))
        .route("/v1/jobs/parse", axum::routing::post(parse_job))
        .route("/v1/jobs/match", axum::routing::post(match_jobs))
        .route("/v1/jobs/search", get(search_jobs))
        .route("/v1/jobs/seed-demo", axum::routing::post(seed_demo))
        .route("/v1/jobs/recommend", get(recommend_jobs))
        .route("/v1/jobs", get(list_jobs))
        .route("/v1/jobs/:job_id", get(job_detail))
        .route("/v1/job-posts", axum::routing::post(create_post).get(list_posts))
        .route(
            "/v1/job-posts/:post_id",
            axum::routing::put(update_post).delete(delete_post),
        )
        .route("/v1/job-posts/:post_id/matches", get(post_matches))
        .with_state(state)
}

async fn auth(state: &AppState, headers: &HeaderMap, mode: AuthMode) -> Result<Principal, Response> {
    resolve_principal(state, headers, mode).await
}

async fn list_jobs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let _ = auth(&state, &headers, AuthMode::Optional).await;
    match load_jobs(&state).await {
        Ok(jobs) => {
            let total = jobs.len();
            Json(json!({ "jobs": jobs, "total": total })).into_response()
        }
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e),
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    location: String,
}

async fn search_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SearchQuery>,
) -> Response {
    let _ = auth(&state, &headers, AuthMode::Optional).await;
    let jobs = match load_jobs(&state).await {
        Ok(j) => j,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e),
    };
    let needle = q.q.trim().to_lowercase();
    let loc = q.location.trim().to_lowercase();
    if needle.is_empty() && loc.is_empty() {
        let total = jobs.len();
        return Json(json!({ "jobs": jobs, "total": total })).into_response();
    }
    let filtered: Vec<Value> = jobs
        .into_iter()
        .filter(|j| {
            let blob = j.to_string().to_lowercase();
            (needle.is_empty() || blob.contains(&needle))
                && (loc.is_empty() || blob.contains(&loc))
        })
        .collect();
    let total = filtered.len();
    Json(json!({ "jobs": filtered, "total": total })).into_response()
}

async fn recommend_jobs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    list_jobs(State(state), headers).await
}

async fn job_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Response {
    let _ = auth(&state, &headers, AuthMode::Optional).await;
    let jobs = match load_jobs(&state).await {
        Ok(j) => j,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e),
    };
    jobs.into_iter()
        .find(|j| j.get("id").and_then(|v| v.as_str()) == Some(job_id.as_str()))
        .map(|j| Json(j).into_response())
        .unwrap_or_else(|| fail(StatusCode::NOT_FOUND, "not_found", "职位不存在"))
}

#[derive(Deserialize, Default)]
struct TextBody {
    #[serde(default)]
    text: String,
}

async fn parse_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TextBody>,
) -> Response {
    let principal = match auth(&state, &headers, AuthMode::Optional).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if body.text.trim().is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "job text required");
    }
    llm_extract(&state, &principal, &body.text).await
}

async fn upload_job(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let _ = match auth(&state, &headers, AuthMode::Optional).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    fail(
        StatusCode::UNPROCESSABLE_ENTITY,
        "convert_failed",
        "职位文件解析尚未接入 MarkItDown，请改用 POST /v1/jobs/parse 提交纯文本",
    )
}

#[derive(Deserialize, Default)]
struct MatchBody {
    #[serde(default)]
    resume_text: String,
}

async fn match_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MatchBody>,
) -> Response {
    let principal = match auth(&state, &headers, AuthMode::Optional).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if body.resume_text.trim().len() < 20 {
        return fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unprocessable",
            "resume 内容不足以计算匹配",
        );
    }
    if let Err(r) = consume_daily(&state, &principal, Resource::JobMatch).await {
        return r;
    }
    match library_match_payload(&state, body.resume_text.trim()).await {
        Ok(payload) => Json(payload).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e),
    }
}

async fn library_match_payload(state: &AppState, resume_text: &str) -> Result<Value, String> {
    let jobs = load_jobs(state).await?;
    let total_evaluated = jobs.iter().filter_map(crate::library_match::job_id).count();
    if total_evaluated == 0 {
        return Ok(json!({
            "matches": [],
            "total_evaluated": 0,
            "match_status": "empty_library",
        }));
    }
    let ranked = crate::library_match::rank_library_jobs(resume_text, &jobs);
    if ranked.is_empty() {
        return Ok(json!({
            "matches": [],
            "total_evaluated": total_evaluated,
            "match_status": "no_fit",
        }));
    }
    let prompt_ids: Vec<String> = ranked
        .iter()
        .filter_map(|m| m.get("job_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let matches = if let Some(raw) = common::looma::call_llm_unredacted(
        &state.settings,
        &crate::library_match::catalog_prompt(resume_text, &ranked, &jobs),
        0.2,
    )
    .await
    {
        let parsed = json_block(&raw).unwrap_or_else(|| json!({}));
        let llm_matches = parsed
            .get("matches")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        crate::library_match::pin_to_catalog(&prompt_ids, &llm_matches, &ranked)
    } else {
        ranked
    };
    Ok(json!({
        "matches": matches,
        "total_evaluated": total_evaluated,
        "match_status": "ok",
    }))
}

#[derive(Deserialize, Default)]
struct SeedBody {
    #[serde(default)]
    jobs: Vec<Value>,
}

async fn seed_demo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SeedBody>,
) -> Response {
    let principal = match auth(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let jobs = if body.jobs.is_empty() {
        vec![json!({"id": "demo-1", "title": "演示职位", "company": "Demo"})]
    } else {
        body.jobs
    };
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    for job in jobs {
        let job_id = job
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("demo")
            .to_string();
        if job_exists_doc(&state, &job_id).await.unwrap_or(false) {
            skipped.push(job_id);
            continue;
        }
        let title = job
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("未命名职位");
        let mut meta = json!({ "user_id": principal.user_id, "job_id": job_id, "parsed": job });
        meta["parsed"]["id"] = json!(job_id);
        let model = documents::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            title: Set(title.into()),
            file_path: Set(format!("job/{}/{}", principal.user_id, uuid::Uuid::new_v4())),
            doc_type: Set("job".into()),
            file_size: Set(Some(0)),
            metadata: Set(Some(meta.to_string())),
            status: Set(Some("processed".into())),
            chunk_count: Set(None),
            created_at: Set(now_tz()),
            processed_at: Set(None),
        };
        if model.insert(&state.db).await.is_ok() {
            created.push(job_id);
        }
    }
    Json(json!({ "created": created, "skipped": skipped })).into_response()
}

#[derive(Deserialize, Default)]
struct PostBody {
    #[serde(default)]
    title: String,
    #[serde(default)]
    company: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    requirements: String,
    #[serde(default)]
    status: String,
}

async fn create_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PostBody>,
) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if body.title.trim().is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "title required");
    }
    let limit = post_limit(&principal.tier);
    let count = job_posts::Entity::find()
        .filter(job_posts::Column::UserId.eq(principal.user_id.clone()))
        .all(&state.db)
        .await
        .map(|r| r.len())
        .unwrap_or(0);
    if count >= limit {
        return fail(StatusCode::FORBIDDEN, "forbidden", "职位发布数量已达当前档位上限");
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = Some(now_tz());
    let model = job_posts::ActiveModel {
        id: Set(id.clone()),
        user_id: Set(principal.user_id.clone()),
        title: Set(body.title.clone()),
        company: Set(Some(body.company.clone())),
        description: Set(Some(body.description.clone())),
        requirements: Set(Some(body.requirements.clone())),
        status: Set(Some(if body.status.is_empty() { "active".into() } else { body.status })),
        created_at: Set(now),
        updated_at: Set(now),
    };
    match model.insert(&state.db).await {
        Ok(row) => (StatusCode::CREATED, Json(post_json(&row))).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "create_failed", e.to_string()),
    }
}

async fn list_posts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let rows = job_posts::Entity::find()
        .filter(job_posts::Column::UserId.eq(principal.user_id.clone()))
        .order_by_desc(job_posts::Column::CreatedAt)
        .all(&state.db)
        .await
        .unwrap_or_default();
    let count = rows.len();
    let posts: Vec<Value> = rows.iter().map(post_json).collect();
    Json(json!({
        "job_posts": posts,
        "limit": post_limit(&principal.tier),
        "count": count,
    }))
    .into_response()
}

async fn update_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(post_id): Path<String>,
    Json(body): Json<PostBody>,
) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(row) = owned_post(&state, &post_id, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "职位不存在");
    };
    let mut active: job_posts::ActiveModel = row.into();
    if !body.title.trim().is_empty() {
        active.title = Set(body.title);
    }
    if !body.company.is_empty() {
        active.company = Set(Some(body.company));
    }
    if !body.description.is_empty() {
        active.description = Set(Some(body.description));
    }
    if !body.requirements.is_empty() {
        active.requirements = Set(Some(body.requirements));
    }
    active.updated_at = Set(Some(now_tz()));
    match active.update(&state.db).await {
        Ok(row) => Json(post_json(&row)).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "update_failed", e.to_string()),
    }
}

async fn delete_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(post_id): Path<String>,
) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let res = job_posts::Entity::delete_many()
        .filter(job_posts::Column::Id.eq(post_id))
        .filter(job_posts::Column::UserId.eq(principal.user_id))
        .exec(&state.db)
        .await;
    match res {
        Ok(r) if r.rows_affected > 0 => Json(json!({ "ok": true })).into_response(),
        Ok(_) => fail(StatusCode::NOT_FOUND, "not_found", "职位不存在"),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "delete_failed", e.to_string()),
    }
}

async fn post_matches(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(post_id): Path<String>,
) -> Response {
    let principal = match supporter(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(post) = owned_post(&state, &post_id, &principal.user_id).await else {
        return fail(StatusCode::NOT_FOUND, "not_found", "职位不存在");
    };
    use common::entities::{candidates, enterprise_users};
    let Some(eu) = enterprise_users::Entity::find()
        .filter(enterprise_users::Column::UserId.eq(principal.user_id.clone()))
        .one(&state.db)
        .await
        .ok()
        .flatten()
    else {
        return fail(StatusCode::NOT_FOUND, "not_found", "请先创建或加入企业");
    };
    let rows = candidates::Entity::find()
        .filter(candidates::Column::EnterpriseId.eq(eu.enterprise_id))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let mut matches: Vec<Value> = rows
        .into_iter()
        .map(|c| {
            let score = keyword_score(&post, c.profile_data.as_deref().unwrap_or(""));
            json!({ "candidate": { "id": c.id, "name": c.name }, "match_score": score })
        })
        .collect();
    matches.sort_by(|a, b| {
        let sa = a["match_score"].as_f64().unwrap_or(0.0);
        let sb = b["match_score"].as_f64().unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    let total = matches.len();
    Json(json!({ "job_post": post_json(&post), "matches": matches, "total": total })).into_response()
}

fn keyword_score(post: &job_posts::Model, profile: &str) -> f64 {
    let blob = format!(
        "{} {}",
        post.requirements.clone().unwrap_or_default(),
        post.description.clone().unwrap_or_default()
    )
    .to_lowercase();
    let keywords: Vec<&str> = blob.split_whitespace().filter(|k| k.len() > 2).collect();
    if keywords.is_empty() {
        return 0.5;
    }
    let profile = profile.to_lowercase();
    let hits = keywords.iter().filter(|k| profile.contains(*k)).count();
    ((hits as f64) / (keywords.len() as f64)).min(1.0)
}

fn post_limit(tier: &str) -> usize {
    match tier {
        "pro" => 20,
        "enterprise" => usize::MAX,
        _ => 3,
    }
}

fn post_json(row: &job_posts::Model) -> Value {
    json!({
        "id": row.id,
        "user_id": row.user_id,
        "title": row.title,
        "company": row.company,
        "description": row.description,
        "requirements": row.requirements,
        "status": row.status,
    })
}

async fn supporter(state: &AppState, headers: &HeaderMap) -> Result<Principal, Response> {
    let principal = auth(state, headers, AuthMode::Required).await?;
    require_tier(&principal, "supporter")?;
    Ok(principal)
}

async fn owned_post(state: &AppState, id: &str, user_id: &str) -> Option<job_posts::Model> {
    job_posts::Entity::find_by_id(id.to_string())
        .filter(job_posts::Column::UserId.eq(user_id))
        .one(&state.db)
        .await
        .ok()
        .flatten()
}

async fn load_jobs(state: &AppState) -> Result<Vec<Value>, String> {
    let rows = documents::Entity::find()
        .filter(documents::Column::DocType.eq("job"))
        .filter(documents::Column::Status.eq("processed"))
        .order_by_desc(documents::Column::CreatedAt)
        .all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let meta: Value = serde_json::from_str(r.metadata.as_deref()?).ok()?;
            let mut job = meta.get("parsed").cloned().unwrap_or(meta);
            if job.get("id").is_none() {
                job["id"] = json!(r.id);
            }
            if job.get("title").is_none() {
                job["title"] = json!(r.title);
            }
            Some(job)
        })
        .collect())
}

async fn job_exists_doc(state: &AppState, job_id: &str) -> Result<bool, String> {
    let rows = documents::Entity::find()
        .filter(documents::Column::DocType.eq("job"))
        .all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().any(|r| {
        r.metadata
            .as_deref()
            .and_then(|m| serde_json::from_str::<Value>(m).ok())
            .map(|meta| {
                meta.get("job_id").and_then(|v| v.as_str()) == Some(job_id)
                    || meta.pointer("/parsed/id").and_then(|v| v.as_str()) == Some(job_id)
            })
            .unwrap_or(false)
    }))
}

async fn llm_extract(state: &AppState, _principal: &Principal, text: &str) -> Response {
    let Some(raw) = common::looma::call_llm(
        &state.settings,
        &format!("提取职位 JSON。文本：\n{}", text.chars().take(4000).collect::<String>()),
        0.1,
    )
    .await
    else {
        return fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "parse_failed",
            "AI 服务暂不可用（区域过滤后无可用模型，或上游失败）",
        );
    };
    match json_block(&raw) {
        Some(v) => Json(json!({ "parsed": v })).into_response(),
        None => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "parse_failed",
            "AI 返回了内容但未能解析为有效 JSON",
        ),
    }
}

fn json_block(raw: &str) -> Option<Value> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    serde_json::from_str(&raw[start..=end]).ok()
}
