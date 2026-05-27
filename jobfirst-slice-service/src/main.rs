mod match_algo;
mod models;
mod store;

use match_algo::rank_jobs;

use axum::{
    extract::{Path, Query, State},
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
    State(st): State<AppState>,
    Json(body): Json<PostResumeBody>,
) -> Json<Envelope<Value>> {
    if body.text.trim().is_empty() {
        return err(400, "text 不能为空");
    }
    let record = st.store.add_resume(body.text).await;
    ok(json!({ "resume_id": record.resume_id }))
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
    State(st): State<AppState>,
    Query(q): Query<MatchesQuery>,
) -> Json<Envelope<Value>> {
    let Some(resume) = st.store.get_resume(&q.resume_id).await else {
        return err(404, "resume_id 不存在");
    };
    let jobs = st.store.jobs().await;
    let matches = rank_jobs(&resume.text, &jobs, q.top);
    ok(json!({ "resume_id": q.resume_id, "matches": matches }))
}

async fn post_apply(
    State(st): State<AppState>,
    Json(body): Json<PostApplyBody>,
) -> Json<Envelope<Value>> {
    if body.job_ids.is_empty() {
        return err(400, "job_ids 不能为空");
    }
    if st.store.get_resume(&body.resume_id).await.is_none() {
        return err(404, "resume_id 不存在");
    }
    match st
        .store
        .create_applications(&body.resume_id, &body.job_ids)
        .await
    {
        Ok(apps) => ok(json!({ "applications": apps })),
        Err(e) => err(400, e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct InboxQuery {
    resume_id: String,
}

async fn get_inbox(
    State(st): State<AppState>,
    Query(q): Query<InboxQuery>,
) -> Json<Envelope<Value>> {
    let items = st.store.inbox_for_resume(&q.resume_id).await;
    ok(json!({ "resume_id": q.resume_id, "items": items }))
}

async fn post_decision(
    State(st): State<AppState>,
    Path(application_id): Path<String>,
    Json(body): Json<DecisionBody>,
) -> Json<Envelope<Value>> {
    match st.store.set_decision(&application_id, &body.action).await {
        Ok(app) => ok(json!(app)),
        Err(e) => err(400, e.to_string()),
    }
}

async fn post_mock_feedback(
    State(st): State<AppState>,
    Json(body): Json<MockFeedbackBody>,
) -> Json<Envelope<Value>> {
    let status = match body.status.as_str() {
        "replied" => ApplicationStatus::Replied,
        "invited" => ApplicationStatus::Invited,
        "rejected" => ApplicationStatus::Rejected,
        _ => return err(400, "status 须为 replied | invited | rejected"),
    };
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
                "feedback recorded (M0 log only)"
            );
            ok(json!(app))
        }
        Err(e) => err(404, e.to_string()),
    }
}
