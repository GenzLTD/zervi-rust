//! `/v1/resume/*` 八个端点。身份、同意、配额、错误体对齐 looma。
//! 二进制简历转换（MarkItDown）不在本服务；PDF/DOCX 返回 `convert_failed`。
//! LLM 走 X1，过滤后无可用 provider 则 `parse_failed` / 分析字段为 null。
//!
//! 契约：looma-zervi backend/contracts/resume.v1.json v0.1.0。
//! templates 为全局只读目录（auth: optional，无 consent，非用户资源）——
//! 0.1 不提供写操作；用户自建模板须另开 /v1/resume/templates/custom 并升 minor。

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use common::entities::documents;
use common::looma::{
    consume_daily, fail, now_tz, require_consent, resolve_principal, AppState,
    AuthMode, Principal, Resource,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Set,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/resume/parse", post(parse_resume))
        .route("/v1/resume/upload", post(upload_resume))
        .route("/v1/resume/ingest", post(ingest_resume))
        .route("/v1/resume/improve", post(improve_resume))
        .route("/v1/resume/list", get(list_resumes))
        .route("/v1/resume/templates", get(list_templates))
        .route("/v1/resume/analysis", get(analysis_resume))
        .route("/v1/resume/:resume_id", axum::routing::delete(delete_resume))
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
    match insert_resume(&state, &principal, &filename, &markdown, body.extracted, doc_status).await {
        Ok((id, _)) => Json(json!({ "resume_id": id, "status": api_status })).into_response(),
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
    let principal = match gate(&state, &headers, AuthMode::Required, None).await {
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

async fn delete_resume(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(resume_id): Path<String>,
) -> Response {
    let principal = match gate(&state, &headers, AuthMode::Required, None).await {
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

fn parse_meta(raw: Option<&str>) -> Value {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}))
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
