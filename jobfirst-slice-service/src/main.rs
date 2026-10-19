mod match_algo;
mod models;
mod store;

use match_algo::rank_jobs;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::models::{
    ApplicationStatus, DecisionBody, MockFeedbackBody, PostApplyBody, PostResumeBody,
};
use crate::store::Store;

#[derive(Clone)]
struct AppState {
    store: Store,
}

#[derive(Serialize)]
struct Envelope<T: Serialize> {
    code: i32,
    msg: String,
    data: Option<T>,
}

fn ok<T: Serialize>(data: T) -> Json<Envelope<T>> {
    Json(Envelope {
        code: 0,
        msg: "成功".into(),
        data: Some(data),
    })
}

fn err(code: i32, msg: impl Into<String>) -> Json<Envelope<Value>> {
    Json(Envelope {
        code,
        msg: msg.into(),
        data: None,
    })
}

fn err_with_status(status: StatusCode, code: i32, msg: impl Into<String>) -> Response {
    (status, err(code, msg)).into_response()
}

/// 终端身份：网关注入的原始 `sub`（GFCC 1.3.0 `actor_sub`）。
fn actor_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-actor-sub")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 默认 fail-closed：缺身份即 401。
///
/// `JOBFIRST_SLICE_ALLOW_ANONYMOUS=1` 只留给本机旧脚本/单点联调，终端部署不设。
fn actor_required(headers: &HeaderMap) -> Result<Option<String>, Response> {
    let actor = actor_of(headers);
    let allow_anonymous = std::env::var("JOBFIRST_SLICE_ALLOW_ANONYMOUS")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);
    if actor.is_none() && !allow_anonymous {
        return Err(err_with_status(
            StatusCode::UNAUTHORIZED,
            401,
            "缺少终端身份（x-actor-sub）",
        ));
    }
    Ok(actor)
}

/// 数据归属判定。未归因的数据（匿名开发模式写入）不拒绝；有主数据必须同 actor。
const ROLE_LADDER: [&str; 8] = [
    "observer",
    "participant",
    "experimenter",
    "proposer",
    "steward",
    "organizer",
    "actor",
    "petitioner",
];

fn role_at_least(role: &str, minimum: &str) -> bool {
    let rank = |name: &str| ROLE_LADDER.iter().position(|r| *r == name);
    match (rank(role), rank(minimum)) {
        (Some(have), Some(need)) => have >= need,
        _ => false,
    }
}

fn actor_role(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-actor-role")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
}

fn owned_by(owner: &Option<String>, actor: &Option<String>) -> bool {
    match (owner, actor) {
        (Some(o), Some(a)) => o == a,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let store = Store::new().await?;
    let state = AppState { store };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health))
        .route("/resume", post(post_resume))
        .route("/matches", get(get_matches))
        .route("/apply", post(post_apply))
        .route("/inbox", get(get_inbox))
        .route("/inbox/:application_id/decision", post(post_decision))
        .route("/dev/mock-feedback", post(post_mock_feedback))
        .route("/capability-trace", get(get_capability_trace))
        .layer(cors)
        .with_state(state);

    let port: u16 = std::env::var("JOBFIRST_SLICE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3005);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("JobFirst slice listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "OK - jobfirst-slice-service"
}

async fn post_resume(
    headers: HeaderMap,
    State(st): State<AppState>,
    Json(body): Json<PostResumeBody>,
) -> Response {
    let actor = match actor_required(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    if body.text.trim().is_empty() {
        return err(400, "text 不能为空").into_response();
    }
    let record = st.store.add_resume(body.text, actor).await;
    ok(json!({
        "resume_id": record.resume_id,
        "actor_sub": record.actor_sub,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct MatchesQuery {
    resume_id: String,
    #[serde(default = "default_top")]
    top: usize,
}

fn default_top() -> usize {
    10
}

async fn get_matches(
    headers: HeaderMap,
    State(st): State<AppState>,
    Query(q): Query<MatchesQuery>,
) -> Response {
    let actor = match actor_required(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let Some(resume) = st.store.get_resume(&q.resume_id).await else {
        return err_with_status(StatusCode::NOT_FOUND, 404, "resume_id 不存在");
    };
    // 不区分「不存在」与「不是你的」：避免用 404/403 差异探测他人 resume_id。
    if !owned_by(&resume.actor_sub, &actor) {
        return err_with_status(StatusCode::NOT_FOUND, 404, "resume_id 不存在");
    }
    let jobs = st.store.jobs().await;
    let matches = rank_jobs(&resume.text, &jobs, q.top);
    if let Some(actor_sub) = actor.as_deref() {
        st.store
            .append_job_match_traces(actor_sub, &matches)
            .await;
    }
    ok(json!({ "resume_id": q.resume_id, "matches": matches })).into_response()
}

/// 只读本人的辅助痕迹。读记忆/身份的最低角色是 observer。
async fn get_capability_trace(headers: HeaderMap, State(st): State<AppState>) -> Response {
    let actor = match actor_required(&headers) {
        Ok(Some(actor)) => actor,
        Ok(None) => {
            return err_with_status(StatusCode::UNAUTHORIZED, 401, "缺少终端身份（x-actor-sub）");
        }
        Err(resp) => return resp,
    };
    let Some(role) = actor_role(&headers) else {
        return err_with_status(StatusCode::FORBIDDEN, 403, "缺少八角梯角色");
    };
    if !role_at_least(&role, "observer") {
        return err_with_status(StatusCode::FORBIDDEN, 403, "capability_trace 读取需要 observer");
    }
    let traces = st.store.traces_for(&actor).await;
    ok(json!({ "actor_sub": actor, "traces": traces })).into_response()
}

async fn post_apply(
    headers: HeaderMap,
    State(st): State<AppState>,
    Json(body): Json<PostApplyBody>,
) -> Response {
    let actor = match actor_required(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    if body.job_ids.is_empty() {
        return err(400, "job_ids 不能为空").into_response();
    }
    let Some(resume) = st.store.get_resume(&body.resume_id).await else {
        return err_with_status(StatusCode::NOT_FOUND, 404, "resume_id 不存在");
    };
    if !owned_by(&resume.actor_sub, &actor) {
        return err_with_status(StatusCode::NOT_FOUND, 404, "resume_id 不存在");
    }
    match st
        .store
        .create_applications(&body.resume_id, &body.job_ids, resume.actor_sub)
        .await
    {
        Ok(apps) => ok(json!({ "applications": apps })).into_response(),
        Err(e) => err(400, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct InboxQuery {
    resume_id: String,
}

async fn get_inbox(
    headers: HeaderMap,
    State(st): State<AppState>,
    Query(q): Query<InboxQuery>,
) -> Response {
    let actor = match actor_required(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let Some(resume) = st.store.get_resume(&q.resume_id).await else {
        return err_with_status(StatusCode::NOT_FOUND, 404, "resume_id 不存在");
    };
    if !owned_by(&resume.actor_sub, &actor) {
        return err_with_status(StatusCode::NOT_FOUND, 404, "resume_id 不存在");
    }
    let items = st.store.inbox_for_resume(&q.resume_id).await;
    ok(json!({ "resume_id": q.resume_id, "items": items })).into_response()
}

async fn post_decision(
    headers: HeaderMap,
    State(st): State<AppState>,
    Path(application_id): Path<String>,
    Json(body): Json<DecisionBody>,
) -> Response {
    let actor = match actor_required(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let Some(app) = st.store.get_application(&application_id).await else {
        return err_with_status(StatusCode::NOT_FOUND, 404, "unknown application_id");
    };
    if !owned_by(&app.actor_sub, &actor) {
        return err_with_status(StatusCode::NOT_FOUND, 404, "unknown application_id");
    }
    match st.store.set_decision(&application_id, &body.action).await {
        Ok(app) => ok(json!(app)).into_response(),
        Err(e) => err(400, e.to_string()).into_response(),
    }
}

async fn post_mock_feedback(
    headers: HeaderMap,
    State(st): State<AppState>,
    Json(body): Json<MockFeedbackBody>,
) -> Response {
    let actor = match actor_required(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let status = match body.status.as_str() {
        "replied" => ApplicationStatus::Replied,
        "invited" => ApplicationStatus::Invited,
        "rejected" => ApplicationStatus::Rejected,
        _ => return err(400, "status 须为 replied | invited | rejected").into_response(),
    };
    let Some(app) = st.store.get_application(&body.application_id).await else {
        return err_with_status(StatusCode::NOT_FOUND, 404, "unknown application_id");
    };
    if !owned_by(&app.actor_sub, &actor) {
        return err_with_status(StatusCode::NOT_FOUND, 404, "unknown application_id");
    }
    match st.store.set_feedback(&body.application_id, status.clone()).await {
        Ok(app) => {
            let event = match status {
                ApplicationStatus::Invited => "jobfirst.feedback.invited",
                ApplicationStatus::Rejected => "jobfirst.feedback.rejected",
                _ => "jobfirst.feedback.received",
            };
            info!(
                target: "jobfirst.events",
                event = event,
                application_id = %app.application_id,
                company = %app.company_name,
                title = %app.title,
                actor_sub = app.actor_sub.as_deref().unwrap_or("-"),
                "feedback recorded (M0 log only)"
            );
            ok(json!(app)).into_response()
        }
        Err(e) => err_with_status(StatusCode::NOT_FOUND, 404, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::role_at_least;
    use crate::models::MatchItem;
    use crate::store::Store;

    #[test]
    fn observer_can_read_but_cannot_pass_participant_gate() {
        assert!(role_at_least("observer", "observer"));
        assert!(!role_at_least("observer", "participant"));
        assert!(role_at_least("participant", "observer"));
        assert!(!role_at_least("candidate", "observer"));
    }

    /// job_match 只写 auxiliary，且只挂在当前 actor 上。空身份或空结果不落痕迹。
    #[tokio::test]
    async fn job_match_writes_auxiliary_trace_for_that_actor_only() {
        let store = Store::new().await.expect("seed jobs");
        let item = MatchItem {
            job_id: "j1".into(),
            title: "Rust 工程师".into(),
            company_name: "acme".into(),
            score: 1.0,
            reason_preview: "kw".into(),
        };
        store.append_job_match_traces("", &[item.clone()]).await;
        store.append_job_match_traces("actor-a", &[]).await;
        assert!(store.traces_for("actor-a").await.is_empty());

        store.append_job_match_traces("actor-a", &[item]).await;
        let mine = store.traces_for("actor-a").await;
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].label, "Rust 工程师");
        assert_eq!(mine[0].source_port, "job_match");
        assert_eq!(mine[0].weight, "auxiliary");
        assert_eq!(mine[0].actor_sub, "actor-a");
        assert!(store.traces_for("actor-b").await.is_empty());
    }
}
