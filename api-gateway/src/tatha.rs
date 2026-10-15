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
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{
    decode, decode_header,
    jwk::{Jwk, KeyAlgorithm},
    Algorithm, DecodingKey, Validation,
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
/// 网关独占注入的头部前缀。
///
/// 转发前一律剥离客户端自带的同名头，**按前缀而非逐个点名**：注入是条件性的
/// （如 `X-User-Email-Hash` 仅在 token 带 email 时写入），逐个点名迟早会在新增
/// 头时漏掉一个，而漏掉的那一个恰好就是可伪造的入口。
const X_USER_PREFIX: &str = "x-user-";
/// JWKS 缓存寿命，对齐 looma `PyJWKClient(lifespan=600)`。
/// 缓存是硬要求：不得每请求拉网（ADR-0012 D1.1）。
const JWKS_TTL: Duration = Duration::from_secs(600);
/// JWKS 最小重试间隔：距上次**尝试**（成功或失败都算）不足此值时不再出网。
///
/// 防两件事：
/// - 未认证触发的出网放大：任何人可构造 `{"alg":"ES256","kid":"<随机>"}` 打到
///   `/v1/ask`；若每个未知 `kid` 都触发一次拉取，即变成对 Supabase JWKS 的放大
/// - 拉取失败后的每请求重试：失败时缓存内容不更新，没有节流就会一直重试
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(30);

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
    /// 上次**成功**拉取的时间；`None` = 从未成功过。
    ///
    /// 刻意用 `Option` 而不是「造一个过去的时间戳」来表达"从未成功"：
    /// `Instant` 在 macOS/Linux 上取 CLOCK_MONOTONIC（开机时长），用
    /// `checked_sub(JWKS_TTL)` 伪造"已过期"会在 uptime < TTL 时退化成
    /// "刚刚成功"，空缓存于是被判为新鲜 —— TTL 内既不重试也不出网，
    /// 全程静默 401。`Option` 让这个 hack 整个消失。
    fetched_at: Option<Instant>,
    /// 上次**尝试**的时间（成功或失败都推进）—— 用于重试节流
    last_attempt_at: Instant,
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

/// 按 `kid` 找 JWK → `DecodingKey`；JWK 自身若声明了 `alg`，必须与 header 的
/// `alg` 一致才采用（防止 key 与算法错配）。
fn jwk_to_key(keys: &[Jwk], kid: &str, alg: Algorithm) -> Option<DecodingKey> {
    keys.iter()
        .find(|j| j.common.key_id.as_deref() == Some(kid))
        .filter(|j| match j.common.key_algorithm {
            Some(declared) => key_alg_to_alg(declared) == Some(alg),
            None => true, // JWK 未声明 alg 时不做额外限制
        })
        .and_then(|j| DecodingKey::from_jwk(j).ok())
}

/// JWK 侧的 `KeyAlgorithm` → JWT 侧的 `Algorithm`。
///
/// 两者是独立枚举且无现成转换。JWK 还含仅用于**加密**的变体
/// （`RSA1_5` / `A128KW` 等），那些一律视为不匹配 ⇒ 对应 key 不被采用。
fn key_alg_to_alg(declared: KeyAlgorithm) -> Option<Algorithm> {
    Some(match declared {
        KeyAlgorithm::HS256 => Algorithm::HS256,
        KeyAlgorithm::HS384 => Algorithm::HS384,
        KeyAlgorithm::HS512 => Algorithm::HS512,
        KeyAlgorithm::ES256 => Algorithm::ES256,
        KeyAlgorithm::ES384 => Algorithm::ES384,
        KeyAlgorithm::RS256 => Algorithm::RS256,
        KeyAlgorithm::RS384 => Algorithm::RS384,
        KeyAlgorithm::RS512 => Algorithm::RS512,
        KeyAlgorithm::PS256 => Algorithm::PS256,
        KeyAlgorithm::PS384 => Algorithm::PS384,
        KeyAlgorithm::PS512 => Algorithm::PS512,
        KeyAlgorithm::EdDSA => Algorithm::EdDSA,
        _ => return None,
    })
}

/// 取 `kid` 对应的验签 key。
///
/// 三条约束必须同时成立，缺一不可 —— 它们彼此是有张力的，只满足其中两条
/// 都会出事：
///
/// 1. **放大有界**：未认证请求可构造任意 `kid`，若每个未知 `kid` 都触发一次
///    拉取，就变成对 Supabase JWKS 的放大攻击。故刷新受 `JWKS_MIN_REFRESH`
///    节流，最坏 ≤ 1 次 / 30s
/// 2. **轮换可见**：缓存新鲜但 `kid` 未命中时**不能**直接返回 `None`。
///    那样即使堵住了放大，也会让 Supabase 轮换 kid 之后、持新 kid 的 token
///    在最长 `JWKS_TTL` 内全部 401，且**零次刷新尝试**（10 分钟无法登录，
///    生产事故量级）。未命中应落到节流后的刷新，把轮换响应压到 ≤ 30s
/// 3. **占位式节流**：`last_attempt_at` 必须在**出网之前**于写锁内 claim，
///    否则冷启动或窗口刚过时，并发 N 个请求会同时通过闸门、出网 N 次
///
/// 刻意**不在持锁期间 await** —— claim 与写回都是短临界区，拉网发生在锁外。
/// 最终取不到 key 则返回 `None` ⇒ 调用方转 401（fail-closed，不回落）。
async fn jwks_key(kid: &str, alg: Algorithm) -> Option<DecodingKey> {
    let url = env_nonempty(ENV_JWKS_URL)?;

    // 闸门 1：缓存新鲜且**命中** → 直接返回，不出网
    let mut missed_in_fresh_cache = false;
    {
        let guard = jwks_cache().read().ok()?;
        if let Some(c) = guard.as_ref() {
            let fresh = c.fetched_at.map(|t| t.elapsed() < JWKS_TTL).unwrap_or(false);
            if fresh {
                if let Some(k) = jwk_to_key(&c.keys, kid, alg) {
                    return Some(k);
                }
                // 未命中：此处**不** return None —— 见文档第 2 点（轮换盲窗）
                missed_in_fresh_cache = true;
            }
        }
    }

    // 闸门 2 + 占位 claim：窗口内沿用旧缓存（不出网），窗口外先 claim 再出网
    {
        let mut guard = jwks_cache().write().ok()?;
        let now = Instant::now();
        match guard.as_mut() {
            Some(c) => {
                if c.last_attempt_at.elapsed() < JWKS_MIN_REFRESH {
                    // 窗口内：沿用现有缓存；未命中即 None，不出网
                    return jwk_to_key(&c.keys, kid, alg);
                }
                c.last_attempt_at = now; // claim（见文档第 3 点）
            }
            None => {
                // 首次：占位条目，keys 为空、fetched_at 为 None（从未成功）
                *guard = Some(JwksCache {
                    fetched_at: None,
                    last_attempt_at: now,
                    keys: Vec::new(),
                });
            }
        }
    }

    // 已确定要出网 —— 日志放在这里，频率因此同样 ≤ 1 次 / JWKS_MIN_REFRESH。
    // kid 由请求方控制，截断后再记录，避免日志注入与刷屏。
    if missed_in_fresh_cache {
        let safe_kid: String = kid.chars().take(64).collect();
        warn!(
            "⚠️  JWKS: kid absent from fresh cache (kid={}); refreshing (throttled to 1/{}s)",
            safe_kid,
            JWKS_MIN_REFRESH.as_secs()
        );
    }

    // 慢路径：拉网（不持锁）
    let fetched: Option<Vec<Jwk>> = match http_client().get(&url).send().await {
        Ok(resp) => resp.json::<JwksDocument>().await.ok().map(|d| d.keys),
        Err(e) => {
            warn!("⚠️  JWKS fetch failed ({}): {}", ENV_JWKS_URL, e);
            None
        }
    };

    {
        let now = Instant::now();
        let mut guard = jwks_cache().write().ok()?;
        // 上面的 claim 已保证条目存在（首次会占位）
        let c = guard.as_mut()?;
        if let Some(keys) = fetched {
            c.fetched_at = Some(now);
            c.keys = keys;
        }
        // 拉取失败：保留旧 key 集（瞬时故障不该洗掉缓存、导致合法鉴权全挂），
        // fetched_at 不动 —— 于是缓存保持"已过期 / 从未成功"，下次走节流重试
        c.last_attempt_at = now;
    }

    let guard = jwks_cache().read().ok()?;
    jwk_to_key(&guard.as_ref()?.keys, kid, alg)
}

/// 非对称算法走 JWKS 轨（Supabase 原生 token）。
///
/// 注释与实现对齐：ES256 / ES384 / RS256 / RS384 / RS512 / PS256 / PS384 /
/// PS512 / EdDSA —— 凡非对称即走 JWKS。HS384 / HS512 虽同属 HMAC 族，但
/// ADR 只提 HS256 轨，故一律拒（fail-closed）。
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
    // 两轨皆未配置 ⇒ 无法鉴权，直接拒：既不解析 token，也不触发任何出网
    if env_nonempty(ENV_JWT_SECRET).is_none() && env_nonempty(ENV_JWKS_URL).is_none() {
        return None;
    }

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
        // HS256 轨：looma 代签 / 存量 looma token。
        //
        // **必须**明文关掉 aud 校验：jsonwebtoken 的 `Validation::new()` 默认
        // `validate_aud = true`，不关则存量 looma token 当场全挂。
        // 这不是可选的对齐，是维持存量可用的必要条件。
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
        let key = jwks_key(kid, header.alg).await?;
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

        // 剥离客户端自带的 `x-user-*` 头（P0）：
        // 身份注入是**条件性**的 —— `X-User-Email-Hash` 仅在 token 带 email 时
        // 写入；不清理的话，持「无 email claim 的合法 token」（D2 明确允许的小
        // 程序-only 用户）即可自带 `X-User-Email-Hash: <任意>` 直通下游。该头是
        // 跨端关联键，放任等于把身份挂到别人的跨端身份上 —— 越权，不是可用性问题。
        //
        // 按前缀清理而非逐个点名：将来新增 `x-user-*` 头时不会重蹈覆辙。
        let forged: Vec<HeaderName> = headers
            .keys()
            .filter(|k| k.as_str().starts_with(X_USER_PREFIX))
            .cloned()
            .collect();
        for k in forged {
            headers.remove(k);
        }

        if let Ok(v) = HeaderValue::from_str(&uid) {
            headers.insert("X-User-Id", v);
        }
        // 无真实邮箱（小程序-only 用户）时**不注入**该头 —— 上面的剥离已保证它
        // 不会以客户端携带的值漏出，从而维持 D2 的 email_hash=None 语义
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
