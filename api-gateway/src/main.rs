use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Router,
};
use consul_client::ServiceDiscovery;
use reqwest::Client;
use std::{net::SocketAddr, sync::Arc};
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

mod local_auth;
mod tatha;

#[derive(Clone)]
struct AppState {
    http_client: Client,
    service_discovery: Arc<ServiceDiscovery>,
    /// 本地开发账号体系用的 PG 连接池（懒连接，不阻塞启动）。
    ///
    /// 只有 `/v1/auth/register|login|me` 会用到；转发链路（resume/job/...）不依赖它，
    /// 所以 `DATABASE_URL` 缺失或 PG 未起时网关仍能正常做反向代理。
    db: Option<sqlx::PgPool>,
}

/// 构建网关 Router。
///
/// 从 `main()` 抽出，唯一目的是让测试能复用它：`api-gateway` 是 binary crate
/// （无 `src/lib.rs`），`tests/` 下的集成测试访问不到本文件的项，故端到端测试
/// 以 `#[cfg(test)] mod tests` 形式放在本文件内。
///
/// 路由、CORS、state 构造与抽离前**逐行一致**，不含任何逻辑改动。

/// looma 单体后端（Flask）端口 —— 过渡期兜底目标。
/// 本轮未迁移的 Blueprint（poetry / game / trust / compliance / reports / ...）
/// 仍在 :5200。
///
/// ⚠️ **payment 已于 2026-10-21 部分移出本兜底** —— 原先它被列在上面那串
///「仍在 :5200」里，payment-service 后来已在 :3009 独立运行却没人回头改注释，
/// 网关迟迟没给直通路由。
///
/// **但要特别注意：looma :5200 并非没有支付实现。** 它挂着 `payment_bp`
///（url_prefix="/v1"）与 `huawei_iap_bp`（url_prefix="/v1/payment/huawei"），
/// 提供微信 / Stripe / PayPal / Airwallex / 华为 IAP，即 `/v1/payment/*` 在
/// :5200 **本来就存在**，只是**不含 Apple 分支**。所以：
///   · 修复前 Apple 三端点落到 :5200 得到的是 **404**（不是"那边没支付"）；
///   · 本机 looma 未启动时表现为 502 连接失败。
///
/// **数据面与 looma 并不互通（跨仓一致性检查，2026-10-21）**：
/// looma 侧落 **SQLite**（`backend/src/db/manager.py`，`subscriptions` / `users.tier`），
/// 本仓 payment-service 落 **PostgreSQL**（`iap_subscriptions` / `apple_iap_transactions`
/// / `users.tier`）。两侧表名与列名同名但不同库 ⇒ Apple 支付成功后写的是 PG，
/// looma 的 `/v1/payment/status` 读 SQLite，看到的仍是 free。**这是当前最大的
/// 一致性缺口，收敛方案待定，勿假定两边会自动同步。**
const LOOMA_LEGACY_PORT: u16 = 5200;

fn app() -> Router {
    // 创建状态
    //
    // ⚠️ `reqwest` 默认会读取环境变量 `HTTP_PROXY` / `HTTPS_PROXY`，
    // 但网关做的是**内网服务间调用**（Consul 服务发现 + localhost 回落），
    // 走外部代理会带来三个问题：
    //   1. 请求行退化为 absolute-form（`GET http://localhost:3004/v1/xxx`），
    //      与 looma 直连语义不一致；
    //   2. 每次转发多一跳，延迟与失败面扩大；
    //   3. 代理不可用时整个网关瘫痪。
    // ⇒ 显式 `no_proxy()`，服务间调用一律直连。
    let http_client = Client::builder()
        .no_proxy()
        .build()
        .expect("failed to build reqwest client");

    let state = AppState {
        http_client,
        service_discovery: Arc::new(ServiceDiscovery::new()),
        // `connect_lazy` 不发起真实连接 ⇒ 网关不会因为 PG 没起而启动失败
        db: std::env::var("DATABASE_URL")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .and_then(|url| sqlx::postgres::PgPool::connect_lazy(&url).ok()),
    };

    // 配置 CORS（前端走 Bearer header，不用 cookie ⇒ 无需 allow_credentials）
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    // 构建路由
    // 口径（2026-10-16 拍板）：
    //   · 对外只暴露 /v1/*（无 /api 层，也不加 /api/v1/* 别名）
    //   · 转发保留**完整原始 path+query**（不剥前缀）⇒ 下游服务可与 looma 逐字对齐
    //   · 鉴权**不在网关裁决**：Authorization 原样透传，由服务层按 looma 装饰器语义收口
    //   · 未迁移的 looma Blueprint 由 fallback 兜到 :5200
    Router::new()
        // ── 网关自有端点 ────────────────────────────────────────────────
        .route("/health", axum::routing::get(health_check))
        // GFCC ask 单入口：网关鉴权后转发到 Tatha 主仓（Python 中央大脑）
        .route("/v1/ask", post(tatha::ask))
        // 量子认证兼容端点：Tatha 的 QUANTUM_AUTH_URL 指向此处
        .route("/v1/auth/verify", get(tatha::verify))

        // ── 本地开发账号体系（邮箱 + 密码，ZERVI_DEV_AUTH=1 才启用）────────
        // 生产走 Supabase Auth；这三条仅用于本地让客户端的 Required 级接口可用。
        .route("/v1/auth/register", post(local_auth::register))
        .route("/v1/auth/login", post(local_auth::login))
        .route("/v1/auth/me", get(local_auth::me))

        // ── resume-service · /v1/resume/* (8) ───────────────────────────
        // 静态段优先于 :resume_id（契约 path_design.literal_before_param）：
        // templates 必须显式注册，不能落进 :resume_id 通配。
        .route("/v1/resume/parse", any(route_to_resume_service))
        .route("/v1/resume/upload", any(route_to_resume_service))
        .route("/v1/resume/ingest", any(route_to_resume_service))
        .route("/v1/resume/improve", any(route_to_resume_service))
        .route("/v1/resume/list", any(route_to_resume_service))
        .route("/v1/resume/templates", any(route_to_resume_service))
        .route("/v1/resume/analysis", any(route_to_resume_service))
        .route("/v1/resume/:resume_id", any(route_to_resume_service))

        // ── job-service · /v1/jobs/* (9) ────────────────────────────────
        // #12 根路径：looma 侧 strict_slashes=False（/v1/jobs 与 /v1/jobs/ 都收）；
        // 尾斜杠版本轮落 fallback → :5200，行为仍与 looma 一致。
        .route("/v1/jobs", any(route_to_job_service))
        .route("/v1/jobs/list", any(route_to_job_service))
        .route("/v1/jobs/upload", any(route_to_job_service))
        .route("/v1/jobs/parse", any(route_to_job_service))
        .route("/v1/jobs/match", any(route_to_job_service))
        .route("/v1/jobs/search", any(route_to_job_service))
        .route("/v1/jobs/seed-demo", any(route_to_job_service))
        .route("/v1/jobs/recommend", any(route_to_job_service))
        .route("/v1/jobs/:job_id", any(route_to_job_service))

        // ── job-service · /v1/job-posts (5) ──  双轨 B 轨入口
        .route("/v1/job-posts", any(route_to_job_service))
        .route("/v1/job-posts/:post_id", any(route_to_job_service))
        .route("/v1/job-posts/:post_id/matches", any(route_to_job_service))

        // ── application-service · /v1/application/* (5) ─────────────────
        .route("/v1/application", any(route_to_application_service))
        .route(
            "/v1/application/:application_id",
            any(route_to_application_service),
        )
        .route(
            "/v1/application/:application_id/report",
            any(route_to_application_service),
        )
        // #27 D6：跨前缀路径 `/v1/jobs/<job_id>/applications` → application-service
        //（matchit 中 param 分支比 wildcard/static 兄弟节点独立建树，与 `/v1/jobs/:job_id` 不冲突）
        .route(
            "/v1/jobs/:job_id/applications",
            any(route_to_application_service),
        )

        // ── company-service · /v1/enterprise/* (8) ──────────────────────
        // 对外路径用 looma 的 enterprise 命名；crate 名保持 company-service（已拍板不改）
        .route("/v1/enterprise/create", any(route_to_company_service))
        .route("/v1/enterprise/join", any(route_to_company_service))
        .route("/v1/enterprise/profile", any(route_to_company_service))
        .route("/v1/enterprise/candidates", any(route_to_company_service))
        .route(
            "/v1/enterprise/candidate/:candidate_id",
            any(route_to_company_service),
        )
        .route(
            "/v1/enterprise/candidates/import-share",
            any(route_to_company_service),
        )
        .route(
            "/v1/enterprise/candidates/add",
            any(route_to_company_service),
        )
        .route(
            "/v1/enterprise/contact-sales",
            any(route_to_company_service),
        )

        // ── payment-service · /v1/payment/apple/* (3) ───────────────────
        // Apple IAP（2026-10-21 由 feat/apple-iap 合入主线）→ payment-service :3009。
        //
        // ⚠️ **`/v1/payment/` 前缀现在是分裂的，勿假定同源**：
        //   · `/v1/payment/apple/*`  → 本仓 payment-service :3009（PostgreSQL）
        //   · `/v1/payment/*` 其余   → fallback looma :5200（SQLite，微信/Stripe/
        //                              PayPal/Airwallex/华为 IAP，**不含 Apple**）
        // 两侧数据不互通，Apple 支付成功后写 PG，looma 的 status 查 SQLite 仍为 free。
        // 详见文件顶部 `LOOMA_LEGACY_PORT` 处的说明。
        //
        // 转发遵循 D2 口径保留完整 path+query，故网关对外路径与 payment-service
        // 内部路由逐字相同（/v1/payment/apple/verify 等）。
        // Apple 的 webhook 由 App Store 服务器回调，无网关鉴权头 —— 鉴权在
        // payment-service 内部按 Apple 签名校验收口（与网关「不在网关裁决」口径一致）。
        .route("/v1/payment/apple/webhook", any(route_to_payment_service))
        .route("/v1/payment/apple/verify", any(route_to_payment_service))
        .route("/v1/payment/apple/restore", any(route_to_payment_service))

        // ── 过渡期 /api/v1/* 通路（main 侧既有，非 looma 35 端点口径）──────
        // 合并说明：上面的 /v1/* 是新口径（10-16 拍板），但 auth / user /
        // upload / slice 这几个 Blueprint 尚未迁到 /v1/*，GFCC 终端仍在打
        // /api/v1/slice/*。原 #8 分支重写路由表时把整段 /api/v1/* 删掉了，
        // 那会让终端 job_match 通路静默失效 ⇒ 此处显式补回。
        // *path 不做抽取由 handler 自行处理，`any()` 仍可匹配。
        .route("/api/v1/auth/*path", any(route_to_auth_service))
        .route("/api/v1/user/*path", any(route_to_user_service))
        .route("/api/v1/resume/*path", any(route_to_resume_service))
        .route("/api/v1/job/*path", any(route_to_job_service))
        .route(
            "/api/v1/application/*path",
            any(route_to_application_service),
        )
        .route("/api/v1/slice/*path", any(route_to_jobfirst_slice))
        .route("/api/v1/company/*path", any(route_to_company_service))
        .route("/api/v1/upload/*path", any(route_to_upload_service))
        .route("/api/*path", any(route_to_legacy_backend))

        // ── 其余 looma Blueprint（poetry / game / trust / compliance /
        //    reports / ...）→ 仍在 :5200 ────────────────────────────────
        //    payment 已不再此列 —— 见上方 /v1/payment/apple/* 直通路由。
        .fallback(route_to_legacy_backend)
        .layer(cors)
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载仓库根 .env（不覆盖已存在的环境变量）：
    // JWT_SECRET / DATABASE_URL 等既被 tatha 鉴权用到，也被本地账号体系用到。
    let _ = dotenvy::dotenv();

    // 初始化日志
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    info!("🚀 Starting ADIRP API Gateway...");
    if matches!(
        std::env::var("ZERVI_DEV_AUTH").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    ) {
        info!("🔑 本地开发账号体系已启用：/v1/auth/register|login|me（生产请用 Supabase Auth）");
    }

    let app = app();

    // 启动服务器（用户指定 3333 端口）
    let addr = SocketAddr::from(([0, 0, 0, 0], 3333));
    info!("✅ API Gateway listening on http://{}", addr);
    info!("📍 Consul: http://localhost:8500");
    info!("🔍 Routes (looma /v1/* contract, 35 endpoints + gateway-owned):");
    info!("   - /health                    -> Health check");
    info!("   - /v1/ask                    -> tatha (Python brain, gated)");
    info!("   - /v1/auth/verify            -> token introspection for tatha");
    info!("   - /v1/auth/register|login|me -> local dev auth (ZERVI_DEV_AUTH=1)");
    info!("   - /v1/resume/*          (8)  -> resume-service");
    info!("   - /v1/jobs/*            (9)  -> job-service");
    info!("   - /v1/job-posts*        (5)  -> job-service (双轨 B)");
    info!("   - /v1/application/*     (5)  -> application-service");
    info!("   - /v1/jobs/:id/applications  -> application-service (D6 跨前缀)");
    info!("   - /v1/enterprise/*      (8)  -> company-service");
    info!("   - /v1/payment/apple/*   (3)  -> payment-service :3009 (Apple IAP)");
    info!(
        "   - * (fallback)               -> looma legacy :{}",
        LOOMA_LEGACY_PORT
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "OK - API Gateway is running"
}

// ── 路由处理函数 ───────────────────────────────────────────────────────────

async fn route_to_resume_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "resume-service", 3003, req).await
}

async fn route_to_job_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "job-service", 3004, req).await
}

async fn route_to_application_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "application-service", 3005, req).await
}

async fn route_to_company_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "company-service", 3006, req).await
}

/// Apple 内购支付（paywall / 收据校验 / 订阅恢复）→ payment-service :3009。
///
/// 端口 3009 由 payment-service 启动日志实测确认（`payment-service listening
/// on 0.0.0.0:3009`），不是从既有文档推得 —— 早期笔记里写的 3006/3005 均与
/// company / application-service 冲突。
async fn route_to_payment_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "payment-service", 3009, req).await
}

// ── 以下服务本轮不注册（对应 Blueprint 仍在 looma :5200，注册会抢走 looma 端点）──
//    auth-service  → looma auth_bp 仍在 :5200（仅 /v1/auth/verify 由网关自有）
//    user-service  → looma 无独立 user 路由组
//    upload-service→ looma 的上传在 /v1/resume/upload、/v1/jobs/upload（属 resume/job 域）
//    jobfirst-slice → 非 35 端点，端口已从 3005 让到 3008（解 D7 冲突），待后续轮次
//    保留实现供后续接入。

#[allow(dead_code)]
async fn route_to_auth_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "auth-service", 3001, req).await
}

#[allow(dead_code)]
async fn route_to_user_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "user-service", 3002, req).await
}

/// `/api/v1/slice/*` —— JobFirst M0 Slice（收件箱），终端侧 job_match 适配器入口。
///
/// 与 `/v1/ask` 共用 `authenticate_and_stamp`：先验 token，再注入
/// `X-Actor-Sub`（原始 sub）与 `X-User-Id`（sha256(sub)）。此前这条路由是
/// **纯透传**：匿名也能写简历、读收件箱，且下游拿不到任何身份 ——
/// GFCC 1.3.0 要求 `job_match` 结果只能以 `weight: auxiliary` 喂
/// `identity.capability_trace`，没有 actor 就无法归因，适配器等于不可用。
///
/// 合并说明（PR #8 × main）：main 原先手工把 query 拼回 path 再传给下游，
/// 是因为当时的 `route_to_service` 会丢 query；本分支合并后采信 main.rs 底部
/// `route_to_service` 的 D2 口径 —— 内部统一保留**完整原始 path+query**，
/// 因此此处不再提取 `Path`，直接透传 `req` 即可，语义与旧实现等价。
async fn route_to_jobfirst_slice(State(state): State<AppState>, mut req: Request) -> Response {
    if let Err(resp) = tatha::authenticate_and_stamp(&mut req).await {
        tracing::warn!("🚫 /api/v1/slice rejected: no identity");
        return resp;
    }
    // D7：端口让到 3008，避免与 application-service（3005）撞车。
    route_to_service(&state, "jobfirst-slice-service", 3008, req).await
}

#[allow(dead_code)]
async fn route_to_upload_service(State(state): State<AppState>, req: Request) -> Response {
    route_to_service(&state, "upload-service", 3007, req).await
}

/// 兜底：转发到 looma 单体后端（:5200），**保留完整原始 path+query**。
async fn route_to_legacy_backend(State(state): State<AppState>, req: Request) -> Response {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let fallback_url = format!("http://localhost:{}{}", LOOMA_LEGACY_PORT, path_and_query);
    info!("🔄 Routing to looma legacy backend: {}", fallback_url);
    proxy_request(&state.http_client, &fallback_url, req).await
}

// ── 核心服务发现和路由逻辑 ─────────────────────────────────────────────────

/// 转发到注册在 Consul 的服务；Consul 未命中时回落到 localhost:{fallback_port}。
///
/// **D2 关键**：转发保留**完整原始 path+query**（早前实现取 `Path(path)` 后
/// `format!("{}/{}", url, path)` 会剥掉 `/v1/xxx` 前缀，导致下游只能收到 `/parse`，
/// 服务内路由无法与 looma 对齐）。
async fn route_to_service(
    state: &AppState,
    service_name: &str,
    fallback_port: u16,
    req: Request,
) -> Response {
    // 保留完整原始 path + query
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    // 1. 从 Consul 发现服务
    let service_url = match state.service_discovery.discover_service(service_name).await {
        Ok(Some(url)) => {
            info!("✅ Discovered {} at {}", service_name, url);
            url
        }
        Ok(None) => {
            warn!(
                "⚠️  Service {} not found in Consul, using fallback localhost:{}",
                service_name, fallback_port
            );
            format!("http://localhost:{}", fallback_port)
        }
        Err(e) => {
            error!("❌ Failed to discover service {}: {}", service_name, e);
            format!("http://localhost:{}", fallback_port)
        }
    };

    // 2. 构建完整 URL（保留完整路径，不剥前缀）
    let target_url = format!("{}{}", service_url.trim_end_matches('/'), path_and_query);
    info!("🔄 Proxying to: {}", target_url);

    // 3. 转发请求（Authorization 原样透传，网关不做身份裁定）
    proxy_request(&state.http_client, &target_url, req).await
}

// HTTP 代理实现
async fn proxy_request(client: &Client, target_url: &str, req: Request) -> Response {
    let method = req.method().clone();
    let headers = req.headers().clone();
    let body = req.into_body();

    // 读取请求体
    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response();
        }
    };

    // 转换 method 为 reqwest 的 Method
    let reqwest_method = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => {
            error!("Unsupported method: {}", method);
            return (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response();
        }
    };

    // 构建 reqwest 请求
    let mut req_builder = client.request(reqwest_method, target_url);

    // 复制 headers（跳过不应转发的）
    // 注意：Authorization **原样透传** —— 网关不注入 X-User-Id / X-User-Tier，
    // 由服务层按 looma `require_auth`/`optional_auth` 语义收口到 users.id。
    for (name, value) in headers.iter() {
        let name_str = name.as_str();
        if name_str != "host" && name_str != "connection" {
            if let Ok(value_str) = value.to_str() {
                req_builder = req_builder.header(name_str, value_str);
            }
        }
    }

    // 添加请求体
    req_builder = req_builder.body(body_bytes.to_vec());

    // 发起代理请求
    let proxy_resp = match req_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Proxy request failed: {}", e);
            return (
                StatusCode::BAD_GATEWAY,
                format!("Service unavailable: {}", e),
            )
                .into_response();
        }
    };

    // 获取状态码
    let status_code = proxy_resp.status().as_u16();
    let axum_status =
        StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    // 读取响应体
    let response_body = match proxy_resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Failed to read response body: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to read response body",
            )
                .into_response();
        }
    };

    // 构建响应
    let mut response = Response::new(Body::from(response_body));
    *response.status_mut() = axum_status;

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    // ── 测试夹具 ──
    //
    // env 是**进程级全局**（`verify_bearer` 直接读 `GATEWAY_JWT_SECRET` 等），
    // JWKS 缓存同样是进程级 `static`。两者叠加 ⇒ 用例并行会互相干扰，
    // 故所有端到端用例共用一个锁串行执行。

    const HS_SECRET: &str = "unit-test-secret-do-not-use-in-prod";

    /// 测试专用 ES256 密钥（公开无害，仅用于本地回归）。
    const ES_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgxSRWiH1RaEPWQvPj\nOAX/JhWJq/DkzL1ceWTPuyjHR+2hRANCAASazzfzCWyyvQ1GF/9g0Yz2gHFXrd9m\nM5PXNxSUHoPaWrqKEBtX1MznaZs3/bMjMys7ZhqfxiY4SlUTilsdS/31\n-----END PRIVATE KEY-----\n";
    const ES_KID: &str = "test-es256-key";
    const ES_JWKS: &str = r#"{"keys":[{"kty":"EC","crv":"P-256","kid":"test-es256-key","x":"ms838wlssr0NRhf_YNGM9oBxV63fZjOT1zcUlB6D2lo","y":"uooQG1fUzOdpmzf9syMzKztmGp_GJjhKVROKWx1L_fU","alg":"ES256","use":"sig"}]}"#;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 起一个临时 server，返回其 base URL（端口由 OS 分配）。
    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{}", addr)
    }

    /// mock JWKS 端点，并对每次拉取计数（用于验证「不放大出网」）。
    fn jwks_router(counter: Arc<AtomicUsize>) -> Router {
        Router::new().route(
            "/jwks.json",
            axum::routing::get(move || {
                let c = counter.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    ([(header::CONTENT_TYPE, "application/json")], ES_JWKS)
                }
            }),
        )
    }

    /// mock 上游：把**实际收到的头**回显成 JSON。
    /// 「注入了什么」因此是观测值，不是推断值。
    fn echo_router() -> Router {
        Router::new().route(
            "/v1/ask",
            any(|req: Request| async move {
                let mut map = serde_json::Map::new();
                for (k, v) in req.headers().iter() {
                    map.insert(
                        k.as_str().to_string(),
                        json!(v.to_str().unwrap_or("<binary>").to_string()),
                    );
                }
                Json(Value::Object(map))
            }),
        )
    }

    fn now_secs() -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
    }

    /// 造 HS256 token（header 无 kid ⇒ 不会触发 JWKS 轨）。
    fn hs_token(sub: &str, email: Option<&str>, tier: Option<&str>, ttl: i64) -> String {
        let now = now_secs();
        let mut claims = serde_json::Map::new();
        claims.insert("sub".into(), json!(sub));
        claims.insert("aud".into(), json!("authenticated"));
        claims.insert("iat".into(), json!(now));
        claims.insert("exp".into(), json!(now + ttl));
        if let Some(e) = email {
            claims.insert("email".into(), json!(e));
        }
        if let Some(t) = tier {
            claims.insert("tier".into(), json!(t));
        }
        encode(
            &Header::new(Algorithm::HS256),
            &Value::Object(claims),
            &EncodingKey::from_secret(HS_SECRET.as_bytes()),
        )
        .unwrap()
    }

    fn hs_token_with_role(sub: &str, role: &str) -> String {
        let now = now_secs();
        let claims = json!({
            "sub": sub,
            "role": role,
            "aud": "authenticated",
            "iat": now,
            "exp": now + 3600,
        });
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(HS_SECRET.as_bytes()),
        )
        .unwrap()
    }

    /// 造 ES256 token（带 kid ⇒ 走 JWKS 轨）。
    fn es_token(sub: &str, aud: &str, kid: Option<&str>) -> String {
        let now = now_secs();
        let mut header = Header::new(Algorithm::ES256);
        if let Some(k) = kid {
            header.kid = Some(k.to_string());
        }
        let claims = json!({
            "sub": sub,
            "email": "es@example.com",
            "tier": "pro",
            "aud": aud,
            "iat": now,
            "exp": now + 3600,
        });
        encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(ES_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    // ── 用例 ──

    #[tokio::test]
    async fn hs256_valid_token_is_accepted() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let gw = spawn(app()).await;
        let resp = reqwest::Client::new()
            .get(format!("{}/v1/auth/verify", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token("s1", Some("a@b.com"), Some("pro"), 3600)),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn forged_and_expired_tokens_are_rejected() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let gw = spawn(app()).await;
        let client = reqwest::Client::new();

        let r = client
            .get(format!("{}/v1/auth/verify", gw))
            .header("authorization", "Bearer bogus")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401, "伪造 token 应 401");

        let r = client
            .get(format!("{}/v1/auth/verify", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token("s1", None, None, -600)),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401, "过期 token 应 401");
    }

    /// HS384 属 HMAC 族但不在白名单内 —— ADR 只提 HS256 轨，其余一律拒。
    #[tokio::test]
    async fn hs384_is_rejected() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let now = now_secs();
        let claims = json!({"sub": "s1", "aud": "authenticated", "iat": now, "exp": now + 3600});
        let tok = encode(
            &Header::new(Algorithm::HS384),
            &claims,
            &EncodingKey::from_secret(HS_SECRET.as_bytes()),
        )
        .unwrap();

        let gw = spawn(app()).await;
        let r = reqwest::Client::new()
            .get(format!("{}/v1/auth/verify", gw))
            .header("authorization", format!("Bearer {}", tok))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401);
    }

    /// D2：无 email ⇒ **不注入** `X-User-Email-Hash`（小程序-only 用户）。
    #[tokio::test]
    async fn ask_without_email_omits_email_hash() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let upstream = spawn(echo_router()).await;
        std::env::set_var("TATHA_BASE_URL", &upstream);
        std::env::set_var("TATHA_SERVICE_TOKEN", "svc-test");

        let gw = spawn(app()).await;
        let body: Value = reqwest::Client::new()
            .post(format!("{}/v1/ask", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token("s1", None, Some("pro"), 3600)),
            )
            .body("{}")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert!(body.get("x-user-id").is_some(), "稳定主键必有");
        assert_eq!(body.get("x-user-tier").and_then(|v| v.as_str()), Some("pro"));
        assert!(body.get("x-user-email-hash").is_none(), "无 email 时不得注入");
        assert_eq!(
            body.get("x-actor-sub").and_then(|v| v.as_str()),
            Some("s1"),
            "X-Actor-Sub 必须是原始 sub"
        );
        let uid = body.get("x-user-id").and_then(|v| v.as_str()).unwrap_or("");
        assert_ne!(uid, "s1", "X-User-Id 仍是 sha256(sub)，不改成原始 sub");
        assert_eq!(uid.len(), 64);
    }

    /// 有 email ⇒ 注入跨端关联键。
    #[tokio::test]
    async fn ask_with_email_injects_email_hash() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let upstream = spawn(echo_router()).await;
        std::env::set_var("TATHA_BASE_URL", &upstream);
        std::env::set_var("TATHA_SERVICE_TOKEN", "svc-test");

        let gw = spawn(app()).await;
        let body: Value = reqwest::Client::new()
            .post(format!("{}/v1/ask", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token("s1", Some("Foo@Bar.com"), Some("pro"), 3600)),
            )
            .body("{}")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert!(body.get("x-user-email-hash").is_some(), "有 email 应注入");
    }

    /// P0 回归：客户端自带的 `x-user-*` 头必须被剥离。
    /// 否则持「无 email 的合法 token」（D2 允许的小程序-only 用户）即可
    /// 自带 `X-User-Email-Hash` 冒充他人跨端身份。
    #[tokio::test]
    async fn ask_strips_client_supplied_x_user_headers() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let upstream = spawn(echo_router()).await;
        std::env::set_var("TATHA_BASE_URL", &upstream);
        std::env::set_var("TATHA_SERVICE_TOKEN", "svc-test");

        let gw = spawn(app()).await;
        let body: Value = reqwest::Client::new()
            .post(format!("{}/v1/ask", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token("s1", None, Some("pro"), 3600)),
            )
            .header("X-User-Email-Hash", "FORGED-BY-CLIENT")
            .header("X-User-Id", "FORGED-ID")
            .body("{}")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert!(
            body.get("x-user-email-hash").is_none(),
            "客户端伪造的 Email-Hash 必须被剥离，实测收到 {:?}",
            body.get("x-user-email-hash")
        );
        assert_ne!(
            body.get("x-user-id").and_then(|v| v.as_str()),
            Some("FORGED-ID"),
            "客户端伪造的 Id 必须被覆盖"
        );
    }

    /// 客户端自带的 `X-Actor-Sub` 必须被换成 token 里的原始 sub。
    /// 放行伪造值等于允许用别人的 sub 读 memory.recall。
    #[tokio::test]
    async fn ask_replaces_client_supplied_actor_sub() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let upstream = spawn(echo_router()).await;
        std::env::set_var("TATHA_BASE_URL", &upstream);
        std::env::set_var("TATHA_SERVICE_TOKEN", "svc-test");

        let gw = spawn(app()).await;
        let body: Value = reqwest::Client::new()
            .post(format!("{}/v1/ask", gw))
            .header(
                "authorization",
                format!(
                    "Bearer {}",
                    hs_token("  user-win-001  ", None, Some("free"), 3600)
                ),
            )
            .header("X-Actor-Sub", "forged-other-sub")
            .body("{}")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(
            body.get("x-actor-sub").and_then(|v| v.as_str()),
            Some("user-win-001")
        );
        assert_ne!(
            body.get("x-user-id").and_then(|v| v.as_str()),
            Some("user-win-001")
        );
    }

    /// token 里的八角梯角色注入 `X-Actor-Role`；客户端自带的同名头被丢掉。
    /// 不在梯上的角色（如 candidate）不注入，避免把市场外字符串当成 participant。
    #[tokio::test]
    async fn ask_stamps_ladder_role_and_strips_client_role() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let upstream = spawn(echo_router()).await;
        std::env::set_var("TATHA_BASE_URL", &upstream);
        std::env::set_var("TATHA_SERVICE_TOKEN", "svc-test");

        let gw = spawn(app()).await;
        let client = reqwest::Client::new();
        let body: Value = client
            .post(format!("{}/v1/ask", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token_with_role("s1", "Participant")),
            )
            .header("X-Actor-Role", "petitioner")
            .body("{}")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            body.get("x-actor-role").and_then(|v| v.as_str()),
            Some("participant")
        );

        let body: Value = client
            .post(format!("{}/v1/ask", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token_with_role("s1", "candidate")),
            )
            .header("X-Actor-Role", "steward")
            .body("{}")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            body.get("x-actor-role").is_none(),
            "梯外角色不得注入，客户端伪造值也不得留下: {body:?}"
        );
    }

    /// 空白 sub 不能转发：下游会把缺头当成 401，同时可能已经扣过配额。
    #[tokio::test]
    async fn ask_rejects_blank_sub_without_forwarding() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);
        std::env::remove_var("GATEWAY_JWKS_URL");

        let upstream = spawn(echo_router()).await;
        std::env::set_var("TATHA_BASE_URL", &upstream);
        std::env::set_var("TATHA_SERVICE_TOKEN", "svc-test");

        let gw = spawn(app()).await;
        let r = reqwest::Client::new()
            .post(format!("{}/v1/ask", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token("   ", None, None, 3600)),
            )
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401);
    }

    /// ES256（Supabase 原生形态）：mock JWKS + 本地 P-256 密钥端到端验证。
    #[tokio::test]
    async fn es256_valid_token_is_accepted() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tatha::reset_jwks_cache();

        let counter = Arc::new(AtomicUsize::new(0));
        let jwks_url = format!("{}/jwks.json", spawn(jwks_router(counter.clone())).await);
        std::env::set_var("GATEWAY_JWKS_URL", &jwks_url);
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);

        let gw = spawn(app()).await;
        let r = reqwest::Client::new()
            .get(format!("{}/v1/auth/verify", gw))
            .header(
                "authorization",
                format!("Bearer {}", es_token("es-sub-1", "authenticated", Some(ES_KID))),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200, "ES256 合法 token 应 200");
    }

    /// JWKS 轨校验 `aud=authenticated`（对齐 looma 口径）。
    #[tokio::test]
    async fn es256_wrong_audience_is_rejected() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tatha::reset_jwks_cache();

        let counter = Arc::new(AtomicUsize::new(0));
        let jwks_url = format!("{}/jwks.json", spawn(jwks_router(counter.clone())).await);
        std::env::set_var("GATEWAY_JWKS_URL", &jwks_url);
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);

        let gw = spawn(app()).await;
        let r = reqwest::Client::new()
            .get(format!("{}/v1/auth/verify", gw))
            .header(
                "authorization",
                format!("Bearer {}", es_token("es-sub-1", "someone-else", Some(ES_KID))),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401);
    }

    /// 防放大（评审第 1 条 / 手测 #13）：构造随机 kid 连打，
    /// 缓存新鲜时命中节流 ⇒ 一次都不该出网。
    #[tokio::test]
    async fn forged_kid_does_not_amplify_jwks_fetches() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tatha::reset_jwks_cache();

        let counter = Arc::new(AtomicUsize::new(0));
        let jwks_url = format!("{}/jwks.json", spawn(jwks_router(counter.clone())).await);
        std::env::set_var("GATEWAY_JWKS_URL", &jwks_url);
        std::env::set_var("GATEWAY_JWT_SECRET", HS_SECRET);

        let gw = spawn(app()).await;
        let client = reqwest::Client::new();

        // 先用合法 kid 拉一次，填充缓存并推进 last_attempt_at
        let _ = client
            .get(format!("{}/v1/auth/verify", gw))
            .header(
                "authorization",
                format!("Bearer {}", es_token("es-sub-1", "authenticated", Some(ES_KID))),
            )
            .send()
            .await
            .unwrap();
        let after_prime = counter.load(Ordering::SeqCst);

        // 连打 5 次伪造 kid
        for _ in 0..5 {
            let r = client
                .get(format!("{}/v1/auth/verify", gw))
                .header(
                    "authorization",
                    format!(
                        "Bearer {}",
                        es_token("es-sub-1", "authenticated", Some("random-unknown-kid"))
                    ),
                )
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), 401);
        }

        let delta = counter.load(Ordering::SeqCst) - after_prime;
        assert_eq!(delta, 0, "伪造 kid 不应触发 JWKS 出网，实测增量 {}", delta);
    }

    /// 两轨皆未配置 ⇒ 一律 401（fail-closed）。
    #[tokio::test]
    async fn no_track_configured_rejects_everything() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tatha::reset_jwks_cache();
        std::env::remove_var("GATEWAY_JWT_SECRET");
        std::env::remove_var("GATEWAY_JWKS_URL");
        std::env::remove_var("TATHA_BASE_URL");

        let gw = spawn(app()).await;
        let r = reqwest::Client::new()
            .get(format!("{}/v1/auth/verify", gw))
            .header(
                "authorization",
                format!("Bearer {}", hs_token("s1", Some("a@b.com"), None, 3600)),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401);
    }
}
