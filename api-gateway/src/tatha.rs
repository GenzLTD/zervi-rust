//! Tatha 单入口转发 —— GFCC `ask.envelope.v1` 的网关侧骨架。
//!
//! 拓扑（Tatha/docs/架构_单入口与中央大脑.md §5.2 拓扑 A）：
//!
//! ```text
//! 用户 → 网关(zervi-rust: 单入口 + 鉴权) → POST /v1/ask → Tatha(Python) → 意图解析 → 返回
//! ```
//!
//! 三条硬约束，逐条对应 Tatha `src/tatha/api/auth.py::verify_token` 的
//! 「远程校验失败 → 降级 stub → 任意非空 token 放行」反模式：
//!
//! 1. 网关鉴权失败 → 401，**绝不转发**（否则未鉴权流量会撞上 Tatha 的 stub 放行）
//! 2. `TATHA_BASE_URL` 未配 → 503，**绝不静默放行**（对标它自己的 `QUANTUM_AUTH_URL`，但拒绝降级）
//! 3. 身份注入 `X-User-Id = sha256(email)`，且**不透传用户 token**
//!    （Tatha 的 stub 会把 user_id 派生为 `stub-<token>`，切断 THOMAS 跨端关联）
//!
//! TODO(对齐 GFCC)：Tatha 侧需改读 `X-User-Id`，或将其 `QUANTUM_AUTH_URL` 指向本网关，
//! 否则下游身份仍是 stub 派生值，网关这层只保证「不放未鉴权流量进去」。

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{error, info, warn};

use crate::{proxy_request, AppState};

/// Tatha 主仓地址，例如 `http://127.0.0.1:8000`
const ENV_TATHA_BASE_URL: &str = "TATHA_BASE_URL";
/// 网关校验用户 JWT 的 HS256 密钥（与 looma-zervi 签发侧一致）
const ENV_JWT_SECRET: &str = "GATEWAY_JWT_SECRET";
/// 网关 → Tatha 的服务间凭据（替代用户 token，不透传）
const ENV_SERVICE_TOKEN: &str = "TATHA_SERVICE_TOKEN";

#[derive(Debug, Deserialize)]
struct GatewayClaims {
    /// JWT `sub`：THOMAS 生态内为邮箱，是 `email_hash` 的输入
    sub: String,
}

/// 读取环境变量；空串视同未配置。
fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 归一化邮箱：去首尾空格 + 小写。与 THOMAS 生态 `email_hash` 口径一致。
fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// THOMAS 生态跨端关联键：`sha256(email)`。
fn email_hash(email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_email(email).as_bytes());
    hex::encode(hasher.finalize())
}

/// 网关侧校验：仅接受 HS256。任何失败返回 `None`，由调用方转 401。
///
/// 未配置 `GATEWAY_JWT_SECRET` 时同样返回 `None` —— 无法鉴权即拒绝，不降级。
fn verify_bearer(authorization: Option<&HeaderValue>) -> Option<String> {
    let secret = env_nonempty(ENV_JWT_SECRET)?;
    let raw = authorization?.to_str().ok()?.trim().to_string();
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?
        .trim();
    if token.is_empty() {
        return None;
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_aud = false;
    let data = decode::<GatewayClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()?;

    Some(data.claims.sub)
}

/// `POST /v1/ask` —— 校验后转发到 Tatha 主仓。
pub async fn ask(State(state): State<AppState>, mut req: Request) -> Response {
    // 1) 网关鉴权：失败即拒，不转发
    let sub = match verify_bearer(req.headers().get(header::AUTHORIZATION)) {
        Some(sub) => sub,
        None => {
            warn!("🚫 /v1/ask rejected: missing or invalid bearer token");
            return (StatusCode::UNAUTHORIZED, "invalid or missing bearer token").into_response();
        }
    };

    // 2) 上游地址：未配则 503，不放行
    let base = match env_nonempty(ENV_TATHA_BASE_URL) {
        Some(b) => b.trim_end_matches('/').to_string(),
        None => {
            error!("🚫 /v1/ask rejected: {} not configured", ENV_TATHA_BASE_URL);
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "tatha upstream not configured",
            )
                .into_response();
        }
    };

    // 3) 重写身份头：注入生态关联键，不透传用户 token
    let uid = email_hash(&sub);
    {
        let headers = req.headers_mut();
        headers.remove(header::AUTHORIZATION);
        if let Ok(v) = HeaderValue::from_str(&uid) {
            headers.insert("X-User-Id", v);
        }
        match env_nonempty(ENV_SERVICE_TOKEN) {
            Some(t) => match HeaderValue::from_str(&format!("Bearer {}", t)) {
                Ok(v) => {
                    headers.insert(header::AUTHORIZATION, v);
                }
                Err(_) => error!(
                    "🚫 /v1/ask: {} contains invalid header bytes",
                    ENV_SERVICE_TOKEN
                ),
            },
            None => warn!(
                "⚠️  {} not set; forwarding /v1/ask without downstream credential",
                ENV_SERVICE_TOKEN
            ),
        }
    }

    let target = format!("{}/v1/ask", base);
    info!("🔄 /v1/ask → {} (uid_hash={}…)", target, &uid[..8]);
    proxy_request(&state.http_client, &target, req).await
}
