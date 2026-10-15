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
//! 3. 身份注入按 **ADR-0012 D2** 双头，且**不透传用户 token**
//!    （Tatha 的 stub 会把 user_id 派生为 `stub-<token>`，切断 THOMAS 跨端关联）：
//!    - `X-User-Id = sha256(sub)`，`sub` = Supabase UUID —— **稳定主键，必有**
//!    - `X-User-Email-Hash = sha256(lower(trim(email)))` —— **无真实邮箱时不注入**
//!
//! 验签按 **ADR-0012 D1** 双轨：JWKS（Supabase 原生，非对称，缓存 600s）与
//! HS256（looma 代签 / 存量）按 `alg` 互斥；两轨皆未配置即 401，不降级。
//!
//! 接通方式（Tatha 侧零改动）：
//! 把 Tatha 的 `QUANTUM_AUTH_URL` 指向本网关的 `GET /v1/auth/verify`，
//! 网关返回 `200 + {"user_id": "<sha256(sub)>", "tier": "free|basic|pro"}`,
//! 其余一律 401。配合 Tatha 侧的 fail-closed（远程失败不回落 stub），
//! 网关成为唯一鉴权点，且下游身份统一为生态稳定主键而非 `stub-<token>`。

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{
    decode, decode_header, jwk::Jwk, Algorithm, DecodingKey, Validation,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::{proxy_request, AppState};

/// Tatha 主仓地址，例如 `http://127.0.0.1:8000`
const ENV_TATHA_BASE_URL: &str = "TATHA_BASE_URL";
/// 网关校验用户 JWT 的 HS256 密钥（looma 代签 / 存量 looma token 轨）
const ENV_JWT_SECRET: &str = "GATEWAY_JWT_SECRET";
/// Supabase 原生 token 的 JWKS 地址（ADR-0012 D1.1 新增轨）
const ENV_JWKS_URL: &str = "GATEWAY_JWKS_URL";
/// 网关 → Tatha 的服务间凭据（替代用户 token，不透传）
const ENV_SERVICE_TOKEN: &str = "TATHA_SERVICE_TOKEN";
/// 缺省订阅档位（Tatha 侧配额：free=3 / basic=20 / pro=9999 次 job_match）
const ENV_DEFAULT_TIER: &str = "GATEWAY_DEFAULT_TIER";
/// Supabase 原生 token 的受众（对齐 looma `jwt_handler.py` 的 `aud=authenticated`）
const SUPABASE_AUDIENCE: &str = "authenticated";
/// JWKS 缓存寿命，对齐 looma `PyJWKClient(lifespan=600)`。
/// 缓存是硬要求：不得每请求拉网（ADR-0012 D1.1）。
const JWKS_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
struct GatewayClaims {
    /// JWT `sub`：ADR-0012 D2 之后语义固定为 **Supabase UUID**（稳定主键）。
    /// 不再假定为邮箱 —— `X-User-Id` 直接取 `sha256(sub)`。
    sub: String,
    /// 可选：真实邮箱。存在且非空时派生 `X-User-Email-Hash`；
    /// 缺失（如小程序-only 用户）则**不注入**该头，保持 `email_hash=None` 语义。
    #[serde(default)]
    email: Option<String>,
    /// 可选：签发侧写入的订阅档位；缺省回落 `GATEWAY_DEFAULT_TIER`，再缺省 `free`
    #[serde(default)]
    tier: Option<String>,
}

/// 解析档位：只认 free/basic/pro，其余一律按 free（拒绝放宽）。
fn resolve_tier(claims: &GatewayClaims) -> &'static str {
    let fallback = env_nonempty(ENV_DEFAULT_TIER);
    let raw = claims
        .tier
        .as_deref()
        .or(fallback.as_deref())
        .unwrap_or("free");
    match raw.trim().to_lowercase().as_str() {
        "basic" => "basic",
        "pro" => "pro",
        _ => "free",
    }
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

/// ADR-0012 D2 稳定主键：`X-User-Id = sha256(sub)`，`sub` 为 Supabase UUID。
///
/// 与 `email_hash` 的区别：本函数哈希的是**不随邮箱变更而变**的稳定主键。
fn stable_user_id(sub: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sub.trim().as_bytes());
    hex::encode(hasher.finalize())
}

/// JWKS 文档：`{"keys": [...]}`
#[derive(Deserialize)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

struct JwksCache {
    fetched_at: Instant,
    keys: Vec<Jwk>,
}

/// 进程内 JWKS 缓存：读多写少，避免每请求拉网（ADR-0012 D1.1 硬要求）。
fn jwks_cache() -> &'static RwLock<Option<JwksCache>> {
    static CACHE: OnceLock<RwLock<Option<JwksCache>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(None))
}

/// 共用 HTTP 客户端：复用连接池。
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// 按 `kid` 找 JWK → `DecodingKey`。
fn jwk_to_key(keys: &[Jwk], kid: &str) -> Option<DecodingKey> {
    keys.iter()
        .find(|j| j.common.key_id.as_deref() == Some(kid))
        .and_then(|j| DecodingKey::from_jwk(j).ok())
}

/// 取 `kid` 对应的验签 key：缓存新鲜则直接返回，否则拉取一次并写回。
///
/// 刻意**不在持锁期间 await** —— 先拉网、再取写锁写入，临界区内无 await。
/// 拉取失败返回 `None` ⇒ 调用方转 401（fail-closed，不回落）。
async fn jwks_key(kid: &str) -> Option<DecodingKey> {
    let url = env_nonempty(ENV_JWKS_URL)?;

    // 快路径：缓存新鲜
    {
        let guard = jwks_cache().read().ok()?;
        if let Some(c) = guard.as_ref() {
            if c.fetched_at.elapsed() < JWKS_TTL {
                if let Some(k) = jwk_to_key(&c.keys, kid) {
                    return Some(k);
                }
            }
        }
    }

    // 慢路径：拉网（不持锁）
    let doc: JwksDocument = http_client()
        .get(&url)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    {
        let mut guard = jwks_cache().write().ok()?;
        *guard = Some(JwksCache {
            fetched_at: Instant::now(),
            keys: doc.keys,
        });
    }

    let guard = jwks_cache().read().ok()?;
    jwk_to_key(&guard.as_ref()?.keys, kid)
}

/// 非对称算法走 JWKS 轨（Supabase 原生 token）。
fn is_asymmetric(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::EdDSA
    )
}

/// 网关侧校验：**双轨**（ADR-0012 D1）—— JWKS（非对称）与 HS256（共享密钥）
/// 按 `alg` 互斥，任一通过即放行，都不通过即 401。
///
/// 两轨都未配置时同样返回 `None` —— 无法鉴权即拒绝，不降级、不回落 stub。
async fn verify_bearer(authorization: Option<&HeaderValue>) -> Option<GatewayClaims> {
    let raw = authorization?.to_str().ok()?.trim().to_string();
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?
        .trim();
    if token.is_empty() {
        return None;
    }

    let header = decode_header(token).ok()?;
    // 显式指定算法（ADR-0012 D1.3）：杜绝算法混淆空间
    let mut validation = Validation::new(header.alg);

    if header.alg == Algorithm::HS256 {
        // HS256 轨：looma 代签 / 存量 looma token。保持既有的 aud 宽松策略，
        // 不给存量 token 增加新的拒绝面。
        let secret = env_nonempty(ENV_JWT_SECRET)?;
        validation.validate_aud = false;
        decode::<GatewayClaims>(
            token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &validation,
        )
        .ok()
        .map(|d| d.claims)
    } else if is_asymmetric(header.alg) {
        // JWKS 轨：Supabase 原生 token（ES256 为主），校验 aud=authenticated
        let kid = header.kid.as_deref()?;
        let key = jwks_key(kid).await?;
        validation.validate_aud = true;
        validation.aud = Some(HashSet::from([SUPABASE_AUDIENCE.to_string()]));
        decode::<GatewayClaims>(token, &key, &validation)
            .ok()
            .map(|d| d.claims)
    } else {
        // 其余算法（HS384 / HS512 等）一律不接受
        None
    }
}

/// `POST /v1/ask` —— 校验后转发到 Tatha 主仓。
pub async fn ask(State(state): State<AppState>, mut req: Request) -> Response {
    // 1) 网关鉴权：失败即拒，不转发
    let claims = match verify_bearer(req.headers().get(header::AUTHORIZATION)).await {
        Some(claims) => claims,
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

    // 3) 重写身份头（ADR-0012 D2）：稳定主键 + 可选邮箱哈希，不透传用户 token
    let uid = stable_user_id(&claims.sub);
    let tier = resolve_tier(&claims);
    {
        let headers = req.headers_mut();
        headers.remove(header::AUTHORIZATION);
        if let Ok(v) = HeaderValue::from_str(&uid) {
            headers.insert("X-User-Id", v);
        }
        // 无真实邮箱（小程序-only 用户）时不注入该头，保持 email_hash=None 语义
        if let Some(email) = claims
            .email
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            if let Ok(v) = HeaderValue::from_str(&email_hash(email)) {
                headers.insert("X-User-Email-Hash", v);
            }
        }
        headers.insert("X-User-Tier", HeaderValue::from_static(tier));
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
    info!(
        "🔄 /v1/ask → {} (uid_hash={}…, tier={})",
        target,
        &uid[..8],
        tier
    );
    proxy_request(&state.http_client, &target, req).await
}

/// `GET /v1/auth/verify` —— 量子认证兼容端点（Tatha `QUANTUM_AUTH_URL` 指向此处）。
///
/// 契约对齐 Tatha `src/tatha/api/auth.py::_verify_token_remote`：
/// `200 + {"user_id": ..., "tier": ...}` 视为通过，其余一律 401。
///
/// 两点关键：
/// - `user_id` 返回 ADR-0012 D2 稳定主键 `sha256(sub)`（`sub` = Supabase UUID），
///   不是 `stub-<token>`，因此 Tatha 的配额（free=3 / basic=20 / pro=9999）
///   按同一身份计数；跨端关联另用 `X-User-Email-Hash`
/// - 校验失败即 401；叠加 Tatha 侧的 fail-closed 后不会回落 stub
pub async fn verify(req: Request) -> Response {
    let claims = match verify_bearer(req.headers().get(header::AUTHORIZATION)).await {
        Some(claims) => claims,
        None => {
            warn!("🚫 /v1/auth/verify rejected: missing or invalid bearer token");
            return (StatusCode::UNAUTHORIZED, "invalid or missing bearer token").into_response();
        }
    };

    // ADR-0012 D2：user_id 为稳定主键 sha256(sub)，与 X-User-Id 同义
    let user_id = stable_user_id(&claims.sub);
    let tier = resolve_tier(&claims);
    info!(
        "✅ /v1/auth/verify ok (uid_hash={}…, tier={})",
        &user_id[..8],
        tier
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "user_id": user_id, "tier": tier }).to_string(),
    )
        .into_response()
}
