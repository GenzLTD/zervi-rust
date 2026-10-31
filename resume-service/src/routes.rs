//! `/v1/resume/*` 十五个端点。身份、同意、配额、错误体对齐 looma。
//! 二进制简历转换（MarkItDown）不在本服务；PDF/DOCX 返回 `convert_failed`。
//! LLM 走 X1，过滤后无可用 provider 则 `parse_failed` / 分析字段为 null。
//!
//! 契约：looma-zervi backend/contracts/resume.v1.json v0.1.0。
//! templates 为全局只读目录（auth: optional，无 consent，非用户资源）——
//! 0.1 不提供写操作；用户自建模板须另开 /v1/resume/templates/custom 并升 minor。
//! blacklist 为用户维屏蔽规则集（auth: required + jobseeker_core，落 resume_blacklist
//! 表，非 documents；不占文档配额，随 users 级联删除）。
//!
//! detail/update 共用一个 `:resume_id` 路径（GET/PUT/DELETE 三种方法）：
//! · detail 只读，visibility 从 `metadata.permission.visibility` 读（缺省 private）；
//! · update 只写正文三字段（title/extracted/markdown），merge 语义，不碰
//!   visibility / status——visibility 的唯一写入口是 `/permission`，status 由
//!   入库链路派生，避免未被请求的字段被隐式改写。
//! · analysis / delete 同属 owner 简历数据面，均持 jobseeker_core 授权闸
//!   （2026-10-30 收口，对齐 detail/update/permission/blacklist）。
//!
//! `/v1/resume/:resume_id/permission`（GET/PUT）是授权面的唯一写入口，写的就是
//! `metadata.permission = {visibility, grants, updated_at}`——与 detail 读的是同一处，
//! 不另建表、不留第二条写路径。grants 为全量替换：PUT 省略 grants 等于清空。
//! expires_at 语义（2026-10-30）：过期的 grant 读响应不返回、业务判定按无授权处理；
//! GET 读到过期条目就地剔除回写、PUT 写前剔除（惰性清理），全表清扫交给后台
//! `sweep`（`RESUME_SWEEP_INTERVAL_SECS`，见 sweep.rs）——判定与写回复用本文件同两个函数。

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use common::entities::{documents, enterprises, resume_blacklist};
use common::looma::{
    consume_daily, fail, now_tz, require_consent, resolve_principal, AppState,
    AuthMode, Principal, Resource,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/resume/parse", post(parse_resume))
        .route("/v1/resume/upload", post(upload_resume))
        .route("/v1/resume/ingest", post(ingest_resume))
        .route("/v1/resume/improve", post(improve_resume))
        .route("/v1/resume/list", get(list_resumes))
        .route("/v1/resume/templates", get(list_templates))
        .route("/v1/resume/analysis", get(analysis_resume))
        // 用户维屏蔽企业：静态段 blacklist 必须在 :resume_id 之前（契约 path_design.literal_before_param）
        .route(
            "/v1/resume/blacklist",
            get(list_blacklist).post(add_blacklist),
        )
        .route(
            "/v1/resume/blacklist/:item_id",
            axum::routing::delete(remove_blacklist),
        )
        // 授权面：两段路径，与单段 :resume_id 不冲突；静态段优先于参数段仍成立。
        .route(
            "/v1/resume/:resume_id/permission",
            get(get_permission).put(put_permission),
        )
        // 三条方法共用 :resume_id；静态段已在上方注册，axum 优先精确匹配。
        .route(
            "/v1/resume/:resume_id",
            get(detail_resume)
                .put(update_resume)
                .delete(delete_resume),
        )
        .with_state(state)
}

async fn gate(
    state: &AppState,
    headers: &HeaderMap,
    mode: AuthMode,
    consent: Option<&str>,
) -> Result<Principal, Response> {
    let principal = resolve_principal(state, headers, mode).await?;
    if let Some(scope) = consent {
        require_consent(state, &principal, scope).await?;
    }
    Ok(principal)
}

fn owner_filter(user_id: &str) -> Condition {
    Condition::all()
        .add(documents::Column::DocType.eq("resume"))
        .add(sea_orm::sea_query::Expr::cust_with_values(
            "metadata::json->>'user_id' = $1",
            [user_id.to_string()],
        ))
}

#[derive(Deserialize)]
struct ParseBody {
    #[serde(default)]
    text: String,
}

async fn parse_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ParseBody>,
) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Optional, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "resume text required");
    }
    if let Err(r) = consume_daily(&state, &principal, Resource::ResumeParse).await {
        return r;
    }
    match analyze(&state, "resume", &text).await {
        Ok(extracted) => Json(json!({ "extracted": extracted })).into_response(),
        Err(r) => r,
    }
}

async fn upload_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Optional, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let mut filename = "resume.txt".to_string();
    let mut bytes = Vec::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("file") {
            if let Some(name) = field.file_name() {
                filename = name.to_string();
            }
            bytes = match field.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    return fail(StatusCode::BAD_REQUEST, "bad_request", e.to_string());
                }
            };
        }
    }
    if bytes.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "file required");
    }
    if bytes.starts_with(b"%PDF") || bytes.starts_with(b"PK") {
        return fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "convert_failed",
            "文档解析失败。本服务尚未接入 MarkItDown，请改传纯文本或走 /v1/resume/ingest",
        );
    }
    let markdown = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            return fail(
                StatusCode::UNPROCESSABLE_ENTITY,
                "convert_failed",
                "文档内容不是 UTF-8 文本",
            );
        }
    };
    if let Err(r) = consume_daily(&state, &principal, Resource::ResumeParse).await {
        return r;
    }
    let extracted = match analyze(&state, "resume", &markdown).await {
        Ok(v) => Some(v),
        Err(_) => None,
    };
    match insert_resume(
        &state,
        &principal,
        &filename,
        &markdown,
        extracted.clone(),
        if extracted.is_some() { "processed" } else { "uploaded" },
        None,
    )
    .await
    {
        Ok((id, _)) => Json(json!({
            "filename": filename,
            "resume_id": id,
            "status": if extracted.is_some() { "complete" } else { "partial" },
            "extracted": extracted,
        }))
        .into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize, Default)]
struct IngestBody {
    #[serde(default)]
    markdown: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    extracted: Option<Value>,
}

async fn ingest_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IngestBody>,
) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Optional, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let markdown = body.markdown.trim().to_string();
    if markdown.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "markdown is required");
    }
    let filename = if body.filename.trim().is_empty() {
        "resume_ingest.txt".into()
    } else {
        body.filename
    };
    let api_status = if body.extracted.is_some() { "processed" } else { "stored" };
    let doc_status = if body.extracted.is_some() { "processed" } else { "uploaded" };
    let fp = resume_content_fp(&principal.user_id, &markdown);
    if let Some(id) = find_resume_id_by_fp(&state, &principal.user_id, &fp).await {
        return Json(json!({
            "resume_id": id,
            "status": api_status,
            "matches": [],
            "total_evaluated": 0,
            "match_status": "skipped_duplicate",
            "match_error": "同一份简历已入库，未重复匹配、未再扣配额。可点「计算匹配」。",
        }))
        .into_response();
    }
    match insert_resume(
        &state,
        &principal,
        &filename,
        &markdown,
        body.extracted,
        doc_status,
        Some(&fp),
    )
    .await
    {
        Ok((id, _)) => {
            let matched = fetch_library_matches(&headers, &markdown).await;
            Json(json!({
                "resume_id": id,
                "status": api_status,
                "matches": matched.get("matches").cloned().unwrap_or(json!([])),
                "total_evaluated": matched.get("total_evaluated").cloned().unwrap_or(json!(0)),
                "match_status": matched.get("match_status").cloned().unwrap_or(json!("ok")),
                "match_error": matched.get("match_error").cloned().unwrap_or(json!(null)),
            }))
            .into_response()
        }
        Err(r) => r,
    }
}

#[derive(Deserialize, Default)]
struct ImproveBody {
    #[serde(default)]
    resume_text: String,
    #[serde(default)]
    extracted: Option<Value>,
    #[serde(default)]
    focus: String,
}

async fn improve_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ImproveBody>,
) -> Response {
    let _principal = match gate(&state, &headers, AuthMode::Optional, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let resume_text = body.resume_text.trim().to_string();
    if resume_text.is_empty() && body.extracted.is_none() {
        return fail(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "请提供简历文本(resume_text)或已解析数据(extracted)",
        );
    }
    let text: String = if !resume_text.is_empty() {
        resume_text.chars().take(4000).collect()
    } else {
        body.extracted.unwrap().to_string().chars().take(4000).collect()
    };
    let focus = body.focus.trim().to_lowercase();
    let focus_prompt = match focus.as_str() {
        "skills" => "请重点分析技能描述是否充分、与目标岗位的匹配度、技能分类是否合理",
        "experience" => "请重点分析工作/项目经历的写法是否突出了成果和量化指标，描述是否具体",
        "education" => "请重点分析教育背景的呈现方式、与岗位要求的关联度",
        _ => "请从整体结构、内容完整度、关键信息呈现、语言表达等方面给出建议",
    };
    let prompt = format!(
        "你是一位资深HR和简历优化专家。请仔细分析以下简历内容，给出具体可操作的改进建议。\n\n分析维度：{focus_prompt}\n\n请只输出 JSON，字段 overall_score, strengths, weaknesses, suggestions。\n\n{text}"
    );
    match common::looma::call_llm(&state.settings, &prompt, 0.3).await {
        Some(raw) => match parse_json_block(&raw) {
            Some(improvements) => Json(json!({ "improvements": improvements })).into_response(),
            None => fail(StatusCode::INTERNAL_SERVER_ERROR, "improve_failed", "AI 返回了内容但未能解析为有效 JSON"),
        },
        None => fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "improve_failed",
            "AI 服务暂不可用（区域过滤后无可用模型，或上游失败）",
        ),
    }
}

async fn list_resumes(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Required, None).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let rows = match documents::Entity::find()
        .filter(owner_filter(&principal.user_id))
        .order_by_desc(documents::Column::CreatedAt)
        .limit(50)
        .all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(_) => {
            return Json(json!({ "resumes": [], "total": 0 })).into_response();
        }
    };
    let resumes: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let meta = parse_meta(r.metadata.as_deref());
            json!({
                "id": r.id.to_string(),
                "title": if r.title.is_empty() { "未命名简历".into() } else { r.title.clone() },
                "filename": if r.title.is_empty() { r.file_path.clone() } else { r.title },
                "file_size": r.file_size,
                "uploaded_at": r.created_at.to_rfc3339(),
                "extracted": meta.get("extracted").cloned().unwrap_or(json!({})),
            })
        })
        .collect();
    let total = resumes.len();
    Json(json!({ "resumes": resumes, "total": total })).into_response()
}

/// 全局模板目录（resume.v1.json v0.1.0 · templates_list）。
///
/// 静态目录，非用户资源，不入库：0.1 只读且无个性化排序。
/// `preview_fields` 取值对齐 ParsedResume 字段名（name/education/experience/skills/expectation）。
/// 若未来按 identity.value_axes 排序推荐，另在 gfcc 声明 optional adapter（must_not_write）。
async fn list_templates(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // 契约 auth: optional、无 consent：游客可浏览目录，登录不写库、不建 users 行
    // （Auto 建档对目录浏览没有意义，但保持与 upload/ingest 同一入口语义最简单）。
    if let Err(r) = gate(&state, &headers, AuthMode::Optional, None).await {
        return r;
    }
    let templates = template_catalog();
    let total = templates.len();
    Json(json!({ "templates": templates, "total": total })).into_response()
}

fn template_catalog() -> Vec<Value> {
    vec![
        json!({
            "id": "standard-zh",
            "name": "标准求职版",
            "locale": "zh-CN",
            "summary": "信息完整、结构均衡，适合大多数社招投递场景。",
            "tags": ["通用", "社招"],
            "preview_fields": ["name", "education", "experience", "skills", "expectation"],
        }),
        json!({
            "id": "tech-zh",
            "name": "技术专员版",
            "locale": "zh-CN",
            "summary": "突出技能栈与项目亮点，弱化时间线，适合技术岗。",
            "tags": ["技术", "项目导向"],
            "preview_fields": ["name", "skills", "experience", "expectation"],
        }),
        json!({
            "id": "fresh-zh",
            "name": "应届生版",
            "locale": "zh-CN",
            "summary": "教育背景前置，突出校园经历与实习成果。",
            "tags": ["应届", "校招"],
            "preview_fields": ["name", "education", "experience", "skills"],
        }),
        json!({
            "id": "onepage-zh",
            "name": "简约一页版",
            "locale": "zh-CN",
            "summary": "一页纸原则，只留最强信号，适合快速筛选场景。",
            "tags": ["一页", "高效"],
            "preview_fields": ["name", "experience", "skills", "expectation"],
        }),
        json!({
            "id": "standard-en",
            "name": "Standard Professional",
            "locale": "en-US",
            "summary": "Balanced structure for general job applications.",
            "tags": ["general"],
            "preview_fields": ["name", "education", "experience", "skills", "expectation"],
        }),
        json!({
            "id": "tech-en",
            "name": "Engineering Focused",
            "locale": "en-US",
            "summary": "Skills-first layout highlighting projects and impact.",
            "tags": ["engineering"],
            "preview_fields": ["name", "skills", "experience", "expectation"],
        }),
    ]
}

// ── 屏蔽企业（用户维，非简历资源）──────────────────────────────────────
//
// 契约：blacklist_list / blacklist_add / blacklist_remove，dimension=user。
// 表：resume_blacklist（shared/resume_blacklist_v0.1.sql）。
// 与 documents 解耦：屏蔽是「匹配规则集」，命中即从推荐/投递侧过滤，
// 不占 ResumeParse 配额（Resource 只有 ResumeParse），随 users 级联删除。

const BLACKLIST_MATCHES: [&str; 3] = ["exact_name", "credit_code", "enterprise_id"];
/// 0.1 不分页（契约无 cursor）；上限兜底防单用户堆积，total 仍报真实总数。
const BLACKLIST_LIST_LIMIT: u64 = 500;

fn blacklist_item_json(row: &resume_blacklist::Model) -> Value {
    let mut item = json!({
        "id": row.id,
        "company_name": row.company_name,
        "match": row.match_kind,
        "created_at": row.created_at.to_rfc3339(),
    });
    // 可选字段只在有值时出现：契约里它们是 type:string（不是 string|null），
    // 塞 null 会被严格校验器判违约。
    if let Some(v) = &row.credit_code {
        item["credit_code"] = json!(v);
    }
    if let Some(v) = &row.enterprise_id {
        item["enterprise_id"] = json!(v);
    }
    if let Some(v) = &row.note {
        item["note"] = json!(v);
    }
    item
}

async fn list_blacklist(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Required, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let total = match resume_blacklist::Entity::find()
        .filter(resume_blacklist::Column::UserId.eq(principal.user_id.clone()))
        .count(&state.db)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("读取屏蔽列表失败: {e}"),
            )
        }
    };
    let rows = match resume_blacklist::Entity::find()
        .filter(resume_blacklist::Column::UserId.eq(principal.user_id.clone()))
        .order_by_desc(resume_blacklist::Column::CreatedAt)
        .limit(BLACKLIST_LIST_LIMIT)
        .all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("读取屏蔽列表失败: {e}"),
            )
        }
    };
    Json(json!({
        "items": rows.iter().map(blacklist_item_json).collect::<Vec<_>>(),
        "total": total,
    }))
    .into_response()
}

#[derive(Deserialize, Default)]
struct BlacklistAddBody {
    /// 契约字段名 `match`（Rust 关键字，故字段叫 match_kind）。
    #[serde(rename = "match", default)]
    match_kind: String,
    #[serde(default)]
    company_name: String,
    #[serde(default)]
    credit_code: String,
    #[serde(default)]
    enterprise_id: String,
    #[serde(default)]
    note: String,
}

async fn add_blacklist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BlacklistAddBody>,
) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Required, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let match_kind = body.match_kind.trim().to_lowercase();
    if !BLACKLIST_MATCHES.contains(&match_kind.as_str()) {
        return fail(
            StatusCode::BAD_REQUEST,
            "bad_request",
            format!(
                "match 非法：{}（只接受 exact_name|credit_code|enterprise_id）",
                body.match_kind
            ),
        );
    }
    let mut company_name = body.company_name.trim().to_string();
    // 统一社会信用代码入库前归一为大写：唯一索引按 upper() 建表达式，
    // 不对齐的话同码大小写两版会各插一条。
    let credit_code = body.credit_code.trim().to_uppercase();
    let enterprise_id = body.enterprise_id.trim().to_string();

    // 分情况必填（契约 body）。DB 的 CHECK 是第二道闸，这里先给可读的 400。
    let missing = match match_kind.as_str() {
        "exact_name" if company_name.is_empty() => Some("match=exact_name 需提供 company_name"),
        "credit_code" if credit_code.is_empty() => Some("match=credit_code 需提供 credit_code"),
        "enterprise_id" if enterprise_id.is_empty() => {
            Some("match=enterprise_id 需提供 enterprise_id")
        }
        _ => None,
    };
    if let Some(msg) = missing {
        return fail(StatusCode::BAD_REQUEST, "bad_request", msg);
    }

    // 只写与 match 对应的那一列，其余置空：三条部分唯一索引按维度分桶，
    // 多留一列会让「同名但不同维度」的语义变糊。
    let (credit_code_db, enterprise_id_db) = match match_kind.as_str() {
        "credit_code" => (Some(credit_code.clone()), None),
        "enterprise_id" => (None, Some(enterprise_id.clone())),
        _ => (None, None),
    };

    // 客户端在非 exact_name 形态下只传匹配键（见 miniprogram blacklist 页），
    // company_name 缺省时 best-effort 从 enterprises 补展示名；补不到就留空串。
    if company_name.is_empty() && match_kind == "enterprise_id" {
        if let Ok(Some(ent)) = enterprises::Entity::find_by_id(enterprise_id.clone())
            .one(&state.db)
            .await
        {
            company_name = ent.name;
        }
    }

    // 幂等添加：同用户 + 同维度 + 同取值已存在就回既有行（200），
    // 前端「添加 → 重拉列表」不需要区分 409；并发由唯一索引兜底（撞了回查）。
    if let Some(existing) = find_blacklist_duplicate(
        &state,
        &principal,
        &match_kind,
        &company_name,
        &credit_code,
        &enterprise_id,
    )
    .await
    {
        return Json(blacklist_item_json(&existing)).into_response();
    }

    let note = body.note.trim().to_string();
    let model = resume_blacklist::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        user_id: Set(principal.user_id.clone()),
        match_kind: Set(match_kind.clone()),
        company_name: Set(company_name.clone()),
        credit_code: Set(credit_code_db),
        enterprise_id: Set(enterprise_id_db),
        note: Set(if note.is_empty() { None } else { Some(note) }),
        // DB DEFAULT now()：NotSet 不进 INSERT 列清单
        created_at: sea_orm::ActiveValue::NotSet,
    };
    match model.insert(&state.db).await {
        Ok(row) => Json(blacklist_item_json(&row)).into_response(),
        Err(e) => {
            // 唯一索引竞态（另一请求刚插了同一条）→ 回查后按幂等返回
            if let Some(existing) = find_blacklist_duplicate(
                &state,
                &principal,
                &match_kind,
                &company_name,
                &credit_code,
                &enterprise_id,
            )
            .await
            {
                return Json(blacklist_item_json(&existing)).into_response();
            }
            fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                format!("写入屏蔽列表失败: {e}"),
            )
        }
    }
}

async fn remove_blacklist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Required, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let id = item_id.trim();
    if id.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "item_id required");
    }
    // 双条件删除：id 命中但 user_id 不符一律走 404，
    // 不暴露「这条 id 存在、只是不属于你」。
    match resume_blacklist::Entity::delete_many()
        .filter(resume_blacklist::Column::Id.eq(id))
        .filter(resume_blacklist::Column::UserId.eq(principal.user_id.clone()))
        .exec(&state.db)
        .await
    {
        Ok(res) if res.rows_affected > 0 => {
            Json(json!({ "message": "removed", "id": id })).into_response()
        }
        Ok(_) => fail(StatusCode::NOT_FOUND, "not_found", "屏蔽项不存在"),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            format!("删除屏蔽项失败: {e}"),
        ),
    }
}

/// 查重：与 DDL 的三条部分唯一索引同口径（名称 lower+btrim、信用代码 upper+btrim、企业 ID btrim）。
/// 应用层先查只为「幂等返回既有行」，最终裁决仍在索引。
async fn find_blacklist_duplicate(
    state: &AppState,
    principal: &Principal,
    match_kind: &str,
    company_name: &str,
    credit_code: &str,
    enterprise_id: &str,
) -> Option<resume_blacklist::Model> {
    use sea_orm::sea_query::Expr;
    let mut cond = Condition::all()
        .add(resume_blacklist::Column::UserId.eq(&principal.user_id))
        .add(resume_blacklist::Column::MatchKind.eq(match_kind));
    cond = match match_kind {
        "credit_code" => cond.add(Expr::cust_with_values(
            "upper(btrim(credit_code)) = upper(btrim($1))",
            [credit_code.to_string()],
        )),
        "enterprise_id" => cond.add(Expr::cust_with_values(
            "btrim(enterprise_id) = btrim($1)",
            [enterprise_id.to_string()],
        )),
        _ => cond.add(Expr::cust_with_values(
            "lower(btrim(company_name)) = lower(btrim($1))",
            [company_name.to_string()],
        )),
    };
    resume_blacklist::Entity::find()
        .filter(cond)
        .one(&state.db)
        .await
        .ok()
        .flatten()
}

#[derive(Deserialize)]
struct AnalysisQuery {
    #[serde(default)]
    resume_id: String,
}

async fn analysis_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AnalysisQuery>,
) -> Response {
    // 契约一致性收口：analysis 读 owner 简历正文（同 detail），须持 jobseeker_core 授权。
    // 未授权 → 403 consent_required（consent.rs 统一口径，dev 旁路 ZERVI_DEV_BYPASS_CONSENT=1）。
    let principal = match gate(&state, &headers, AuthMode::Required, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let resume_id = q.resume_id.trim();
    if resume_id.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "resume_id required");
    }
    let Ok(id) = resume_id.parse::<i64>() else {
        return fail(StatusCode::NOT_FOUND, "not_found", "简历不存在");
    };
    let row = documents::Entity::find_by_id(id)
        .filter(owner_filter(&principal.user_id))
        .one(&state.db)
        .await;
    let row = match row {
        Ok(Some(row)) => row,
        _ => return fail(StatusCode::NOT_FOUND, "not_found", "简历不存在"),
    };
    let meta = parse_meta(row.metadata.as_deref());
    let extracted = meta.get("extracted").cloned().unwrap_or(json!({}));
    let markdown = meta
        .get("markdown")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let analysis = if markdown.is_empty() {
        None
    } else {
        let prompt = format!("请用 JSON 评价这份简历，字段 strengths, weaknesses, suggestions, summary。\n{markdown}");
        common::looma::call_llm(&state.settings, &prompt, 0.3)
            .await
            .and_then(|raw| parse_json_block(&raw))
    };
    Json(json!({
        "resume_id": row.id.to_string(),
        "title": row.title,
        "extracted": extracted,
        "markdown": markdown,
        "analysis": analysis,
    }))
    .into_response()
}

/// visibility 的合法取值（契约 enums.ResumeVisibility）。
const VISIBILITY_VALUES: [&str; 3] = ["everyone", "trusted", "private"];

/// 契约里 detail.status 只允许 uploaded/processed/partial。
/// documents.status 是自由文本，未知值一律按 uploaded 呈现，避免客户端拿到枚举外的值。
fn resume_status(raw: Option<&str>) -> &'static str {
    match raw {
        Some("processed") => "processed",
        Some("partial") => "partial",
        _ => "uploaded",
    }
}

/// 未设置权限时回落 private：最小权限，避免默认把简历暴露给 trusted/everyone。
fn resume_visibility(meta: &Value) -> String {
    match meta
        .get("permission")
        .and_then(|p| p.get("visibility"))
        .and_then(|v| v.as_str())
    {
        Some(v) if VISIBILITY_VALUES.contains(&v) => v.to_string(),
        _ => "private".to_string(),
    }
}

/// detail 与 update 共用同一份响应体，保证两个端点字段口径一致。
fn detail_json(row: &documents::Model) -> Value {
    let meta = parse_meta(row.metadata.as_deref());
    let meta_filename = meta
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let filename = if !meta_filename.is_empty() {
        meta_filename.clone()
    } else if !row.title.is_empty() {
        row.title.clone()
    } else {
        row.file_path.clone()
    };
    let title = if !row.title.is_empty() {
        row.title.clone()
    } else if !meta_filename.is_empty() {
        meta_filename
    } else {
        "未命名简历".to_string()
    };
    let mut body = json!({
        "id": row.id.to_string(),
        "title": title,
        "filename": filename,
        "uploaded_at": row.created_at.to_rfc3339(),
        "extracted": meta.get("extracted").cloned().unwrap_or(json!({})),
        "markdown": meta.get("markdown").and_then(|v| v.as_str()).unwrap_or(""),
        "visibility": resume_visibility(&meta),
        "status": resume_status(row.status.as_deref()),
    });
    // 契约里 file_size 是 integer（非 null）；没有就省略，不塞 null。
    if let Some(size) = row.file_size {
        body["file_size"] = json!(size);
    }
    body
}

/// 授权面（`/permission`）的取值域与上限。
const SUBJECT_TYPES: [&str; 3] = ["user", "enterprise", "share_code_ref"];
const PERMISSION_SCOPES: [&str; 2] = ["read", "download"];
const GRANTS_MAX: usize = 200;
const SUBJECT_ID_MAX_CHARS: usize = 200;

/// 单条 grant 归一化：只保留契约声明的四个键，顺序固定，`expires_at` 缺失补 null。
/// 返回 Err 用于 400 文案；读路径用 .ok() 过滤脏数据，不因历史脏行把整份授权面打成 500。
fn normalize_grant(raw: &Value) -> Result<Value, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "grants 的每一项必须是对象".to_string())?;
    let subject_type = obj
        .get("subject_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !SUBJECT_TYPES.contains(&subject_type) {
        return Err(format!("subject_type 非法：{subject_type}"));
    }
    let subject_id = obj
        .get("subject_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if subject_id.is_empty() {
        return Err("subject_id 不能为空".to_string());
    }
    if subject_id.chars().count() > SUBJECT_ID_MAX_CHARS {
        return Err(format!("subject_id 超过 {SUBJECT_ID_MAX_CHARS} 字符"));
    }
    let scope = obj.get("scope").and_then(|v| v.as_str()).unwrap_or("");
    if !PERMISSION_SCOPES.contains(&scope) {
        return Err(format!("scope 非法：{scope}"));
    }
    let expires_at = match obj.get("expires_at") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) => {
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|_| format!("expires_at 不是 RFC3339：{s}"))?;
            json!(s)
        }
        Some(_) => return Err("expires_at 必须是 RFC3339 字符串或 null".to_string()),
    };
    Ok(json!({
        "subject_type": subject_type,
        "subject_id": subject_id,
        "scope": scope,
        "expires_at": expires_at,
    }))
}

/// PUT 的 grants 校验：必须是数组、不超过上限、条目合法、同一 (主语, 权限) 不重复。
/// 不去重也不静默丢弃——重复条目说明客户端把「全量替换」当增量合并了，直接报错更有用。
fn parse_grants(raw: &Value) -> Result<Vec<Value>, String> {
    let arr = raw
        .as_array()
        .ok_or_else(|| "grants 必须是数组".to_string())?;
    if arr.len() > GRANTS_MAX {
        return Err(format!("grants 超过 {GRANTS_MAX} 条"));
    }
    let mut out = Vec::with_capacity(arr.len());
    let mut seen = std::collections::HashSet::new();
    for item in arr {
        let grant = normalize_grant(item)?;
        let key = format!(
            "{}|{}|{}",
            grant["subject_type"].as_str().unwrap_or(""),
            grant["subject_id"].as_str().unwrap_or(""),
            grant["scope"].as_str().unwrap_or("")
        );
        if !seen.insert(key.clone()) {
            return Err(format!("grants 存在重复条目：{key}"));
        }
        out.push(grant);
    }
    Ok(out)
}

/// GET/PUT 共用响应体：`updated_at` 只有真写过授权面才会出现（契约里它是可选字段）。
fn permission_json(resume_id: i64, meta: &Value) -> Value {
    let grants: Vec<Value> = meta
        .get("permission")
        .and_then(|p| p.get("grants"))
        .and_then(|g| g.as_array())
        .map(|arr| arr.iter().filter_map(|v| normalize_grant(v).ok()).collect())
        .unwrap_or_default();
    let mut body = json!({
        "resume_id": resume_id.to_string(),
        "visibility": resume_visibility(meta),
        "grants": grants,
    });
    if let Some(ts) = meta
        .get("permission")
        .and_then(|p| p.get("updated_at"))
        .and_then(|v| v.as_str())
    {
        body["updated_at"] = json!(ts);
    }
    body
}

/// 过期 grant 判定：expires_at 非空且 <= now → 过期。
/// 缺失/null 解析不出 → 未过期（读路径对历史脏行不放大成错误，与 normalize_grant .ok() 口径一致）。
fn grant_expired(grant: &Value, now: chrono::DateTime<chrono::FixedOffset>) -> bool {
    grant
        .get("expires_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|exp| exp <= now)
        .unwrap_or(false)
}

/// 从 metadata.permission.grants 拆（未过期, 已过期）。
/// 归一化失败的脏行：不进 live 也不进 expired（permission_json 同口径直接过滤掉）。
/// `pub(crate)`：后台清扫 `sweep` 必须用同一套判定，不许另写一份。
pub(crate) fn partition_grants(meta: &Value, now: chrono::DateTime<chrono::FixedOffset>) -> (Vec<Value>, Vec<Value>) {
    let mut live = Vec::new();
    let mut expired = Vec::new();
    let grants = meta
        .get("permission")
        .and_then(|p| p.get("grants"))
        .and_then(|g| g.as_array())
        .map(|arr| arr.iter().filter_map(|v| normalize_grant(v).ok()).collect::<Vec<_>>())
        .unwrap_or_default();
    for g in grants {
        if grant_expired(&g, now) {
            expired.push(g);
        } else {
            live.push(g);
        }
    }
    (live, expired)
}

/// `:resume_id` 解析失败与「不是本人简历」都回 404：
/// 不区分「不存在」与「无权限」，避免探测他人简历是否存在。
fn parse_resume_id(raw: &str) -> Result<i64, Response> {
    raw.trim()
        .parse::<i64>()
        .map_err(|_| fail(StatusCode::NOT_FOUND, "not_found", "简历不存在"))
}

async fn find_owned_resume(
    state: &AppState,
    user_id: &str,
    id: i64,
) -> Result<documents::Model, Response> {
    documents::Entity::find_by_id(id)
        .filter(owner_filter(user_id))
        .one(&state.db)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "server_error", e.to_string()))?
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "not_found", "简历不存在"))
}

async fn detail_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(resume_id): Path<String>,
) -> Response {
    let principal = match gate(
        &state,
        &headers,
        AuthMode::Required,
        Some("jobseeker_core"),
    )
    .await
    {
        Ok(p) => p,
        Err(r) => return r,
    };
    let id = match parse_resume_id(&resume_id) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match find_owned_resume(&state, &principal.user_id, id).await {
        Ok(row) => Json(detail_json(&row)).into_response(),
        Err(r) => r,
    }
}

/// PUT body：三个字段全可选。`Option<T>` 下显式 null 与「未传」等价，
/// 因此无法用 null 清空字段——契约里这三个字段都是必填型 string/object，本就不该被清空。
#[derive(Deserialize)]
struct UpdateBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    extracted: Option<Value>,
    #[serde(default)]
    markdown: Option<String>,
}

const TITLE_MAX_CHARS: usize = 200;

async fn update_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(resume_id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Response {
    let principal = match gate(
        &state,
        &headers,
        AuthMode::Required,
        Some("jobseeker_core"),
    )
    .await
    {
        Ok(p) => p,
        Err(r) => return r,
    };
    if body.title.is_none() && body.extracted.is_none() && body.markdown.is_none() {
        return fail(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "至少提供 title / extracted / markdown 之一",
        );
    }
    let next_title = match body.title {
        Some(raw) => {
            let t = raw.trim().to_string();
            if t.is_empty() {
                return fail(StatusCode::BAD_REQUEST, "bad_request", "title 不能为空");
            }
            if t.chars().count() > TITLE_MAX_CHARS {
                return fail(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    format!("title 超过 {TITLE_MAX_CHARS} 字符"),
                );
            }
            Some(t)
        }
        None => None,
    };
    if let Some(v) = &body.extracted {
        if !v.is_object() {
            return fail(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "extracted 必须是对象",
            );
        }
    }
    let id = match parse_resume_id(&resume_id) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let row = match find_owned_resume(&state, &principal.user_id, id).await {
        Ok(row) => row,
        Err(r) => return r,
    };
    // merge 而不是重建：metadata 里还压着 user_id（owner 归属）与 filename，
    // 整体覆盖会直接让这份简历失去归属、从 list/detail 里消失。
    let mut meta = parse_meta(row.metadata.as_deref());
    let mut next_size = row.file_size;
    if let Some(markdown) = &body.markdown {
        meta["markdown"] = json!(markdown);
        // 入库时 file_size 记的就是 markdown 字节长（见 insert_resume），
        // 正文改写后同步，否则 list 里的体积会一直是旧值。
        next_size = Some(markdown.len() as i32);
    }
    if let Some(extracted) = &body.extracted {
        meta["extracted"] = extracted.clone();
    }
    match persist_document(&state, row, meta, next_title, next_size).await {
        Ok(updated) => Json(detail_json(&updated)).into_response(),
        Err(r) => r,
    }
}

/// 正文与授权面共用同一条写回路径：只改 metadata（+ 可选 title/file_size），
/// 其余列按原值回写，避免两处各写一套 ActiveModel 后漂移。
/// `pub(crate)`：后台清扫走同一条写回，保证 `updated_at` 语义与端点一致。
pub(crate) async fn persist_document(
    state: &AppState,
    row: documents::Model,
    meta: Value,
    next_title: Option<String>,
    next_size: Option<i32>,
) -> Result<documents::Model, Response> {
    let active = documents::ActiveModel {
        id: Set(row.id),
        title: Set(next_title.unwrap_or_else(|| row.title.clone())),
        file_path: Set(row.file_path.clone()),
        doc_type: Set(row.doc_type.clone()),
        file_size: Set(next_size),
        metadata: Set(Some(meta.to_string())),
        status: Set(row.status.clone()),
        chunk_count: Set(row.chunk_count),
        created_at: Set(row.created_at),
        processed_at: Set(row.processed_at),
    };
    active
        .update(&state.db)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, "update_failed", e.to_string()))
}

/// 仅 owner 可读完整授权面（契约 permission_semantics.note）：
/// trusted/everyone 访客不走上游端点，走业务读路径按 visibility 过滤。
async fn get_permission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(resume_id): Path<String>,
) -> Response {
    let principal = match gate(
        &state,
        &headers,
        AuthMode::Required,
        Some("jobseeker_core"),
    )
    .await
    {
        Ok(p) => p,
        Err(r) => return r,
    };
    let id = match parse_resume_id(&resume_id) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match find_owned_resume(&state, &principal.user_id, id).await {
        Ok(row) => {
            let mut meta = parse_meta(row.metadata.as_deref());
            let (live, expired) = partition_grants(&meta, now_tz());
            if expired.is_empty() {
                return Json(permission_json(row.id, &meta)).into_response();
            }
            // 惰性清理：读到过期条目就地剔除并回写。授权面确被本请求改写，
            // updated_at 服务端重置（契约 write_semantics.updated_at 同口径）。
            let next_size = row.file_size;
            meta["permission"]["grants"] = json!(live);
            meta["permission"]["updated_at"] = json!(now_tz().to_rfc3339());
            match persist_document(&state, row, meta, None, next_size).await {
                Ok(updated) => Json(permission_json(
                    updated.id,
                    &parse_meta(updated.metadata.as_deref()),
                ))
                .into_response(),
                Err(r) => r,
            }
        }
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct PermissionBody {
    #[serde(default)]
    visibility: Option<String>,
    /// 契约 full_replacement：省略 = []（清空授权），不是「保持原值」。
    /// 客户端必须先 GET 再 PUT，把 PUT 当增量合并会静默清掉既有 grants。
    #[serde(default)]
    grants: Option<Value>,
}

async fn put_permission(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(resume_id): Path<String>,
    Json(body): Json<PermissionBody>,
) -> Response {
    let principal = match gate(
        &state,
        &headers,
        AuthMode::Required,
        Some("jobseeker_core"),
    )
    .await
    {
        Ok(p) => p,
        Err(r) => return r,
    };
    let visibility = match body.visibility.as_deref() {
        Some(v) if VISIBILITY_VALUES.contains(&v) => v.to_string(),
        Some(v) => return fail(StatusCode::BAD_REQUEST, "bad_request", format!("visibility 非法：{v}")),
        None => return fail(StatusCode::BAD_REQUEST, "bad_request", "visibility 必填"),
    };
    let grants = match &body.grants {
        None => Vec::new(),
        Some(raw) => match parse_grants(raw) {
            Ok(g) => g,
            Err(msg) => return fail(StatusCode::BAD_REQUEST, "bad_request", msg),
        },
    };
    // 写前惰性清理：校验（含去重）后剔除已过期条目，不让过期授权借 PUT 复活。
    // 归一化在 parse_grants 已做，这里只按 expires_at 过滤。
    let now = now_tz();
    let grants: Vec<Value> = grants.into_iter().filter(|g| !grant_expired(g, now)).collect();
    let id = match parse_resume_id(&resume_id) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let row = match find_owned_resume(&state, &principal.user_id, id).await {
        Ok(row) => row,
        Err(r) => return r,
    };
    // 只替换 permission 子树，正文/归属/文件名原样保留（与 update 同一条写回路径）。
    let mut meta = parse_meta(row.metadata.as_deref());
    meta["permission"] = json!({
        "visibility": visibility,
        "grants": grants,
        // 服务端生成；客户端传入一律忽略（契约 write_semantics.updated_at）
        "updated_at": now_tz().to_rfc3339(),
    });
    let next_size = row.file_size;
    match persist_document(&state, row, meta, None, next_size).await {
        Ok(updated) => Json(permission_json(
            updated.id,
            &parse_meta(updated.metadata.as_deref()),
        ))
        .into_response(),
        Err(r) => r,
    }
}

async fn delete_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(resume_id): Path<String>,
) -> Response {
    // 契约一致性收口：delete 删 owner 简历（破坏性写），须持 jobseeker_core 授权。
    // 未授权 → 403 consent_required（consent.rs 统一口径，dev 旁路 ZERVI_DEV_BYPASS_CONSENT=1）。
    let principal = match gate(&state, &headers, AuthMode::Required, Some("jobseeker_core")).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Ok(id) = resume_id.parse::<i64>() else {
        return fail(StatusCode::NOT_FOUND, "not_found", "简历不存在");
    };
    let result = documents::Entity::delete_many()
        .filter(documents::Column::Id.eq(id))
        .filter(owner_filter(&principal.user_id))
        .exec(&state.db)
        .await;
    match result {
        Ok(res) if res.rows_affected > 0 => {
            Json(json!({ "message": "deleted", "resume_id": resume_id })).into_response()
        }
        Ok(_) => fail(StatusCode::NOT_FOUND, "not_found", "简历不存在"),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "delete_failed", e.to_string()),
    }
}

async fn insert_resume(
    state: &AppState,
    principal: &Principal,
    filename: &str,
    markdown: &str,
    extracted: Option<Value>,
    doc_status: &str,
    content_fp: Option<&str>,
) -> Result<(String, String), Response> {
    let owner = if principal.guest {
        principal.user_id.clone()
    } else {
        principal.user_id.clone()
    };
    let mut meta = json!({
        "markdown": markdown,
        "user_id": owner,
        "filename": filename,
    });
    if let Some(extracted) = extracted {
        meta["extracted"] = extracted;
    }
    if let Some(fp) = content_fp {
        meta["content_fp"] = json!(fp);
    }
    let provisional = format!("resume/{owner}/{}", uuid::Uuid::new_v4().simple());
    let model = documents::ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        title: Set(filename.to_string()),
        file_path: Set(provisional),
        doc_type: Set("resume".into()),
        file_size: Set(Some(markdown.len() as i32)),
        metadata: Set(Some(meta.to_string())),
        status: Set(Some(doc_status.into())),
        chunk_count: Set(None),
        created_at: Set(now_tz()),
        processed_at: Set(None),
    };
    let inserted = model.insert(&state.db).await.map_err(|e| {
        fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            format!("入库失败: {e}"),
        )
    })?;
    let final_path = format!("resume/{owner}/{}_{filename}", inserted.id);
    let mut active: documents::ActiveModel = inserted.clone().into();
    active.file_path = Set(final_path.clone());
    active.update(&state.db).await.map_err(|e| {
        fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            format!("入库失败: {e}"),
        )
    })?;
    Ok((inserted.id.to_string(), final_path))
}

/// 简历解析的字段契约。
///
/// 之前的提示词只说「提取为 JSON 对象」，等于没有契约，于是模型各自发挥：
/// · qwen2.5-coder:1.5b → 把「蚂蚁集团支付网关负责人」整段当成公司名，还把期望职位当成现任职位
/// · qwen2.5:7b         → 改用中文键名（姓名 / 工作经验 / 教育背景），下游拿不到稳定字段
/// 现在显式给出英文键名与类型，并要求缺失字段留空值。
fn build_extract_prompt(doc_type: &str, text: &str) -> String {
    if doc_type.eq_ignore_ascii_case("resume") {
        format!(
            "你是简历解析引擎。从下面的简历文本中提取结构化信息。\n\
             严格要求：\n\
             1. 只输出一个 JSON 对象，不要任何解释文字，不要 markdown 代码块标记\n\
             2. 必须使用下列英文键名，不得改名，不得翻译成中文\n\
             3. 找不到的字段留空字符串、空数组或 0\n\n\
             输出结构：\n\
             {{\"name\":\"\",\"gender\":\"\",\"age\":0,\"phone\":\"\",\"email\":\"\",\
             \"education\":[{{\"school\":\"\",\"major\":\"\",\"degree\":\"\",\"period\":\"\"}}],\
             \"experience\":[{{\"company\":\"\",\"position\":\"\",\"period\":\"\",\"skills\":[],\"highlights\":[]}}],\
             \"skills\":[],\
             \"expectation\":{{\"position\":\"\",\"city\":\"\",\"salary\":\"\"}}}}\n\n\
             简历文本：\n{text}"
        )
    } else {
        format!("把以下{doc_type}文本提取为 JSON 对象，只输出 JSON。文本：\n{text}")
    }
}

async fn analyze(state: &AppState, doc_type: &str, text: &str) -> Result<Value, Response> {
    let clipped: String = text.chars().take(4000).collect();
    let prompt = build_extract_prompt(doc_type, &clipped);
    let Some(raw) = common::looma::call_llm(&state.settings, &prompt, 0.1).await else {
        return Err(fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "parse_failed",
            "AI 服务暂不可用（模型未配置、区域过滤后无候选，或上游失败）",
        ));
    };
    parse_json_block(&raw).ok_or_else(|| {
        fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "parse_failed",
            "AI 返回了内容但未能解析为有效 JSON",
        )
    })
}

pub(crate) fn parse_meta(raw: Option<&str>) -> Value {
    // 只接受 JSON 对象：后面所有写回都是 meta["xxx"] = ... 的下标赋值，
    // 非对象（null / 数组 / 字符串）会让下标赋值 panic，这里统一归一为 {}。
    let parsed = raw.and_then(|s| serde_json::from_str::<Value>(s).ok());
    match parsed {
        Some(v) if v.is_object() => v,
        _ => json!({}),
    }
}

fn resume_content_fp(user_id: &str, markdown: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(user_id.as_bytes());
    h.update(b"\n");
    h.update(markdown.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

async fn find_resume_id_by_fp(state: &AppState, user_id: &str, fp: &str) -> Option<String> {
    let row = documents::Entity::find()
        .filter(owner_filter(user_id))
        .filter(sea_orm::sea_query::Expr::cust_with_values(
            "metadata::json->>'content_fp' = $1",
            [fp.to_string()],
        ))
        .order_by_desc(documents::Column::CreatedAt)
        .one(&state.db)
        .await
        .ok()
        .flatten()?;
    Some(row.id.to_string())
}

/// ingest 成功后内部打 job-service 库内 match。失败不阻断入库，但带 match_status。
async fn fetch_library_matches(headers: &HeaderMap, resume_text: &str) -> Value {
    if resume_text.trim().len() < 20 {
        return json!({
            "matches": [],
            "total_evaluated": 0,
            "match_status": "no_fit",
        });
    }
    let origin = std::env::var("JOB_SERVICE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let url = format!("{}/v1/jobs/match", origin.trim_end_matches('/'));
    let mut req = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
        .post(url)
        .json(&json!({ "resume_text": resume_text }));
    if let Some(auth) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        req = req.header("Authorization", auth);
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.json::<Value>().await.unwrap_or(json!({}));
            if status.is_success() {
                let mut out = body;
                if out.get("match_status").is_none() {
                    out["match_status"] = json!("ok");
                }
                out
            } else {
                let code = body.get("error").and_then(|v| v.as_str()).unwrap_or("");
                let match_status = if status.as_u16() == 429 || code.contains("quota") {
                    "quota_exceeded"
                } else {
                    "unavailable"
                };
                json!({
                    "matches": [],
                    "total_evaluated": 0,
                    "match_status": match_status,
                    "match_error": body.get("message").cloned().unwrap_or(json!(status.to_string())),
                })
            }
        }
        Err(e) => json!({
            "matches": [],
            "total_evaluated": 0,
            "match_status": "unavailable",
            "match_error": e.to_string(),
        }),
    }
}

fn parse_json_block(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    let body = if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            &trimmed[start..=end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    serde_json::from_str(body).ok()
}
