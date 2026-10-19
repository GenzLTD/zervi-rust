use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{header, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{any, post},
    Router,
};
use consul_client::ServiceDiscovery;
use reqwest::Client;
use std::{net::SocketAddr, sync::Arc};
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

mod tatha;

#[derive(Clone)]
struct AppState {
    http_client: Client,
    service_discovery: Arc<ServiceDiscovery>,
}

/// 构建网关 Router。
///
/// 从 `main()` 抽出，唯一目的是让测试能复用它：`api-gateway` 是 binary crate
/// （无 `src/lib.rs`），`tests/` 下的集成测试访问不到本文件的项，故端到端测试
/// 以 `#[cfg(test)] mod tests` 形式放在本文件内。
///
/// 路由、CORS、state 构造与抽离前**逐行一致**，不含任何逻辑改动。
fn app() -> Router {
    // 创建状态
    let state = AppState {
        http_client: Client::new(),
        service_discovery: Arc::new(ServiceDiscovery::new()),
    };

    // 配置CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    // 构建路由
    Router::new()
        .route("/health", axum::routing::get(health_check))
        // GFCC ask 单入口：网关鉴权后转发到 Tatha 主仓（Python 中央大脑）
        .route("/v1/ask", post(tatha::ask))
        // 量子认证兼容端点：Tatha 的 QUANTUM_AUTH_URL 指向此处
        .route("/v1/auth/verify", axum::routing::get(tatha::verify))
        // 认证服务路由
        .route("/api/v1/auth/*path", any(route_to_auth_service))
        // 用户服务路由
        .route("/api/v1/user/*path", any(route_to_user_service))
        // 简历服务路由
        .route("/api/v1/resume/*path", any(route_to_resume_service))
        // 职位服务路由
        .route("/api/v1/job/*path", any(route_to_job_service))
        // 投递服务路由
        .route(
            "/api/v1/application/*path",
            any(route_to_application_service),
        )
        // JobFirst M0 Slice（收件箱）
        .route("/api/v1/slice/*path", any(route_to_jobfirst_slice))
        // 企业服务路由
        .route("/api/v1/company/*path", any(route_to_company_service))
        // 上传服务路由
        .route("/api/v1/upload/*path", any(route_to_upload_service))
        // 临时：转发到旧的单体应用（Week 1使用）
        .route("/api/*path", any(route_to_legacy_backend))
        .layer(cors)
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    info!("🚀 Starting ADIRP API Gateway...");

    let app = app();

    // 启动服务器（使用用户指定的3333端口）
    let addr = SocketAddr::from(([0, 0, 0, 0], 3333));
    info!("✅ API Gateway listening on http://{}", addr);
    info!("📍 Consul: http://localhost:8500");
    info!("🔍 Routes:");
    info!("   - /health                    -> Health check");
    info!("   - /v1/ask                    -> tatha (Python brain, gated)");
    info!("   - /v1/auth/verify            -> token introspection for tatha");
    info!("   - /api/v1/auth/*             -> auth-service");
    info!("   - /api/v1/user/*             -> user-service");
    info!("   - /api/v1/resume/*           -> resume-service");
    info!("   - /api/v1/job/*              -> job-service");
    info!("   - /api/v1/application/*      -> application-service");
    info!("   - /api/v1/slice/*            -> jobfirst-slice-service");
    info!("   - /api/v1/company/*          -> company-service");
    info!("   - /api/v1/upload/*           -> upload-service");
    info!("   - /api/*                     -> legacy backend (temp)");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "OK - API Gateway is running"
}

// 路由处理函数
async fn route_to_auth_service(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    route_to_service(&state, "auth-service", 3001, &path, req).await
}

async fn route_to_user_service(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    route_to_service(&state, "user-service", 3002, &path, req).await
}

async fn route_to_resume_service(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    route_to_service(&state, "resume-service", 3003, &path, req).await
}

async fn route_to_job_service(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    route_to_service(&state, "job-service", 3004, &path, req).await
}

async fn route_to_application_service(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    route_to_service(&state, "application-service", 3005, &path, req).await
}

/// `/api/v1/slice/*` —— 终端侧的 job_match 适配器入口。
///
/// 与 `/v1/ask` 共用 `authenticate_and_stamp`：先验 token，再注入
/// `X-Actor-Sub`（原始 sub）与 `X-User-Id`（sha256(sub)）。此前这条路由是
/// **纯透传**：匿名也能写简历、读收件箱，且下游拿不到任何身份 ——
/// GFCC 1.3.0 要求 `job_match` 结果只能以 `weight: auxiliary` 喂
/// `identity.capability_trace`，没有 actor 就无法归因，适配器等于不可用。
async fn route_to_jobfirst_slice(
    State(state): State<AppState>,
    Path(path): Path<String>,
    uri: Uri,
    mut req: Request,
) -> Response {
    if let Err(resp) = tatha::authenticate_and_stamp(&mut req).await {
        tracing::warn!("🚫 /api/v1/slice/{} rejected: no identity", path);
        return resp;
    }
    // `Path` 只给出路径段，query 会被丢掉；`matches` / `inbox` 都靠 query 传
    // `resume_id`，不补回去下游就会报「missing field resume_id」。
    let path = match uri.query() {
        Some(q) => format!("{}?{}", path, q),
        None => path,
    };
    route_to_service(&state, "jobfirst-slice-service", 3005, &path, req).await
}

async fn route_to_company_service(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    route_to_service(&state, "company-service", 3006, &path, req).await
}

async fn route_to_upload_service(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    route_to_service(&state, "upload-service", 3007, &path, req).await
}

// 临时：路由到旧的单体应用
async fn route_to_legacy_backend(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    // Week 1使用：直接转发到旧的单体应用
    let fallback_url = format!("http://localhost:3000/{}", path);
    info!("🔄 Routing to legacy backend: {}", fallback_url);
    proxy_request(&state.http_client, &fallback_url, req).await
}

// 核心服务发现和路由逻辑
async fn route_to_service(
    state: &AppState,
    service_name: &str,
    fallback_port: u16,
    path: &str,
    req: Request,
) -> Response {
    // 1. 从Consul发现服务
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

    // 2. 构建完整URL
    let target_url = format!("{}/{}", service_url, path);
    info!("🔄 Proxying to: {}", target_url);

    // 3. 转发请求
    proxy_request(&state.http_client, &target_url, req).await
}

// HTTP代理实现
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

    // 转换method为reqwest的Method
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

    // 构建reqwest请求
    let mut req_builder = client.request(reqwest_method, target_url);

    // 复制headers（跳过某些不应该转发的headers）
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
