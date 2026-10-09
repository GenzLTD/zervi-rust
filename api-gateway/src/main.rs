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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    info!("🚀 Starting ADIRP API Gateway...");

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
    let app = Router::new()
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
        .with_state(state);

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

async fn route_to_jobfirst_slice(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
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
