//! Tatha 单入口转发 —— GFCC `ask.envelope.v1` 的网关侧骨架。
//!
//! 拓扑（Tatha/docs/架构_单入口与中央大脑.md §5.2 拓扑 A）：
//!
//! ```text
//! 用户 → 网关(zervi-rust: 单入口 + 鉴权) → POST /v1/ask → Tatha(Python) → 意图解析 → 返回
//! ```
//!
//! 硬约束（对接 ADR-0012《THOMAS 生态身份统一》，2026-10-10 Accepted）：
//!
//! 1. 网关鉴权失败 → 401，**绝不转发**（fail-closed，不降级、不回落 stub）
//! 2. `TATHA_BASE_URL` 未配 → 503，**绝不静默放行**
//! 3. 身份双头注入（ADR-0012 D2）：
//!      - `X-User-Id` = `sha256(sub)`（sub = Supabase UUID，必有、跨端稳定、与 provider 无关）
//!      - `X-User-Email-Hash` = `sha256(email)`（可选；无真实邮箱则不注入，**不造假值**）
//!    不下传用户 token；下游只信任网关注入头。
//!
//! 验签双轨（ADR-0012 D1，fail-closed）：
//!      - JWKS / ES256（Supabase 原生登录，preferred）
//!      - HS256（looma 代签 custom token 与存量 looma token）
//! 任一通过即放行；都不通过即 401。

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, decode_header, jwk::Jwk, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tracing::{error, info, warn};

use crate::{proxy_request, AppState};

/// Tatha 主仓地址，例如 `http://127.0.0.1:8000`
const ENV_TATHA_BASE_URL: &str = "TATHA_BASE_URL";
/// 网关校验用户 JWT 的 HS256 密钥（与 looma-zervi 签发侧一致；用于 custom/存量 token）
const ENV_JWT_SECRET: &str = "GATEWAY_JWT_SECRET";
/// 网关 JWKS 端点（Supabase 原生 ES256 公钥；配置即启用原生登录验签）
const ENV_JWKS_URL: &str = "GATEWAY_JWKS_URL";
/// 网关 → Tatha 的服务间凭据（替代用户 token，不透传）
const ENV_SERVICE_TOKEN: &str = "TATHA_SERVICE_TOKEN";
/// 缺省订阅档位（Tatha 侧配额：free=3 / basic=20 / pro=9999 次 job_match）
const ENV_DEFAULT_TIER: &str = "GATEWAY_DEFAULT_TIER";

/// JWKS 公钥缓存有效期（秒）。超期后在下一次命中/未命中时重新拉取；
/// 拉取失败不降级（无缓存即拒绝，有旧缓存则保留旧缓存）。
const JWKS_CACHE_TTL_SECS: u64 = 600;

#[derive(Debug, Deserialize)]
struct GatewayClaims {
    /// JWT `sub`：THOMAS 生态内为 Supabase UUID（稳定主键），是 `X-User-Id` 的输入
    sub: String,
    /// 可选：签发侧写入的订阅档位；缺省回落 `GATEWAY_DEFAULT_TIER`，再缺省 `free`
    #[serde(default)]
    tier: Option<String>,
    /// 可选：Supabase 原生 token 通常带 `email`；用于 `X-User-Email-Hash`（跨端关联，非主键）
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// JWKS 公钥缓存（按 kid 索引）。首次命中或 kid 未命中时拉取；拉取失败则按是否已有缓存决定降级策略。
pub struct JwksCache {
    url: String,
    keys: RwLock<HashMap<String, Jwk>>,
    fetched_at: RwLock<Option<Instant>>,
}

impl JwksCache {
    pub fn new(url: String) -> Self {
        Self {
            url,
            keys: RwLock::new(HashMap::new()),
            fetched_at: RwLock::new(None),
        }
    }

    async fn refresh(&self) -> anyhow::Result<()> {
        let resp = reqwest::Client::new().get(&self.url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("JWKS fetch status {}", resp.status());
        }
        let jwks: Jwks = resp.json().await?;
        let mut map = HashMap::new();
        for k in jwks.keys {
            if let Some(kid) = k.common.key_id.clone() {
                map.insert(kid, k);
            }
        }
        *self.keys.write().unwrap() = map;
        *self.fetched_at.write().unwrap() = Some(Instant::now());
        Ok(())
    }

    /// 返回指定 kid 的 JWK：命中且未超期直接返回；超期或未命中则尝试拉取一次。
    /// 拉取失败：从未缓存过 → 拒绝（None）；已有旧缓存 → 保留旧缓存（不降级）。
    async fn get_key(&self, kid: &str) -> Option<Jwk> {
        if let Some(k) = self.keys.read().unwrap().get(kid).cloned() {
            let stale = self
                .fetched_at
                .read()
                .unwrap()
                .map(|t| t.elapsed().as_secs() > JWKS_CACHE_TTL_SECS)
                .unwrap_or(true);
            if !stale {
                return Some(k);
            }
        }
        if self.refresh().await.is_err() {
            return self.keys.read().unwrap().get(kid).cloned();
        }
        self.keys.read().unwrap().get(kid).cloned()
    }
}

/// 若配置了 `GATEWAY_JWKS_URL`，构建 JWKS 缓存（供 `AppState` 装配）。
pub fn jwks_cache_from_env() -> Option<Arc<JwksCache>> {
    env_nonempty(ENV_JWKS_URL).map(|u| Arc::new(JwksCache::new(u)))
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

/// 跨端关联键：`sha256(email)`（可选，非主键；无真实邮箱则不注入）。
fn email_hash(email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_email(email).as_bytes());
    hex::encode(hasher.finalize())
}

/// 稳定主键：`sha256(sub)`（sub = Supabase UUID，必有、跨端稳定、与 provider 无关）。
fn sub_hash(sub: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sub.trim().to_lowercase().as_bytes());
    hex::encode(hasher.finalize())
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

/// 网关侧校验：双轨（JWKS/ES256 优先 → HS256 回落），均 fail-closed。
///
/// - 配了 `GATEWAY_JWKS_URL`：先走 Supabase 原生 ES256/RS256（按 kid 取公钥，校验 `aud=authenticated`）
/// - 回落 HS256：looma 代签 custom token 与存量 looma token（使用 `GATEWAY_JWT_SECRET`）
/// - 两者都未配置 / 都验不过 → 返回 `None`，由调用方转 401（不降级、不静默放行）
async fn verify_bearer(state: &AppState, authorization: Option<&HeaderValue>) -> Option<GatewayClaims> {
    let raw = authorization?.to_str().ok()?.trim().to_string();
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?
        .trim();
    if token.is_empty() {
        return None;
    }

    // 1) JWKS / ES256（Supabase 原生登录）—— preferred
    if let Some(jwks) = &state.jwks {
        if let Some(claims) = decode_via_jwks(jwks, token).await {
            return Some(claims);
        }
        // 原生轨失败（非 ES256 / kid 未命中 / 验签失败）→ 回落 HS256 轨
    }

    // 2) HS256（looma 代签 / 存量 looma token）
    let secret = env_nonempty(ENV_JWT_SECRET)?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_aud = false;
    decode::<GatewayClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}

/// 经 JWKS 公钥验签 Supabase 原生 token（使用 `from_jwk`，内部按 base64url 解码，兼容 Supabase）。
async fn decode_via_jwks(jwks: &JwksCache, token: &str) -> Option<GatewayClaims> {
    let header = decode_header(token).ok()?;
    let kid = header.kid.as_deref()?;
    let jwk = jwks.get_key(kid).await?;
    let key = DecodingKey::from_jwk(&jwk).ok()?;
    let mut validation = Validation::new(header.alg);
    validation.set_audience(&["authenticated"]);
    decode::<GatewayClaims>(token, &key, &validation)
        .ok()
        .map(|d| d.claims)
}

/// `POST /v1/ask` —— 校验后转发到 Tatha 主仓。
pub async fn ask(State(state): State<AppState>, mut req: Request) -> Response {
    // 1) 网关鉴权：失败即拒，不转发
    let claims = match verify_bearer(&state, req.headers().get(header::AUTHORIZATION)).await {
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

    // 3) 重写身份头：双头注入（ADR-0012 D2）。主键必有，跨端关联键可选。
    let uid = sub_hash(&claims.sub);
    let tier = resolve_tier(&claims);
    let email_hash_opt = claims
        .email
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(email_hash);
    {
        let headers = req.headers_mut();
        headers.remove(header::AUTHORIZATION);
        if let Ok(v) = HeaderValue::from_str(&uid) {
            headers.insert("X-User-Id", v);
        }
        if let Some(eh) = email_hash_opt {
            if let Ok(v) = HeaderValue::from_str(&eh) {
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
    info!("🔄 /v1/ask → {} (uid_hash={}…, tier={})", target, &uid[..8], tier);
    proxy_request(&state.http_client, &target, req).await
}

/// `GET /v1/auth/verify` —— 量子认证兼容端点（Tatha `QUANTUM_AUTH_URL` 指向此处）。
///
/// ⚠️ 过渡期兼容：单2（Tatha）将改为直接信任网关注入的 `X-User-Id` 并置空 `QUANTUM_AUTH_URL`，
/// 此后本端点不再被调用。此处 `user_id` 仍返回旧口径 `sha256(email)`，避免中断在途的 Tatha#5。
pub async fn verify(State(state): State<AppState>, req: Request) -> Response {
    let claims = match verify_bearer(&state, req.headers().get(header::AUTHORIZATION)).await {
        Some(claims) => claims,
        None => {
            warn!("🚫 /v1/auth/verify rejected: missing or invalid bearer token");
            return (StatusCode::UNAUTHORIZED, "invalid or missing bearer token").into_response();
        }
    };

    let user_id = email_hash(&claims.sub);
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
